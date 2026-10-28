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

use zun_codegen::JitKind;
use zun_interp::value::Value;
use zun_interp::{run_method_on_current_heap, Output, RunError};
use zun_loader::{ModuleLoader, ModuleScope};
use zun_module::read_body;

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
    scope: &ModuleScope,
    loader: &ModuleLoader,
    output: &mut dyn Output,
    rid: u32,
    packed_args: &[u64],
) -> Result<BridgeOutcome, RunError> {
    let module = scope.module();
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

    match run_method_on_current_heap(scope, loader, rid, args, output) {
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
        let outcome = bridge_call(scope, &loader, &mut output, rid, &[19, 23]).unwrap();
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

        let ok = bridge_call(scope, &loader, &mut output, rid, &[7]).unwrap();
        assert_eq!((ok.status, ok.packed as i64), (STATUS_RETURNED, 7));

        let thrown = bridge_call(scope, &loader, &mut output, rid, &[(-1i64) as u64]).unwrap();
        assert_eq!(thrown.status, STATUS_THREW);
    }
}
