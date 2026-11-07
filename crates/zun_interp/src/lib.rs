//! Tier 0: the ZIL interpreter.
//!
//! A straightforward stack-machine loop over a method body's ZIL. It is the
//! always-correct baseline the JIT must match (the equivalence suite in a later
//! phase runs programs through both). Calls recurse through [`Interp::call`];
//! the `print` builtin writes through a pluggable [`Output`] sink so tests can
//! capture it.

mod builtins;
mod call_rest;
mod collections;
mod decorators;
mod dyn_member;
mod errors;
mod eval;
mod gc_policy;
mod hooks;
mod integrity;
mod json;
mod metadata;
mod namespace;
mod native;
mod native_error;
mod node_buffer;
mod operands;
mod output;
mod property_ops;
mod rare_ops;
mod roots;
mod runner;
mod serde;
mod stack;
mod state;
mod string_dyn;
mod typed_arrays;
mod unwind;
pub mod value;

/// Throw a native failure message (`"TypeError: ..."`) from inside the opcode
/// loop: route it to the innermost handler, or propagate it uncaught.
macro_rules! throw_native {
    ($self:ident, $body:ident, $stack:ident, $pc:ident, $at:expr, $message:expr) => {{
        let message: String = $message;
        let exception = native_error::split_native_error(&message)
            .and_then(|(class, text)| $self.make_error_value(class, text));
        let Some(exception) = exception else {
            return Err(RunError::Native(message));
        };
        if $self.route_exception(&$body, &mut $stack, &mut $pc, $at, &exception) {
            continue;
        }
        return Err(RunError::Thrown(exception));
    }};
}

/// Control-flow outcome of a cold opcode arm that lives in its own method
/// outside the dispatch loop (see `dyn_member`, `call_rest`, `rare_ops`).
/// The arm cannot `continue`/`return` the loop directly, so it reports what
/// the dispatcher should do next; a fault travels the `Err` channel instead.
/// The arm methods are `#[inline(always)]` so the compiler folds them back
/// into `call_inner`, keeping the hot dispatch loop's code generation intact.
pub(crate) enum ArmFlow {
    /// Fall through to the next loop iteration (an arm that ran to its end).
    Next,
    /// `continue` the dispatch loop (routed exception or early fast-path exit).
    Continue,
}

/// `throw_native!` for extracted arm methods: identical semantics, but it
/// signals "routed, re-enter the loop" by returning [`ArmFlow::Continue`]
/// instead of executing `continue`. `$body` is already a `&MethodBody`.
macro_rules! throw_native_arm {
    ($self:ident, $body:ident, $stack:ident, $pc:ident, $at:expr, $message:expr) => {{
        let message: String = $message;
        let exception = native_error::split_native_error(&message)
            .and_then(|(class, text)| $self.make_error_value(class, text));
        let Some(exception) = exception else {
            return Err(RunError::Native(message));
        };
        if $self.route_exception($body, &mut $stack, &mut $pc, $at, &exception) {
            return Ok(ArmFlow::Continue);
        }
        return Err(RunError::Thrown(exception));
    }};
}
pub(crate) use throw_native_arm;

use gc_policy::next_gc_interval;
use operands::{
    branch_target, operand_f64, operand_i32, operand_i64, operand_local, operand_strref,
    operand_token, operand_token_imm32,
};
use runner::run_method_with;
pub use runner::{
    GeneratorIteratorResult, GeneratorResult, GeneratorSession, OwnedGeneratorSession,
    ResumableSession, ResumeResult,
};
use stack::{
    bin_float, bin_int, bin_int_minmax, bin_int_pow, bin_int_shift, bin_int_signed_aware, cmp,
    compare, convert, js_max, js_min, js_round, js_sign, pop, pop_n, un_float, un_int_bits,
    underflow, Ordering, ShiftOp,
};
use state::{ExecutionFrame, ExecutionOutcome, Interp, RootLink};

/// Read the result of a settled managed Promise for the synchronous await
/// fast path. Pending promises deliberately return `None` and must be routed
/// through the future continuation/state-machine path.
/// The property-key string for a numeric computed key on a plain dynamic object
/// (`obj[2]` reads/writes property `"2"`, matching JS). Integral values render
/// without a fractional part.
fn number_index_key(key: &Value) -> String {
    match key {
        Value::Int(n, _) => n.to_string(),
        Value::Float(n, _) if n.is_finite() && n.fract() == 0.0 && n.abs() < 9007199254740992.0 => {
            (*n as i64).to_string()
        }
        Value::Float(n, _) => Value::Float(*n, zun_il::NumKind::F64).display(),
        other => other.as_number().to_string(),
    }
}

fn settled_await_value(promise: zun_gc::Obj) -> Result<Option<Value>, RunError> {
    zun_gc::with_current_heap(|heap| {
        match heap.promise_state(promise) {
            Some(zun_core::ManagedPromiseState::Pending) => Ok(None),
            Some(zun_core::ManagedPromiseState::Resolved) => Ok(heap
                .promise_result(promise)
                .and_then(|value| value::Value::unbox_any(value, heap))),
            Some(zun_core::ManagedPromiseState::Rejected) => {
                // A rejection carrying a first-class reason (an object such as
                // an `AggregateError`, or any boxed value) throws that value;
                // otherwise the plain string message is thrown.
                if let Some(reason) = heap.promise_reason(promise) {
                    if let Some(value) = value::Value::unbox_any(reason, heap) {
                        return Err(RunError::Thrown(value));
                    }
                }
                Err(RunError::Thrown(Value::Str(Rc::from(
                    heap.promise_error(promise)
                        .unwrap_or_else(|| "Promise rejected".into()),
                ))))
            }
            None => Err(RunError::BadToken),
        }
    })
}
use std::rc::Rc;
use value::Value;
use zun_il::{decode_one, Opcode, Operand};
use zun_metadata::TableKind;
use zun_module::{read_body, Module};

pub use errors::RunError;
pub use eval::{EvalError, EvalHost, CODEGEN_DISALLOWED};
pub use hooks::CallHook;
pub use namespace::{call_namespace_member, materialize_namespace, zun_dynamic_import};
pub use output::{CaptureOutput, StdoutOutput};
/// Sink for program output: the native modules' [`Sink`](zun_natives::Sink),
/// re-exported so callers keep using `Output`.
pub use zun_natives::Sink as Output;

/// A hook the interpreter consults before interpreting a call, so a faster
/// backend (the JIT) can execute an eligible method instead. Returning `None`
/// means "not handled — interpret it".
/// Run a module's entry point with no arguments, on the interpreter alone.
pub fn run(module: &Module, output: &mut dyn Output) -> Result<Value, RunError> {
    run_with(module, output, None)
}

/// Run a module's entry point, consulting `hook` on each call (tiering: the JIT
/// handles eligible methods, the interpreter the rest).
pub fn run_with(
    module: &Module,
    output: &mut dyn Output,
    hook: Option<&dyn CallHook>,
) -> Result<Value, RunError> {
    let entry = module.entry_point.ok_or(RunError::NoEntryPoint)?;
    run_method_with(module, entry.rid(), Vec::new(), output, hook, None, None)
}

/// Run a module's entry point with an [`EvalHost`] that compiles source strings for
/// `new Function(...)`. Every other entry point has no host, so that intrinsic throws
/// "code generation from strings disallowed".
pub fn run_with_eval_host(
    module: &Module,
    output: &mut dyn Output,
    hook: Option<&dyn CallHook>,
    eval_host: Option<&dyn EvalHost>,
) -> Result<Value, RunError> {
    let entry = module.entry_point.ok_or(RunError::NoEntryPoint)?;
    run_method_with(
        module,
        entry.rid(),
        Vec::new(),
        output,
        hook,
        None,
        eval_host,
    )
}

