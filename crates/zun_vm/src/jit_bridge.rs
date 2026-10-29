//! JIT → interpreter call bridge (slice 1a: the safe core).
//!
//! When the JIT compiles a method that calls an *ineligible* callee, it cannot
//! jump into native code that does not exist. The bridge runs that callee on the
//! interpreter, on the **current** heap (the JIT frame's heap), and hands the
//! result back packed in the shim ABI the JIT already uses for arguments and
//! results (see `jit_hook::{pack_value, unpack_value}`).
//!
//! This module is only the safe core: [`bridge_call`] takes real references and
//! is unit-tested directly. The `extern "C"` entry point and the thread-local
//! that let JIT-emitted code reach it are added when the code-generation call
//! site is wired (bridge slice 1b); keeping the logic here lets it be verified
//! in isolation first, with no unsafe lifetime erasure and no changes to
//! `zun_codegen`.
//!
//! See `docs/design/jit-interp-bridge.md`.

use std::cell::RefCell;

use zun_codegen::JitKind;
use zun_interp::value::Value;
use zun_interp::{run_method_on_current_heap_bare, Output, RunError};
use zun_loader::ModuleLoader;
use zun_module::{read_body, Module};

use crate::jit_hook::{pack_value, unpack_value};

/// Outcome of a bridged call: the packed return value and a status (0 = the
/// callee returned normally, 1 = it threw). The thrown value, when present, is
/// packed as an object handle for the JIT frame to route to a catch (a later
/// slice) — today it is surfaced so the status channel is exercised end to end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BridgeOutcome {
    pub packed: u64,
    pub status: u32,
}

pub(crate) const STATUS_RETURNED: u32 = 0;
pub(crate) const STATUS_THREW: u32 = 1;

/// Run an ineligible callee `rid` of `scope`'s module on the current heap, with
/// its arguments supplied packed in the shim ABI (one `u64` per JIT value, in
/// declaration order, matching the callee's parameter kinds). Returns the packed
/// result and a status. A VM fault (not a catchable throw) propagates as `Err`.
///
/// Must run with a heap installed (the caller's JIT frame heap). Arguments are
/// unpacked and the result packed in short heap borrows that never overlap the
/// interpreter call, which manages the heap itself.
pub(crate) fn bridge_call(
    module: &Module,
    loader: Option<&ModuleLoader>,
    output: &mut dyn Output,
    rid: u32,
    packed_args: &[u64],
) -> Result<BridgeOutcome, RunError> {
    let method = module
        .tables
        .method_defs
        .get(rid as usize - 1)
        .ok_or(RunError::BadToken)?;
    let body = read_body(&module.code, method.body_offset)
        .map_err(|e| RunError::Malformed(e.to_string()))?;
    let param_count = body.param_count as usize;
    if packed_args.len() != param_count {
        return Err(RunError::BadToken);
    }
    let param_kinds: Vec<JitKind> = body.local_kinds[..param_count]
        .iter()
        .map(|&byte| JitKind::from_byte(byte))
        .collect();
    let return_kind = if body.returns_value {
        Some(JitKind::from_byte(body.return_kind))
    } else {
        None
    };

    // Unpack arguments in a short heap borrow (no overlap with the interpreter
    // call below, which borrows the heap itself).
    let heap = zun_gc::current_heap_ptr().ok_or(RunError::BadToken)?;
    let args: Vec<Value> = packed_args
        .iter()
        .zip(&param_kinds)
        .map(|(&bits, &kind)| unpack_value(bits, Some(kind), unsafe { &*heap }))
        .collect();

    match run_method_on_current_heap_bare(module, loader, rid, args, output) {
        Ok(result) => {
            let heap = zun_gc::current_heap_ptr().ok_or(RunError::BadToken)?;
            let packed = match return_kind {
                Some(kind) => pack_value(&result, kind, unsafe { &mut *heap }),
                None => 0,
            };
            Ok(BridgeOutcome { packed, status: STATUS_RETURNED })
        }
        // A catchable throw becomes a status the JIT frame can route; the thrown
        // object is packed as its handle.
        Err(RunError::Thrown(value)) => {
            let packed = match value {
                Value::Object(object) => object.addr() as u64,
                _ => 0,
            };
            Ok(BridgeOutcome { packed, status: STATUS_THREW })
        }
        Err(other) => Err(other),
    }
}