/// Execute a specific method body with positional arguments.
///
/// Hosts use this primitive when a loader resolves a method outside the entry
/// module. The method still runs with the same heap, native output, and GC
/// safeguards as entry-point execution.
pub fn run_method(
    module: &Module,
    method_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    run_method_with(module, method_rid, args, output, None, None, None)
}

/// Execute a callable function handle with its captured environment.
///
/// Closure environments are represented as the hidden first argument, exactly
/// as they are for `CallIndirect`; hosts and schedulers can therefore invoke a
/// queued callback without reconstructing its capture layout.
pub fn run_function(
    module: &Module,
    handle: zun_core::FunctionHandle,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    let mut call_args = Vec::with_capacity(args.len() + 1);
    if let Some(address) = handle.environment() {
        let environment = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
        call_args.push(Value::Object(environment));
    }
    call_args.extend(args);
    run_method(module, handle.method_rid(), call_args, output)
}

/// Invoke a callable function handle with its captured environment on the
/// **current** heap (no fresh heap, no module initializer), for the
/// JIT→interpreter call bridge's `CallIndirect` dispatch.
///
/// Like [`run_function`], the closure environment (or bound record) is supplied
/// as the hidden first argument, so a closure keeps reading its captures; unlike
/// it, execution reuses the JIT frame's live heap (where the environment object
/// and managed arguments already live). `user_args` are the call's explicit
/// arguments, in source order, without the hidden environment slot.
///
/// This handles the concrete function values a JIT `CallIndirect` can hold:
/// a plain function (`environment() == None`) and a closure of a specific
/// function (`environment() == Some(address)`). A bound function is not
/// reachable here — it is a dynamic function value, kept on Tier 0 — so its
/// dedicated dispatch is intentionally not duplicated.
pub fn run_function_on_current_heap_bare(
    module: &Module,
    loader: Option<&zun_loader::ModuleLoader>,
    handle: zun_core::FunctionHandle,
    user_args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    let mut call_args = Vec::with_capacity(user_args.len() + 1);
    if let Some(address) = handle.environment() {
        let environment = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
        call_args.push(Value::Object(environment));
    }
    call_args.extend(user_args);
    run_method_on_current_heap_bare(module, loader, handle.method_rid(), call_args, output)
}

/// Execute a method from a loaded module set, allowing top-level MemberRef calls.
pub fn run_method_loaded(
    scope: &zun_loader::ModuleScope,
    loader: &zun_loader::ModuleLoader,
    method_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    run_method_with(
        scope.module(),
        method_rid,
        args,
        output,
        None,
        Some(loader),
        None,
    )
}

/// Run a loaded method on the **current** heap (no fresh heap, no module
/// initializer), for callers that already hold a heap with live managed values —
/// notably the JIT→interpreter call bridge, which invokes an ineligible callee
/// from a JIT frame on that frame's heap. Thin public wrapper over
/// [`runner::run_method_loaded_on_current_heap`].
pub fn run_method_on_current_heap(
    scope: &zun_loader::ModuleScope,
    loader: &zun_loader::ModuleLoader,
    method_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    runner::run_method_loaded_on_current_heap(scope, loader, method_rid, args, output)
}

/// As [`run_method_on_current_heap`], but taking the module directly and an
/// optional loader (for the JIT→interpreter bridge in a single-module run).
pub fn run_method_on_current_heap_bare(
    module: &Module,
    loader: Option<&zun_loader::ModuleLoader>,
    method_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    runner::run_method_loaded_on_current_heap_bare(module, loader, method_rid, args, output)
}