/// Execution context the `extern "C"` bridge entry reads to reach the
/// interpreter. The pointers are lifetime-erased; [`BridgeScope`] installs them
/// for the duration of a JIT call and restores the previous value on drop, so
/// they are valid whenever bridged code can run.
#[derive(Clone, Copy)]
struct BridgeContext {
    module: *const Module,
    loader: Option<*const ModuleLoader>,
    output: *mut (dyn Output + 'static),
}

thread_local! {
    static BRIDGE_CONTEXT: RefCell<Option<BridgeContext>> = const { RefCell::new(None) };
    /// Set to a thrown object's handle (nonzero) when a bridged callee throws;
    /// 0 means no pending exception. The JIT reads it after a call to decide
    /// whether to route to a catch or propagate; the interpreter consults it
    /// (via `JitHook::take_pending_exception`) after a hooked JIT call.
    static PENDING_EXCEPTION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Record a thrown object handle as the pending exception (set by the bridge
/// wrapper when a bridged callee throws).
pub(crate) fn set_pending_exception(handle: u64) {
    PENDING_EXCEPTION.with(|slot| slot.set(handle));
}

/// Address of the per-thread pending-exception slot, for the JIT to load after a
/// call. Stable for the thread's life; the JIT runs on the thread that built it.
pub(crate) fn pending_slot_addr() -> u64 {
    PENDING_EXCEPTION.with(|slot| slot.as_ptr() as u64)
}

/// The bridge wiring handed to [`zun_codegen::Jit`]: the two `extern "C"` entry
/// points and this thread's pending-exception slot address.
pub(crate) fn bridge_config() -> zun_codegen::BridgeConfig {
    zun_codegen::BridgeConfig {
        call_interp: zun_jit_call_interp as *const u8,
        route_pending: zun_jit_route_pending as *const u8,
        pending_slot_addr: pending_slot_addr(),
    }
}

/// Whether a thrown object of `throw_rid` is caught by `catch_rid` (same type or
/// a subclass), walking the module's `TypeDef` base chain. Bounded against a
/// malformed cyclic hierarchy.
fn type_is_caught(module: &Module, throw_rid: u32, catch_rid: u32) -> bool {
    let mut current = throw_rid;
    for _ in 0..64 {
        if current == catch_rid {
            return true;
        }
        let Some(ty) = module.tables.type_defs.get(current as usize - 1) else {
            return false;
        };
        if ty.base.table() != Some(zun_metadata::TableKind::TypeDef) || ty.base.rid() == 0 {
            return false;
        }
        current = ty.base.rid();
    }
    false
}

/// Route a pending exception against a same-frame catch of type `catch_type_rid`.
/// Returns the thrown object handle and CLEARS the pending slot when the throw is
/// caught by that type (the JIT then branches to the catch with the object);
/// returns 0 (leaving any pending throw set to propagate) when there is no
/// pending throw or it does not match. The exception type is read from the
/// object header; the hierarchy comes from the installed bridge context's module.
pub(crate) extern "C" fn zun_jit_route_pending(catch_type_rid: i64) -> i64 {
    let handle = PENDING_EXCEPTION.with(|slot| slot.get());
    if handle == 0 {
        return 0;
    }
    let type_rid = unsafe { zun_gc::Obj::from_addr(handle as *mut u64) }.type_rid();
    let Some(context) = BRIDGE_CONTEXT.with(|cell| *cell.borrow()) else {
        return 0;
    };
    let module = unsafe { &*context.module };
    if type_is_caught(module, type_rid, catch_type_rid as u32) {
        PENDING_EXCEPTION.with(|slot| slot.set(0));
        handle as i64
    } else {
        0
    }
}

/// Take and clear the pending exception handle, if any.
pub(crate) fn take_pending_exception() -> Option<u64> {
    PENDING_EXCEPTION.with(|slot| {
        let handle = slot.get();
        if handle != 0 {
            slot.set(0);
            Some(handle)
        } else {
            None
        }
    })
}

/// Install the bridge execution context for the current thread, restoring the
/// previous one when dropped. Must wrap any JIT execution that can reach a
/// bridged call; the borrowed `module`/`loader`/`output` must outlive the guard.
pub(crate) struct BridgeScope {
    previous: Option<BridgeContext>,
}

impl BridgeScope {
    pub(crate) fn enter(
        module: &Module,
        loader: Option<&ModuleLoader>,
        output: &mut dyn Output,
    ) -> Self {
        // Erase the output's lifetime; the guard's Drop clears the context before
        // the borrow can end (the caller must keep the borrows alive until then).
        let output: *mut (dyn Output + 'static) =
            unsafe { std::mem::transmute::<*mut dyn Output, *mut (dyn Output + 'static)>(output) };
        let context = BridgeContext {
            module,
            loader: loader.map(|loader| loader as *const ModuleLoader),
            output,
        };
        let previous = BRIDGE_CONTEXT.with(|cell| cell.borrow_mut().replace(context));
        BridgeScope { previous }
    }
}

impl Drop for BridgeScope {
    fn drop(&mut self) {
        BRIDGE_CONTEXT.with(|cell| *cell.borrow_mut() = self.previous);
    }
}

/// The `extern "C"` entry JIT code calls for a bridged call. Reads the installed
/// context, runs the ineligible callee via [`bridge_call`], and returns the
/// packed result. Only non-throwing callees are bridged (eligibility guarantees
/// it), so a throw cannot occur here; a VM fault returns 0 (the callee cannot
/// fault on valid input, and faults are abort-level in both tiers).
pub(crate) extern "C" fn zun_jit_call_interp(rid: i64, args_ptr: *const u64, argc: i64) -> u64 {
    // Copy the context out and drop the borrow before re-entering the
    // interpreter (which could, in principle, reach another bridged call).
    let context = BRIDGE_CONTEXT
        .with(|cell| *cell.borrow())
        .expect("bridge context must be installed before bridged code runs");
    let module = unsafe { &*context.module };
    let loader = context.loader.map(|loader| unsafe { &*loader });
    let output = unsafe { &mut *context.output };
    let args: &[u64] = if argc > 0 {
        unsafe { std::slice::from_raw_parts(args_ptr, argc as usize) }
    } else {
        &[]
    };
    match bridge_call(module, loader, output, rid as u32, args) {
        Ok(outcome) => {
            if outcome.status == STATUS_THREW {
                // Record the thrown object; the JIT frame's post-call check
                // routes it (to a catch, or out of the frame).
                set_pending_exception(outcome.packed);
                0
            } else {
                outcome.packed
            }
        }
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zun_interp::CaptureOutput;

    /// A bridged scalar call returns the same value the interpreter would.
    #[test]
    fn bridges_a_scalar_call_on_the_current_heap() {
        let module = zun_compiler::compile(
            "function add(a: int, b: int): int { return a + b; } function main(): int { return 0; }",
        )
        .expect("compile");
        let loader = {
            let mut loader = ModuleLoader::new();
            loader.load_bytes("app", &module.write()).unwrap();
            loader
        };
        let scope = loader.get("app").unwrap();
        let rid = scope
            .module()
            .tables
            .method_defs
            .iter()
            .position(|m| scope.module().strings.get(m.name) == "add")
            .expect("add method") as u32
            + 1;

        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        let mut output = CaptureOutput::default();
        // pack(19), pack(23) as i32 values.
        let outcome = bridge_call(scope.module(), Some(&loader), &mut output, rid, &[19, 23]).unwrap();
        assert_eq!(outcome.status, STATUS_RETURNED);
        assert_eq!(outcome.packed as i64, 42);
    }

    /// A callee that throws surfaces the throw as a status, not a hard error.
    #[test]
    fn bridges_a_throwing_call_as_a_status() {
        let module = zun_compiler::compile(
            "class Boom {} \
             function boom(n: int): int { if (n < 0) { throw new Boom(); } return n; } \
             function main(): int { return 0; }",
        )
        .expect("compile");
        let loader = {
            let mut loader = ModuleLoader::new();
            loader.load_bytes("app", &module.write()).unwrap();
            loader
        };
        let scope = loader.get("app").unwrap();
        let rid = scope
            .module()
            .tables
            .method_defs
            .iter()
            .position(|m| scope.module().strings.get(m.name) == "boom")
            .expect("boom method") as u32
            + 1;

        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        let mut output = CaptureOutput::default();

        let ok = bridge_call(scope.module(), Some(&loader), &mut output, rid, &[7]).unwrap();
        assert_eq!((ok.status, ok.packed as i64), (STATUS_RETURNED, 7));

        let thrown = bridge_call(scope.module(), Some(&loader), &mut output, rid, &[(-1i64) as u64]).unwrap();
        assert_eq!(thrown.status, STATUS_THREW);
    }

    /// End-to-end: a JIT-compiled method that calls an ineligible, non-throwing
    /// method (one using dynamic-object ops) is eligible via the bridge, and the
    /// bridged call routes through the interpreter to the same result.
    #[test]
    fn jit_bridges_a_call_to_an_ineligible_nonthrowing_callee() {
        let module = zun_compiler::compile(
            "function helper(n: int): int { let o: any = {}; o.v = n; return (o.v) as int; } \
             function caller(n: int): int { return helper(n) + 1; } \
             function main(): int { return 0; }",
        )
        .expect("compile");
        let loader = {
            let mut loader = ModuleLoader::new();
            loader.load_bytes("app", &module.write()).unwrap();
            loader
        };
        let scope = loader.get("app").unwrap();
        let find = |name: &str| {
            scope
                .module()
                .tables
                .method_defs
                .iter()
                .position(|m| scope.module().strings.get(m.name) == name)
                .expect("method") as u32
                + 1
        };
        let (helper_rid, caller_rid) = (find("helper"), find("caller"));

        let jit = unsafe {
            zun_codegen::Jit::compile_with_bridge(scope.module(), bridge_config())
        }
        .expect("jit compiles with bridge");
        assert!(
            !jit.is_compiled(helper_rid),
            "the dynamic helper must stay on the interpreter"
        );
        assert!(
            jit.is_compiled(caller_rid),
            "caller should be JIT-eligible via the bridge to the ineligible helper"
        );

        let mut heap = zun_gc::Heap::new();
        let _hscope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        let mut output = CaptureOutput::default();
        let _bridge = BridgeScope::enter(scope.module(), Some(&loader), &mut output);
        // caller(5) = helper(5) + 1 = 5 + 1 = 6.
        let result = jit
            .call_packed_with_heap(caller_rid, &[5], &mut heap)
            .expect("bridged call");
        assert_eq!(result as i64, 6);
    }

    /// Full exceptional-return channel: a JIT `caller` bridges an ineligible,
    /// throwing `helper`; the throw propagates out of the JIT frame (pending
    /// slot → exceptional exit) back to the interpreter, which routes it to the
    /// same-frame catch in the interpreted `main`.
    #[test]
    fn a_bridged_throw_propagates_to_an_interpreter_catch() {
        let module = zun_compiler::compile(
            "class Boom {} \
             function helper(n: int): int { let o: any = {}; o.v = n; throw new Boom(); return (o.v) as int; } \
             function caller(n: int): int { return helper(n) + 1; } \
             function main(): int { try { return caller(5); } catch (e: Boom) { return 42; } }",
        )
        .expect("compile");
        let bytes = module.write();
        let mut output = CaptureOutput::default();
        // Runs through the production `execute` path (bridge + JitHook active).
        let result = crate::execute_with(&bytes, &mut output).expect("run");
        assert!(
            matches!(result, Value::Int(42, _)),
            "the bridged throw should reach main's catch, got {result:?}"
        );
    }

    /// A bridged throw is routed to a catch in the SAME JIT frame: `run` (JIT)
    /// wraps a call to the throwing dynamic `helper` in a `try`/`catch`, and the
    /// throw is caught in-frame via the runtime type match (no propagation out).
    #[test]
    fn a_bridged_throw_is_caught_in_the_same_jit_frame() {
        let module = zun_compiler::compile(
            "class Boom {} \
             function helper(n: int): int { let o: any = {}; o.v = n; if (n < 0) { throw new Boom(); } return (o.v) as int; } \
             function run(n: int): int { try { return helper(n); } catch (e: Boom) { return 99; } } \
             function main(): int { return run(-1); }",
        )
        .expect("compile");
        let bytes = module.write();
        let mut output = CaptureOutput::default();
        let caught = crate::execute_with(&bytes, &mut output).expect("run");
        assert!(
            matches!(caught, Value::Int(99, _)),
            "the bridged throw should be caught in run's own catch, got {caught:?}"
        );

        // The non-throwing path returns the value through the bridge normally.
        let module_ok = zun_compiler::compile(
            "class Boom {} \
             function helper(n: int): int { let o: any = {}; o.v = n; if (n < 0) { throw new Boom(); } return (o.v) as int; } \
             function run(n: int): int { try { return helper(n); } catch (e: Boom) { return 99; } } \
             function main(): int { return run(7); }",
        )
        .expect("compile");
        let ok = crate::execute_with(&module_ok.write(), &mut output).expect("run");
        assert!(matches!(ok, Value::Int(7, _)), "non-throw path, got {ok:?}");
    }

    /// First-class function: a transient function reference is called indirectly
    /// (`LoadFunction` + `CallIndirect`), dispatched through the bridge to the
    /// target method.
    #[test]
    fn a_transient_indirect_call_dispatches_through_the_bridge() {
        let module = zun_compiler::compile(
            "function target(n: int): int { return n + 10; } \
             function invoke(n: int): int { return (target)(n); } \
             function main(): int { return invoke(5); }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let result = crate::execute_with(&module.write(), &mut output).expect("run");
        assert!(
            matches!(result, Value::Int(15, _)),
            "indirect call should reach target(5)=15, got {result:?}"
        );
    }
}