impl Interp<'_> {
    /// Create the managed identity of a generator invocation. The execution
    /// body is deliberately not entered here; `generatorIteratorNext` owns
    /// resume and stores the suspended state after the first yield.
    /// Whether the method body at `method_rid` contains a `yield` (cached).
    pub(crate) fn method_contains_yield(&self, method_rid: u32) -> bool {
        thread_local! {
            static CACHE: std::cell::RefCell<std::collections::HashMap<(usize, u32), bool>> =
                std::cell::RefCell::new(std::collections::HashMap::new());
        }
        let key = (self.module as *const Module as usize, method_rid);
        if let Some(known) = CACHE.with(|cache| cache.borrow().get(&key).copied()) {
            return known;
        }
        let found = (|| {
            let method = self
                .module
                .tables
                .method_defs
                .get((method_rid as usize).checked_sub(1)?)?;
            let body = read_body(&self.module.code, method.body_offset).ok()?;
            let mut offset = 0;
            while offset < body.code.len() {
                let (instruction, size) = decode_one(&body.code[offset..]).ok()?;
                if instruction.opcode == Opcode::AsyncStart {
                    return Some(false);
                }
                if instruction.opcode == Opcode::Yield {
                    return Some(true);
                }
                offset += size;
            }
            Some(false)
        })()
        .unwrap_or(false);
        CACHE.with(|cache| cache.borrow_mut().insert(key, found));
        found
    }

    pub(crate) fn create_generator(
        &mut self,
        method_rid: u32,
        args: &[Value],
    ) -> Result<Value, RunError> {
        let method = self
            .module
            .tables
            .method_defs
            .get(method_rid as usize - 1)
            .ok_or(RunError::BadToken)?;
        let body = read_body(&self.module.code, method.body_offset)
            .map_err(|error| RunError::Malformed(error.to_string()))?;
        let mut offset = 0;
        let mut is_generator = false;
        while offset < body.code.len() {
            let (instruction, size) = decode_one(&body.code[offset..])
                .map_err(|error| RunError::Malformed(error.to_string()))?;
            is_generator |= instruction.opcode == Opcode::Yield;
            offset += size;
        }
        if !is_generator {
            return Err(RunError::BadToken);
        }
        let boxed = zun_gc::with_current_heap_mut(|heap| {
            args.iter()
                .map(|value| crate::collections::gen_box(value, heap))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let object =
            zun_gc::with_current_heap_mut(|heap| heap.alloc_generator(0, method_rid, boxed));
        self.finish_generator_creation(method_rid, object)
    }

    /// Call the method at `method_rid` (1-based) with `args`, returning its
    /// result ([`Value::Void`] for a void method).
    ///
    /// This wrapper keeps `self.call_stack` balanced: it pushes the activation
    /// on entry and pops it on every exit path (including the error returns
    /// deep inside [`Self::call_inner`]), so a runtime stack capture (see
    /// `Error.stack`) sees exactly the methods currently executing.
    fn call(
        &mut self,
        method_rid: u32,
        args: Vec<Value>,
        parent: Option<&RootLink>,
    ) -> Result<Value, RunError> {
        self.call_stack
            .push(crate::state::CallFrame { method_rid, pc: 0 });
        let result = self.call_inner(method_rid, args, parent);
        self.call_stack.pop();
        result
    }

    fn call_inner(
        &mut self,
        method_rid: u32,
        mut args: Vec<Value>,
        parent: Option<&RootLink>,
    ) -> Result<Value, RunError> {
        let method = self
            .module
            .tables
            .method_defs
            .get(method_rid as usize - 1)
            .ok_or(RunError::BadToken)?;
        if method.body_offset == u32::MAX {
            return Err(RunError::BadToken);
        }
        let body = read_body(&self.module.code, method.body_offset)
            .map_err(|e| RunError::Malformed(e.to_string()))?;

        let resumed = self.resume_frame.is_some();
        if resumed {
            // A resumed coroutine frame never consumes the async-start bypass; leaving
            // it set would let a callee async function skip its coroutine setup.
            unwind::ASYNC_BYPASS.with(|flag| flag.set(false));
        }
        let mut frame = if let Some(frame) = self.resume_frame.take() {
            if frame.method_rid != method_rid {
                return Err(RunError::Malformed(
                    "resumed frame method does not match dispatch target".into(),
                ));
            }
            frame
        } else {
            // Remember how many arguments the caller really passed (a static call
            // site announces it; dynamic calls pass exactly what they carry).
            let announced = unwind::CALL_ARGC_MARK.with(|mark| mark.take());
            unwind::CALL_ARGC_CURRENT.with(|current| current.set(announced.unwrap_or(args.len())));
            // Dynamic-call argument adaptation (ECMAScript semantics): a
            // closure with a rest parameter (`<closure@..@rest>`) packs its
            // trailing arguments into an array; any other callee drops
            // surplus arguments (missing ones read as `undefined`).
            let declared = body.param_count as usize;
            if self.module.strings.get(method.name).ends_with("@rest>") && declared > 0 {
                let fixed = declared - 1;
                let rest_values: Vec<Value> = if args.len() > fixed {
                    args.split_off(fixed)
                } else {
                    Vec::new()
                };
                args.resize(fixed, Value::Void);
                let rest_array = zun_gc::with_current_heap_mut(|heap| -> Result<_, RunError> {
                    let array = heap.alloc_array(rest_values.len());
                    for (index, value) in rest_values.iter().enumerate() {
                        // A function has no `Any` form: it rides in a `functionBox` record.
                        let any = match value {
                            Value::Function(handle) => {
                                let record = heap.alloc_dynamic();
                                heap.dynamic_set_function(record, "__boxedFn", *handle);
                                Value::Object(record).box_any(heap)
                            }
                            other => other.box_any(heap),
                        }
                        .ok_or(RunError::BadToken)?;
                        let _ = heap.array_set(array, index, any);
                    }
                    Ok(array)
                })?;
                args.push(Value::Object(rest_array));
            } else if args.len() > declared {
                args.truncate(declared);
            }
            // Coerce boxed `Any` arguments to the callee's typed parameter
            // representation (dynamic callers cannot know the parameter types).
            for (index, arg) in args.iter_mut().enumerate() {
                if let Value::Any(any) = *arg {
                    let kind = body.local_kinds.get(index).copied().unwrap_or(255);
                    let unboxed = zun_gc::with_current_heap_mut(|heap| Value::unbox_any(any, heap));
                    if let Some(inner) = unboxed {
                        use zun_il::{JitKind, NumKind};
                        let coerced = match (JitKind::from_byte(kind), inner) {
                            (
                                JitKind::Num(n @ (NumKind::F32 | NumKind::F64)),
                                Value::Float(f, _),
                            ) => Some(Value::Float(f, n)),
                            (JitKind::Num(n @ (NumKind::F32 | NumKind::F64)), Value::Int(i, _)) => {
                                Some(Value::Float(i as f64, n))
                            }
                            (JitKind::Num(n @ (NumKind::F32 | NumKind::F64)), Value::Bool(b)) => {
                                Some(Value::Float(if b { 1.0 } else { 0.0 }, n))
                            }
                            (JitKind::Num(n @ (NumKind::F32 | NumKind::F64)), Value::Void) => {
                                Some(Value::Float(f64::NAN, n))
                            }
                            (JitKind::Num(n @ (NumKind::F32 | NumKind::F64)), Value::Null) => {
                                Some(Value::Float(0.0, n))
                            }
                            (JitKind::Num(n), Value::Int(i, _)) => Some(Value::Int(i, n)),
                            (JitKind::Num(n), Value::Float(f, _)) => Some(Value::Int(f as i64, n)),
                            (JitKind::Bool, Value::Bool(b)) => Some(Value::Bool(b)),
                            (JitKind::String, Value::Str(s)) => Some(Value::Str(s)),
                            (JitKind::Object | JitKind::Array, Value::Object(o)) => {
                                Some(Value::Object(o))
                            }
                            _ => None,
                        };
                        if let Some(v) = coerced {
                            *arg = v;
                        }
                    }
                }
            }
            if matches!(decode_one(body.code), Ok((first, _)) if first.opcode == Opcode::AsyncStart)
                && !unwind::ASYNC_BYPASS.with(|flag| flag.replace(false))
            {
                return self.async_start(method_rid, args);
            }
            let mut locals: Vec<Value> = Vec::with_capacity(body.local_count as usize);
            locals.extend(args);
            locals.resize(body.local_count as usize, Value::Void);
            ExecutionFrame::new(method_rid, locals)
        };
        let mut stack = &mut frame.stack;
        let locals = &mut frame.locals;
        let code = body.code;
        let mut pc = &mut frame.pc;

        if resumed && unwind::GENERATOR_RETURN_PENDING.with(|flag| flag.replace(false)) {
            // Close protocol: resume the generator as a `return()` completion so
            // only its `finally` clauses run.
            let sentinel = Value::Str(unwind::GENERATOR_RETURN_SENTINEL.into());
            let throw_pc = (*pc).saturating_sub(1);
            if !self.route_exception(&body, stack, pc, throw_pc, &sentinel) {
                return Err(RunError::Thrown(sentinel));
            }
        }

        if resumed {
            if let Some(thrown) =
                unwind::GENERATOR_THROW_PENDING.with(|slot| slot.borrow_mut().take())
            {
                let throw_pc = (*pc).saturating_sub(1);
                if !self.route_exception(&body, stack, pc, throw_pc, &thrown) {
                    return Err(RunError::Thrown(thrown));
                }
            }
        }

        while *pc < code.len() {
            self.active_frame = Some((method_rid, *pc as u32));
            if let Some(frame) = self.call_stack.last_mut() {
                frame.pc = *pc as u32;
            }
            let (instr, size) =
                decode_one(&code[*pc..]).map_err(|e| RunError::Malformed(e.to_string()))?;
            let next = *pc + size;
            *pc = next;

            match instr.opcode {
                Opcode::Nop | Opcode::AsyncStart | Opcode::AsyncGenStart => {}
                Opcode::LoadStatic => {
                    let token = operand_token(&instr.operand);
                    let (owner, field) = self.static_field_key(token)?;
                    let kind = self.static_field_kind(token)?;
                    let value = zun_gc::with_current_heap(|heap| {
                        // Prefer the data slot; a function-valued static (closures
                        // do not box into `Any`) rides the function slot instead.
                        if let Some(any) = heap.static_get(owner, field) {
                            let value = Value::unbox_any(any, heap).unwrap_or(Value::Void);
                            Some(crate::value::retag_static_numeric(value, kind))
                        } else {
                            heap.static_get_function(owner, field).map(Value::Function)
                        }
                    })
                    .unwrap_or(Value::Void);
                    stack.push(value);
                }
                Opcode::StoreStatic => {
                    let (owner, field) = self.static_field_key(operand_token(&instr.operand))?;
                    let value = pop(stack)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Value::Function(handle) = value {
                            heap.static_set_function(owner, field, handle);
                        } else {
                            let any = value.box_any(heap).unwrap_or_else(zun_core::Any::null);
                            heap.static_set(owner, field, any);
                        }
                    });
                    stack.push(value);
                }
                Opcode::ConstF64 => stack.push(Value::Float(
                    operand_f64(&instr.operand),
                    zun_il::NumKind::F64,
                )),
                Opcode::ConstStr => {
                    let idx = operand_strref(&instr.operand);
                    let s: Rc<str> = self
                        .module
                        .strings
                        .get(zun_metadata::StringIndex(idx))
                        .into();
                    stack.push(Value::Str(s));
                }
                Opcode::ConstTrue => stack.push(Value::Bool(true)),
                Opcode::ConstFalse => stack.push(Value::Bool(false)),
                Opcode::LoadFunction => {
                    let token = match instr.operand {
                        Operand::Token(token)
                            if token.table() == Some(zun_metadata::TableKind::MethodDef) =>
                        {
                            token
                        }
                        _ => return Err(RunError::BadToken),
                    };
                    let rid = token.rid();
                    if rid == 0
                        || self
                            .module
                            .tables
                            .method_defs
                            .get(rid as usize - 1)
                            .is_none()
                    {
                        return Err(RunError::BadToken);
                    }
                    stack.push(Value::Function(zun_core::FunctionHandle::new(rid, None)));
                }
                Opcode::MakeClosure => {
                    let token = match instr.operand {
                        Operand::Token(token)
                            if token.table() == Some(zun_metadata::TableKind::MethodDef) =>
                        {
                            token
                        }
                        _ => return Err(RunError::BadToken),
                    };
                    let rid = token.rid();
                    if rid == 0
                        || self
                            .module
                            .tables
                            .method_defs
                            .get(rid as usize - 1)
                            .is_none()
                    {
                        return Err(RunError::BadToken);
                    }
                    let environment = pop(stack)?;
                    let Value::Object(environment) = environment else {
                        return Err(RunError::BadToken);
                    };
                    stack.push(Value::Function(zun_core::FunctionHandle::new(
                        rid,
                        Some(environment.addr() as usize),
                    )));
                }
                Opcode::BoxAny => {
                    let value = pop(stack)?;
                    // A callable has no `Any` representation; it stays a raw
                    // function value (a null-sentinel default check, for one,
                    // only needs to see that it is not null).
                    if matches!(value, Value::Function(_)) {
                        stack.push(value);
                    } else {
                        let any = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                            .ok_or(RunError::BadToken)?;
                        stack.push(Value::Any(any));
                    }
                }
                Opcode::UnboxAny => {
                    let value = pop(stack)?;
                    if let Value::Any(any) = value {
                        let value =
                            zun_gc::with_current_heap(|heap| value::Value::unbox_any(any, heap))
                                .ok_or(RunError::BadToken)?;
                        stack.push(value);
                    } else {
                        // Array slots retain the tagged runtime value directly;
                        // an explicit numeric cast must accept that representation
                        // as well as a heap-backed `Any` box.
                        stack.push(value);
                    }
                }
                Opcode::CastClass => {
                    match self.op_cast_class(stack, pc, &body, instr, next, size)? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::NewDynamic => {
                    let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                    stack.push(Value::Object(object));
                }
                Opcode::RealmGlobal => {
                    let object = zun_gc::with_current_heap_mut(|heap| heap.global_object());
                    stack.push(Value::Object(object));
                }
                Opcode::LoadDynamic => {
                    match self.op_load_dynamic(stack, pc, &body, instr, next, size)? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::StoreDynamic => {
                    match self.op_store_dynamic(stack, pc, &body, instr, next, size)? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::LoadDynamicValue => {
                    match self.op_load_dynamic_value(stack, pc, &body, next, size)? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::StoreDynamicValue => {
                    match self.op_store_dynamic_value(stack, pc, &body, next, size)? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::NewArray => {
                    let length = pop(stack)?.as_int();
                    if length < 0 {
                        return Err(RunError::Malformed("negative array length".into()));
                    }
                    let array =
                        zun_gc::with_current_heap_mut(|heap| heap.alloc_array(length as usize));
                    stack.push(Value::Object(array));
                }
                Opcode::LoadArray => {
                    let index = pop(stack)?.as_int();
                    let array = pop(stack)?.as_object();
                    if index < 0 {
                        return Err(RunError::Malformed("negative array index".into()));
                    }
                    // Reading past the end answers `undefined`, as in JS.
                    let past_end = zun_gc::with_current_heap(|heap| {
                        heap.array_len(array)
                            .is_some_and(|len| (index as usize) >= len)
                    });
                    if past_end {
                        stack.push(Value::Void);
                        continue;
                    }
                    let value = zun_gc::with_current_heap(|heap| {
                        heap.array_get(array, index as usize)
                            .and_then(|any| value::Value::unbox_any(any, heap))
                    })
                    .ok_or(RunError::BadToken)?;
                    stack.push(value);
                }
                Opcode::StoreArray => {
                    let value = pop(stack)?;
                    let index = pop(stack)?.as_int();
                    let array = pop(stack)?.as_object();
                    if index < 0 {
                        return Err(RunError::Malformed("negative array index".into()));
                    }
                    if let Err(RunError::Native(message)) =
                        self.guard_array(array, integrity::ArrayOp::Write)
                    {
                        throw_native!(self, body, stack, pc, next - size, message);
                    }
                    // A function element rides the array as a function-box record
                    // (a callable is not boxable into `Any` directly); calls
                    // through the element unwrap it again.
                    let any = zun_gc::with_current_heap_mut(|heap| match &value {
                        Value::Function(handle) => {
                            let record = heap.alloc_dynamic();
                            heap.dynamic_set_function(record, "__boxedFn", *handle);
                            Some(zun_core::Any::object(record.addr() as usize))
                        }
                        other => other.box_any(heap),
                    })
                    .ok_or(RunError::BadToken)?;
                    // Writing at or past the end extends the array (JS semantics);
                    // the gap slots stay empty.
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(len) = heap.array_len(array) {
                            if (index as usize) >= len {
                                heap.array_resize(array, index as usize + 1);
                                for gap in len..(index as usize) {
                                    heap.array_set(array, gap, zun_core::Any::hole());
                                }
                            }
                        }
                        heap.array_set(array, index as usize, any)
                    })
                    .ok_or(RunError::BadToken)?;
                    stack.push(value);
                }
                Opcode::ArrayLength => {
                    let array = pop(stack)?.as_object();
                    let length = zun_gc::with_current_heap(|heap| heap.array_len(array))
                        .ok_or(RunError::BadToken)?;
                    stack.push(Value::Int(length as i64, zun_il::NumKind::U64));
                }
                Opcode::ArrayPop => {
                    let array = pop(stack)?.as_object();
                    // `pop()` on an empty array answers `undefined`.
                    if zun_gc::with_current_heap(|heap| heap.array_len(array)) == Some(0) {
                        stack.push(Value::Void);
                        continue;
                    }
                    if zun_gc::with_current_heap(|heap| heap.array_len(array)) != Some(0) {
                        if let Err(RunError::Native(message)) =
                            self.guard_array(array, integrity::ArrayOp::Remove)
                        {
                            throw_native!(self, body, stack, pc, next - size, message);
                        }
                    }
                    let value = zun_gc::with_current_heap_mut(|heap| {
                        heap.array_pop(array)
                            .and_then(|any| value::Value::unbox_any(any, heap))
                    })
                    .ok_or(RunError::BadToken)?;
                    stack.push(value);
                }
                Opcode::ArrayPush => {
                    let value = pop(stack)?;
                    let array = pop(stack)?.as_object();
                    if let Err(RunError::Native(message)) =
                        self.guard_array(array, integrity::ArrayOp::Add)
                    {
                        throw_native!(self, body, stack, pc, next - size, message);
                    }
                    let any = zun_gc::with_current_heap_mut(|heap| match &value {
                        Value::Function(handle) => {
                            let record = heap.alloc_dynamic();
                            heap.dynamic_set_function(record, "__boxedFn", *handle);
                            Some(zun_core::Any::object(record.addr() as usize))
                        }
                        other => other.box_any(heap),
                    })
                    .ok_or(RunError::BadToken)?;
                    let length = zun_gc::with_current_heap_mut(|heap| heap.array_push(array, any))
                        .ok_or(RunError::BadToken)?;
                    stack.push(Value::Int(length as i64, zun_il::NumKind::U64));
                }
                Opcode::LoadLocal => stack.push(locals[operand_local(&instr.operand)].clone()),
                Opcode::StoreLocal => {
                    let v = pop(stack)?;
                    locals[operand_local(&instr.operand)] = v;
                }
                Opcode::Pop => {
                    pop(stack)?;
                }
                Opcode::Dup => {
                    let top = stack.last().cloned().ok_or_else(underflow)?;
                    stack.push(top);
                }
                Opcode::AddF64 => bin_float(stack, |a, b| a + b)?,
                Opcode::SubF64 => bin_float(stack, |a, b| a - b)?,
                Opcode::MulF64 => bin_float(stack, |a, b| a * b)?,
                Opcode::DivF64 => bin_float(stack, |a, b| a / b)?,
                Opcode::RemF64 => bin_float(stack, |a, b| a % b)?,
                // Float math machine ops. Tier 1 lowers each of these to the
                // matching Cranelift instruction; these are the reference
                // results that equivalence testing holds it to.
                Opcode::SqrtF64 => un_float(stack, f64::sqrt)?,
                Opcode::AbsF64 => un_float(stack, f64::abs)?,
                Opcode::NegF64 => un_float(stack, |a| -a)?,
                Opcode::FloorF64 => un_float(stack, f64::floor)?,
                Opcode::CeilF64 => un_float(stack, f64::ceil)?,
                Opcode::TruncF64 => un_float(stack, f64::trunc)?,
                Opcode::MinF64 => bin_float(stack, js_min)?,
                Opcode::MaxF64 => bin_float(stack, js_max)?,
                // Integer machine ops. Widths come from the operand kind, so
                // the bit counts are relative to that width, not to i64.
                Opcode::RoundF64 => un_float(stack, js_round)?,
                Opcode::SignF64 => un_float(stack, js_sign)?,
                Opcode::MinI32 => bin_int_minmax(stack, true)?,
                Opcode::MaxI32 => bin_int_minmax(stack, false)?,
                Opcode::ClzI32 => un_int_bits(stack, |value, bits| {
                    (value.leading_zeros() - (64 - bits)) as u64
                })?,
                Opcode::CtzI32 => un_int_bits(stack, |value, bits| {
                    if value == 0 {
                        bits as u64
                    } else {
                        value.trailing_zeros() as u64
                    }
                })?,
                Opcode::PopcntI32 => un_int_bits(stack, |value, _| value.count_ones() as u64)?,
                // Integer arithmetic is width-generic here: the result kind
                // comes from the operands (which sema guarantees match) and the
                // value wraps to that width. A future typed-stack JIT will
                // specialize these by stack type (PLAN.md §4.2).
                Opcode::ConstI32 => stack.push(Value::Int(
                    operand_i32(&instr.operand) as i64,
                    zun_il::NumKind::I32,
                )),
                Opcode::ConstI64 => stack.push(Value::Int(
                    operand_i64(&instr.operand),
                    zun_il::NumKind::I64,
                )),
                Opcode::AddI32 => bin_int(stack, |a, b| a.wrapping_add(b))?,
                Opcode::SubI32 => bin_int(stack, |a, b| a.wrapping_sub(b))?,
                Opcode::MulI32 => bin_int(stack, |a, b| a.wrapping_mul(b))?,
                Opcode::DivI32 => bin_int_signed_aware(stack, true)?,
                Opcode::RemI32 => bin_int_signed_aware(stack, false)?,
                Opcode::NegI32 => {
                    let v = pop(stack)?;
                    let k = v.int_kind();
                    stack.push(Value::Int(value::wrap_int(v.as_int().wrapping_neg(), k), k));
                }
                // Integer bitwise, shift, and exponent machine ops, width- and
                // signedness-generic from the operand kind.
                Opcode::AndI32 => bin_int(stack, |a, b| a & b)?,
                Opcode::OrI32 => bin_int(stack, |a, b| a | b)?,
                Opcode::XorI32 => bin_int(stack, |a, b| a ^ b)?,
                Opcode::NotI32 => {
                    let v = pop(stack)?;
                    let k = v.int_kind();
                    stack.push(Value::Int(value::wrap_int(!v.as_int(), k), k));
                }
                Opcode::ShlI32 => bin_int_shift(stack, ShiftOp::Left)?,
                Opcode::ShrI32 => bin_int_shift(stack, ShiftOp::Right)?,
                Opcode::UShrI32 => bin_int_shift(stack, ShiftOp::UnsignedRight)?,
                Opcode::PowI32 => bin_int_pow(stack)?,
                Opcode::ConstUndef => stack.push(Value::Void),
                Opcode::Conv => {
                    let target = zun_il::NumKind::from_imm(operand_i32(&instr.operand))
                        .ok_or_else(|| RunError::Malformed("bad conv kind".into()))?;
                    let v = pop(stack)?;
                    stack.push(convert(&v, target)?);
                }
                Opcode::Concat => {
                    let b = pop(stack)?;
                    let a = pop(stack)?;
                    // A string carried through `any` is unboxed before joining.
                    let unbox = |value: Value| match value {
                        Value::Any(any) => {
                            zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                                .unwrap_or(Value::Any(any))
                        }
                        other => other,
                    };
                    let (a, b) = (unbox(a), unbox(b));
                    stack.push(Value::Str(
                        zun_core::wtf16::concat(a.as_str(), b.as_str()).into(),
                    ));
                }
                Opcode::CmpEq => cmp(stack, true)?,
                Opcode::CmpNe => cmp(stack, false)?,
                Opcode::CmpLt | Opcode::CmpLe | Opcode::CmpGt | Opcode::CmpGe => {
                    // Objects run ToPrimitive(number) and a string meeting a non-string
                    // compares numerically, before the primitive comparison.
                    match self.coerce_relational_operands(stack) {
                        Ok(()) => {}
                        Err(RunError::Thrown(exception)) => {
                            if self.route_exception(
                                &body,
                                stack,
                                pc,
                                next - size,
                                &exception,
                            ) {
                                continue;
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(RunError::Native(message)) if message.starts_with("TypeError: ") => {
                            throw_native!(self, body, stack, pc, next - size, message);
                        }
                        Err(error) => return Err(error),
                    }
                    let ordering = match instr.opcode {
                        Opcode::CmpLt => Ordering::Lt,
                        Opcode::CmpLe => Ordering::Le,
                        Opcode::CmpGt => Ordering::Gt,
                        _ => Ordering::Ge,
                    };
                    compare(stack, ordering)?
                }
                Opcode::Branch => {
                    let target = branch_target(next, &instr.operand);
                    if target < *pc {
                        if let Some(hook) = self.hook {
                            hook.on_backedge_state(method_rid, target, *pc, stack, locals);
                        }
                        self.poll_gc(stack, locals, parent);
                    }
                    *pc = target;
                }
                Opcode::BranchTrue => {
                    let condition = pop(stack)?;
                    if zun_gc::with_current_heap(|heap| {
                        crate::collections::is_truthy(&condition, heap)
                    }) {
                        let target = branch_target(next, &instr.operand);
                        if target < *pc {
                            if let Some(hook) = self.hook {
                                hook.on_backedge_state(method_rid, target, *pc, stack, locals);
                            }
                            self.poll_gc(stack, locals, parent);
                        }
                        *pc = target;
                    }
                }
                Opcode::BranchFalse => {
                    let condition = pop(stack)?;
                    if !zun_gc::with_current_heap(|heap| {
                        crate::collections::is_truthy(&condition, heap)
                    }) {
                        let target = branch_target(next, &instr.operand);
                        if target < *pc {
                            if let Some(hook) = self.hook {
                                hook.on_backedge_state(method_rid, target, *pc, stack, locals);
                            }
                            self.poll_gc(stack, locals, parent);
                        }
                        *pc = target;
                    }
                }
                Opcode::Ret => {
                    return Ok(stack.pop().unwrap_or(Value::Void));
                }
                Opcode::Throw => match self.op_throw(stack, pc, &body, size)? {
                    ArmFlow::Next => {}
                    ArmFlow::Continue => continue,
                },
                Opcode::Rethrow => match self.op_rethrow(stack, pc, &body, next, size)? {
                    ArmFlow::Next => {}
                    ArmFlow::Continue => continue,
                },
                Opcode::Call => {
                    let token = operand_token(&instr.operand);
                    if token.table() == Some(TableKind::MemberRef) {
                        let member = self
                            .module
                            .tables
                            .member_refs
                            .get(token.rid() as usize - 1)
                            .ok_or(RunError::BadToken)?;
                        let module_ref = self
                            .module
                            .tables
                            .module_refs
                            .get(member.parent.rid() as usize - 1)
                            .ok_or(RunError::BadToken)?;
                        let module_name = self.module.strings.get(module_ref.name);
                        let member_name = self.module.strings.get(member.name);
                        if let Some(hook) = self.hook {
                            let signature = self.module.blobs.get(member.signature);
                            let arity = signature.first().copied().map(usize::from);
                            if let Some(arity) = arity {
                                if stack.len() < arity {
                                    return Err(RunError::BadToken);
                                }
                                let start = stack.len() - arity;
                                if let Some(result) = hook.try_call_external_typed(
                                    module_name,
                                    member_name,
                                    signature,
                                    &stack[start..],
                                ) {
                                    stack.truncate(start);
                                    stack.push(result?);
                                    continue;
                                }
                            }
                        }
                        let loader = self.loader.ok_or(RunError::BadToken)?;
                        let resolved = loader
                            .resolve_member_ref_in_module(self.module, token)
                            .map_err(|_| RunError::BadToken)?;
                        let callee = resolved.scope.module();
                        let body = read_body(
                            &callee.code,
                            callee.tables.method_defs[resolved.token.rid() as usize - 1]
                                .body_offset,
                        )
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                        let args = pop_n(stack, body.param_count as usize)?;
                        if args.iter().any(|arg| matches!(arg, Value::Object(_))) {
                            return Err(RunError::BadToken);
                        }
                        let result = match run_method_loaded(
                            resolved.scope,
                            loader,
                            resolved.token.rid(),
                            args,
                            self.output,
                        ) {
                            Ok(value) => value,
                            Err(RunError::Thrown(exception)) => {
                                if self.route_exception(
                                    &body,
                                    stack,
                                    pc,
                                    next - size,
                                    &exception,
                                ) {
                                    continue;
                                }
                                return Err(RunError::Thrown(exception));
                            }
                            Err(error) => return Err(error),
                        };
                        stack.push(result);
                        continue;
                    }
                    if token.table() != Some(TableKind::MethodDef) {
                        return Err(RunError::BadToken);
                    }
                    let callee_rid = token.rid();
                    let callee = self
                        .module
                        .tables
                        .method_defs
                        .get(callee_rid as usize - 1)
                        .ok_or(RunError::BadToken)?;
                    if callee.body_offset == u32::MAX {
                        return Err(RunError::BadToken);
                    }
                    let callee_body = read_body(&self.module.code, callee.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    let args = pop_n(stack, callee_body.param_count as usize)?;
                    let link = RootLink {
                        stack: std::ptr::from_mut(stack.as_mut_slice()),
                        locals: std::ptr::from_mut(locals.as_mut_slice()),
                        parent,
                    };
                    let result = match self.dispatch_outcome(callee_rid, args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                stack,
                                pc,
                                next - size,
                                &exception,
                            ) {
                                continue;
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        ExecutionOutcome::Fault(error) => return Err(error),
                        ExecutionOutcome::Suspended(frame) => {
                            return Err(self.propagate_suspended(
                                frame,
                                ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::CallRest => {
                    match self.op_call_rest(
                        stack, pc, locals, &body, instr, next, size, method_rid, parent,
                    )? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::CallRestArray => {
                    match self.op_call_rest_array(
                        stack, pc, locals, &body, instr, next, size, method_rid, parent,
                    )? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::CallRestArrayFixed => {
                    match self.op_call_rest_array_fixed(
                        stack, pc, locals, &body, instr, next, size, method_rid, parent,
                    )? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::CallIndirect | Opcode::CallIndirectN => {
                    // `call.indirect.n` is the dynamic form with an explicit
                    // argument count (operand) instead of a signature token.
                    let counted = instr.opcode == Opcode::CallIndirectN;
                    let token = if counted {
                        zun_metadata::Token::null(TableKind::MethodDef)
                    } else {
                        operand_token(&instr.operand)
                    };
                    if !matches!(
                        token.table(),
                        Some(TableKind::MethodDef | TableKind::MemberRef)
                    ) {
                        return Err(RunError::BadToken);
                    }
                    let dynamic = token.rid() == 0;
                    let reducer = dynamic && token.table() == Some(TableKind::MemberRef);
                    let expected_params = if counted {
                        let count = operand_i32(&instr.operand);
                        if count < 0 {
                            return Err(RunError::BadToken);
                        }
                        count as usize
                    } else if dynamic {
                        if reducer {
                            2
                        } else {
                            1
                        }
                    } else {
                        let signature = self
                            .module
                            .tables
                            .method_defs
                            .get(token.rid() as usize - 1)
                            .ok_or(RunError::BadToken)?;
                        read_body(&self.module.code, signature.body_offset)
                            .map_err(|e| RunError::Malformed(e.to_string()))?
                            .param_count as usize
                    };
                    let args = pop_n(stack, expected_params)?;
                    let function = pop(stack)?;
                    // A function value carried through `any` is unboxed first.
                    let function = match function {
                        Value::Any(any) => {
                            zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                                .unwrap_or(Value::Any(any))
                        }
                        other => other,
                    };
                    // A function stored in an array/`any` slot rides a `__boxedFn` record.
                    let function = zun_gc::with_current_heap(|heap| {
                        crate::collections::unbox_function_box(function, heap)
                    });
                    let Value::Function(handle) = function else {
                        // Calling a non-callable value is a catchable TypeError.
                        let what = match &function {
                            Value::Void => "undefined",
                            Value::Null => "null",
                            Value::Bool(_) => "boolean",
                            Value::Int(..) | Value::Float(..) => "number",
                            Value::Str(_) => "string",
                            Value::Any(any) if any.tag() == zun_core::AnyTag::ClassDescriptor => {
                                "class"
                            }
                            _ => "value",
                        };
                        if what == "value" {
                            return Err(RunError::BadToken);
                        }
                        let message = if what == "class" {
                            "Class constructor cannot be invoked without 'new'".to_string()
                        } else {
                            format!("{what} is not a function")
                        };
                        match self.new_error_object("TypeError", &message) {
                            Some(error) => {
                                let exception = Value::Object(error);
                                if self.route_exception(
                                    &body,
                                    stack,
                                    pc,
                                    next - size,
                                    &exception,
                                ) {
                                    continue;
                                }
                                return Err(RunError::Thrown(exception));
                            }
                            None => return Err(RunError::Native(format!("TypeError: {message}"))),
                        }
                    };
                    if handle.generator_method().is_some() {
                        let result = self.invoke_function_value(handle, args)?;
                        stack.push(result);
                        continue;
                    }
                    if let Some((promise, mode)) = self.settler_of(handle) {
                        // A `Promise.withResolvers` settler: settle natively and
                        // push its `undefined` result (no method body to run).
                        let result = self.invoke_promise_settler(promise, mode, args)?;
                        stack.push(result);
                        continue;
                    }
                    if handle.is_bound() {
                        // A `bind`-produced bound function: resolve its target and
                        // prepended arguments, then dispatch through the shared
                        // interpreter path (routing exceptions like a direct call).
                        let (target_rid, call_args) = self.resolve_call_target(handle, args)?;
                        let link = RootLink {
                            stack: std::ptr::from_mut(stack.as_mut_slice()),
                            locals: std::ptr::from_mut(locals.as_mut_slice()),
                            parent,
                        };
                        let result = match self.dispatch_outcome(target_rid, call_args, Some(&link))
                        {
                            ExecutionOutcome::Complete(value) => value,
                            ExecutionOutcome::Thrown(exception) => {
                                if self.route_exception(
                                    &body,
                                    stack,
                                    pc,
                                    next - size,
                                    &exception,
                                ) {
                                    continue;
                                }
                                return Err(RunError::Thrown(exception));
                            }
                            ExecutionOutcome::Fault(error) => return Err(error),
                            ExecutionOutcome::Suspended(frame) => {
                                return Err(self.propagate_suspended(
                                    frame,
                                    ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                                ));
                            }
                        };
                        stack.push(result);
                        continue;
                    }
                    if !dynamic
                        && (handle.environment().is_some() || handle.method_rid() != token.rid())
                    {
                        return Err(RunError::BadToken);
                    }
                    let environment = handle
                        .environment()
                        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                    let target = self
                        .module
                        .tables
                        .method_defs
                        .get(handle.method_rid() as usize - 1)
                        .ok_or(RunError::BadToken)?;
                    let _target_body = read_body(&self.module.code, target.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    let hidden_environment = environment.is_some();
                    // A dynamic call adapts surplus/missing arguments to the
                    // callee's parameters in `call_inner` (JS semantics), so a
                    // count mismatch is not an error here.
                    let _ = hidden_environment;
                    let call_args = if let Some(environment) = environment {
                        let mut closure_args = Vec::with_capacity(args.len() + 1);
                        closure_args.push(Value::Object(environment));
                        closure_args.extend(args);
                        closure_args
                    } else {
                        args
                    };
                    // Calling a generator function through a function value creates
                    // the generator object instead of running the body.
                    if self.method_contains_yield(handle.method_rid()) {
                        let generator = self.create_generator(handle.method_rid(), &call_args)?;
                        stack.push(generator);
                        continue;
                    }
                    let link = RootLink {
                        stack: std::ptr::from_mut(stack.as_mut_slice()),
                        locals: std::ptr::from_mut(locals.as_mut_slice()),
                        parent,
                    };
                    let result =
                        match self.dispatch_outcome(handle.method_rid(), call_args, Some(&link)) {
                            ExecutionOutcome::Complete(value) => value,
                            ExecutionOutcome::Thrown(exception) => {
                                if self.route_exception(
                                    &body,
                                    stack,
                                    pc,
                                    next - size,
                                    &exception,
                                ) {
                                    continue;
                                }
                                return Err(RunError::Thrown(exception));
                            }
                            ExecutionOutcome::Fault(error) => return Err(error),
                            ExecutionOutcome::Suspended(frame) => {
                                return Err(self.propagate_suspended(
                                    frame,
                                    ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                                ));
                            }
                        };
                    stack.push(result);
                }
                Opcode::CallIndirectRest => {
                    match self.op_call_indirect_rest(
                        stack, pc, locals, &body, instr, next, size, method_rid, parent,
                    )? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::CallBuiltin => {
                    let id = operand_i32(&instr.operand);
                    match builtins::call_builtin(self, id, stack) {
                        Ok(()) => {}
                        Err(RunError::Native(message)) => {
                            // A native failure naming a prelude error class
                            // becomes a catchable thrown error object.
                            let exception = native_error::split_native_error(&message)
                                .and_then(|(class, text)| self.make_error_value(class, text));
                            let Some(exception) = exception else {
                                return Err(RunError::Native(message));
                            };
                            if self.route_exception(
                                &body,
                                stack,
                                pc,
                                next - size,
                                &exception,
                            ) {
                                continue;
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(RunError::Thrown(exception)) => {
                            // A user callback invoked by the native threw.
                            if self.route_exception(
                                &body,
                                stack,
                                pc,
                                next - size,
                                &exception,
                            ) {
                                continue;
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(error) => return Err(error),
                    }
                }
                Opcode::NewObj => {
                    // Safepoint: only the interpreter triggers GC, and only
                    // here, where every live object reference is reachable from
                    // the current frame plus the ancestor root chain (no JIT
                    // frame is executing at this point).
                    self.allocs += 1;
                    self.poll_gc(stack, locals, parent);
                    let token = operand_token(&instr.operand);
                    let (field_start, field_count) = self.type_field_range(token)?;
                    let args = pop_n(stack, field_count)?;
                    let kinds: Vec<u8> = (0..field_count as u32)
                        .map(|i| self.field_kind_at(field_start + i))
                        .collect::<Result<_, _>>()?;
                    let obj = zun_gc::with_current_heap_mut(|h| {
                        let obj = h.alloc(token.rid(), field_count);
                        for (i, arg) in args.iter().enumerate() {
                            value::store_field(kinds[i], obj, i, arg, h);
                        }
                        obj
                    });
                    stack.push(Value::Object(obj));
                }
                Opcode::LoadField => {
                    let token = operand_token(&instr.operand);
                    let field = self
                        .types
                        .get_field(token.rid())
                        .ok_or(RunError::BadToken)?;
                    let slot = field.slot;
                    let kind = field.kind;
                    let obj = pop(stack)?.as_object();
                    // A dynamic object never carries a class's fixed fields
                    // (a private-brand mismatch): TypeError, not a bad read.
                    let loaded = zun_gc::with_current_heap(|h| {
                        if h.is_dynamic_object(obj) {
                            None
                        } else {
                            Some(value::load_field(kind, obj, slot, h))
                        }
                    });
                    let Some(v) = loaded else {
                        throw_native!(self, body, stack, pc, next - size, "TypeError: Cannot read private member from an object whose class did not declare it".to_string());
                    };
                    stack.push(v);
                }
                Opcode::StoreField => {
                    let token = operand_token(&instr.operand);
                    let field = self
                        .types
                        .get_field(token.rid())
                        .ok_or(RunError::BadToken)?;
                    let slot = field.slot;
                    let kind = field.kind;
                    let value = pop(stack)?;
                    let obj = pop(stack)?.as_object();
                    zun_gc::with_current_heap_mut(|h| {
                        value::store_field(kind, obj, slot, &value, h);
                    });
                    stack.push(value);
                }
                Opcode::CallVirt => {
                    let token = operand_token(&instr.operand);
                    if token.table() != Some(TableKind::MethodDef) {
                        return Err(RunError::BadToken);
                    }
                    // The static method gives the call's name and arity.
                    let static_method = self
                        .module
                        .tables
                        .method_defs
                        .get(token.rid() as usize - 1)
                        .ok_or(RunError::BadToken)?;
                    let static_body = read_body(&self.module.code, static_method.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    let args = pop_n(stack, static_body.param_count as usize)?;
                    // Primitive-backed classes use static method binding: the
                    // receiver is still passed as `this`, but has no heap type
                    // to consult for virtual dispatch.
                    let primitive_receiver =
                        matches!(args.first(), Some(Value::Str(_) | Value::Float(_, _)))
                            || matches!(args.first(), Some(Value::Object(object))
                            if zun_gc::with_current_heap(|heap| heap.array_len(*object).is_some()));
                    let target = if primitive_receiver {
                        token.rid()
                    } else {
                        let type_rid = args[0].as_object().type_rid();
                        self.types
                            .resolve_virtual(type_rid, token.rid())
                            .ok_or(RunError::BadToken)?
                    };
                    let link = RootLink {
                        stack: std::ptr::from_mut(stack.as_mut_slice()),
                        locals: std::ptr::from_mut(locals.as_mut_slice()),
                        parent,
                    };
                    let result = match self.dispatch_outcome(target, args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                stack,
                                pc,
                                next - size,
                                &exception,
                            ) {
                                continue;
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        ExecutionOutcome::Fault(error) => return Err(error),
                        ExecutionOutcome::Suspended(frame) => {
                            return Err(self.propagate_suspended(
                                frame,
                                ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::CallVirtRest => {
                    match self.op_call_virt_rest(
                        stack, pc, locals, &body, instr, next, size, method_rid, parent,
                    )? {
                        ArmFlow::Next => {}
                        ArmFlow::Continue => continue,
                    }
                }
                Opcode::CallVirtRestArray | Opcode::CallVirtRestArrayFixed => {
                    let (token, fixed_count) = if instr.opcode == Opcode::CallVirtRestArray {
                        (operand_token(&instr.operand), 0)
                    } else {
                        let (token, count) = operand_token_imm32(&instr.operand);
                        if count < 0 {
                            return Err(RunError::BadToken);
                        }
                        (token, count as usize)
                    };
                    if token.table() != Some(TableKind::MethodDef) {
                        return Err(RunError::BadToken);
                    }
                    let static_method = self
                        .module
                        .tables
                        .method_defs
                        .get(token.rid() as usize - 1)
                        .ok_or(RunError::BadToken)?;
                    let static_body = read_body(&self.module.code, static_method.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    if static_body.param_count != fixed_count as u16 + 2 {
                        return Err(RunError::BadToken);
                    }
                    let mut values = pop_n(stack, fixed_count + 2)?;
                    let rest_array = values.pop().ok_or(RunError::BadToken)?.as_object();
                    let receiver = values.first().ok_or(RunError::BadToken)?;
                    let primitive_receiver = matches!(receiver, Value::Str(_) | Value::Float(_, _))
                        || matches!(receiver, Value::Object(object)
                            if zun_gc::with_current_heap(|heap| heap.array_len(*object).is_some()));
                    let target = if primitive_receiver {
                        token.rid()
                    } else {
                        self.types
                            .resolve_virtual(receiver.as_object().type_rid(), token.rid())
                            .ok_or(RunError::BadToken)?
                    };
                    values.push(Value::Object(rest_array));
                    let link = RootLink {
                        stack: std::ptr::from_mut(stack.as_mut_slice()),
                        locals: std::ptr::from_mut(locals.as_mut_slice()),
                        parent,
                    };
                    let result = match self.dispatch_outcome(target, values, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                stack,
                                pc,
                                next - size,
                                &exception,
                            ) {
                                continue;
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        ExecutionOutcome::Fault(error) => return Err(error),
                        ExecutionOutcome::Suspended(frame) => {
                            return Err(self.propagate_suspended(
                                frame,
                                ExecutionFrame::snapshot(method_rid, *pc, stack, locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::Await => {
                    let promise = pop(stack)?.as_object();
                    // Awaiting a promise handles it — clear any
                    // unhandled-rejection tracking even on the synchronous
                    // already-settled fast path below.
                    self.notice_promise(promise);
                    match settled_await_value(promise) {
                        Ok(Some(value)) => stack.push(value),
                        Ok(None) => {
                            let await_pc = (*pc).saturating_sub(size);
                            let mut suspended = ExecutionFrame::new(method_rid, locals.to_vec());
                            suspended.stack = stack.to_vec();
                            suspended.stack.push(Value::Object(promise));
                            suspended.pc = await_pc;
                            suspended.suspend(promise, await_pc);
                            self.suspended = Some(suspended);
                            return Err(RunError::Suspended);
                        }
                        // A rejected await resumes through the unwinding path:
                        // route the rejection into this method's handler table
                        // (finally / catch) exactly as `throw` does, rather than
                        // escaping the body unconditionally.
                        Err(RunError::Thrown(exception)) => {
                            if self.route_exception(
                                &body,
                                stack,
                                pc,
                                next - size,
                                &exception,
                            ) {
                                continue;
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(error) => return Err(error),
                    }
                }
                Opcode::Yield => {
                    let value = pop(stack)?;
                    self.yielded = Some(value);
                    self.suspended =
                        Some(ExecutionFrame::snapshot(method_rid, *pc, stack, locals));
                    return Err(RunError::Suspended);
                }
                Opcode::ImportDynamic => match self.op_import_dynamic(stack)? {
                    ArmFlow::Next => {}
                    ArmFlow::Continue => continue,
                },
                // Remaining opcodes are not produced by the front-end yet.
                Opcode::ConstNull => {
                    stack.push(Value::Null);
                }
            }
        }
        // Falling off the end with no `ret` returns void; ilgen always appends
        // a trailing `ret`, so this is a safety net.
        Ok(Value::Void)
    }

    /// Run a mark-sweep collection with precise roots: the object references in
    /// the current frame's stack and locals, plus every ancestor frame in the
    /// `parent` chain. Reference fields are traced via the module's type layout.
    fn collect(
        &mut self,
        stack: &mut [Value],
        locals: &mut [Value],
        parent: Option<&RootLink>,
        full: bool,
    ) -> usize {
        let mut extra_roots = Vec::new();
        // Class-static fields now live on the heap's statics object (a persistent
        // GC root), so the heap marks and rewrites them; no interpreter-side root
        // handling is needed here.
        if let Some(hook) = self.hook {
            hook.append_roots(&mut extra_roots);
        }
        // Pending `await` frames, scheduled reactions and combinator aggregates
        // are live across a collection that happens while a reaction callback or
        // a resumed frame is executing: root them here so a promise still owed a
        // reaction is not reclaimed, and rewrite them when the heap moves.
        self.append_parked_roots(&mut extra_roots);
        if full && parent.is_none() {
            let hook = self.hook;
            roots::collect_and_compact_current(
                &self.types,
                stack,
                locals,
                &extra_roots,
                &mut |relocation| {
                    decorators::rewrite_decorator_fields(
                        &mut self.decorator_initializers,
                        &mut self.class_named_methods,
                        &mut self.instance_initializers,
                        &mut self.field_transforms,
                        relocation,
                    );
                    self.async_state.rewrite_after_move(relocation);
                    if let Some(hook) = hook {
                        hook.rewrite_managed_roots(relocation);
                    }
                },
            )
        } else if full {
            roots::collect(&self.types, stack, locals, parent, &extra_roots)
        } else {
            let hook = self.hook;
            roots::collect_young_evacuating(
                &self.types,
                stack,
                locals,
                parent,
                &extra_roots,
                &mut |relocation| {
                    decorators::rewrite_decorator_fields(
                        &mut self.decorator_initializers,
                        &mut self.class_named_methods,
                        &mut self.instance_initializers,
                        &mut self.field_transforms,
                        relocation,
                    );
                    self.async_state.rewrite_after_move(relocation);
                    if let Some(hook) = hook {
                        hook.rewrite_managed_roots(relocation);
                    }
                },
            )
        }
    }

    /// Poll the adaptive GC budget at an interpreter safepoint.
    ///
    /// Backedges and allocation sites are both safe because the current frame
    /// and its caller chain are fully materialized for precise root scanning.
    fn poll_gc(&mut self, stack: &mut [Value], locals: &mut [Value], parent: Option<&RootLink>) {
        if self.allocs < self.gc_interval {
            return;
        }
        self.allocs = 0;
        let young_before = zun_gc::with_current_heap(|heap| heap.generation_counts().0);
        let full = self.minor_collections >= 7;
        let reclaimed = self.collect(stack, locals, parent, full);
        if full {
            self.minor_collections = 0;
        } else {
            self.minor_collections = self.minor_collections.saturating_add(1);
        }
        self.gc_interval = next_gc_interval(self.gc_interval, reclaimed, young_before);
    }
}

#[cfg(test)]
mod method_tests;

/// Resolve the expando property bag of a function object, creating it on
/// first use. Bags are dynamic objects rooted in the statics object under a
/// hidden `__fnbag:<method>:<env>` key, so `fn['x'] = v` on a function value
/// behaves like a property write on a JS function object.
pub(crate) fn function_expando_bag(handle: zun_core::FunctionHandle) -> zun_gc::Obj {
    let key = format!(
        "__fnbag:{}:{}:{}",
        handle.method_rid(),
        handle.is_bound(),
        handle.environment().unwrap_or(0)
    );
    zun_gc::with_current_heap_mut(|heap| {
        let statics = heap.statics_object();
        if let Some(any) = heap.dynamic_get(statics, &key) {
            if let Some(Value::Object(obj)) = Value::unbox_any(any, heap) {
                return obj;
            }
        }
        let bag = heap.alloc_dynamic();
        if let Some(any) = Value::Object(bag).box_any(heap) {
            heap.dynamic_set(statics, key, any);
        }
        bag
    })
}

/// Map a function-valued receiver to its expando bag; other values pass through.
pub(crate) fn expando_receiver(value: Value) -> Value {
    match value {
        Value::Function(handle) => Value::Object(function_expando_bag(handle)),
        other => other,
    }
}
