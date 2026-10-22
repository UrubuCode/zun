//! Tier 0: the ZIL interpreter.
//!
//! A straightforward stack-machine loop over a method body's ZIL. It is the
//! always-correct baseline the JIT must match (the equivalence suite in a later
//! phase runs programs through both). Calls recurse through [`Interp::call`];
//! the `print` builtin writes through a pluggable [`Output`] sink so tests can
//! capture it.

mod builtins;
mod collections;
mod errors;
mod gc_policy;
mod hooks;
mod native;
mod metadata;
mod output;
mod operands;
mod roots;
mod runner;
mod stack;
mod state;
mod unwind;
pub mod value;

use gc_policy::next_gc_interval;
use operands::{
    branch_target, operand_f64, operand_i32, operand_i64, operand_local, operand_strref,
    operand_token, operand_token_imm32,
};
use stack::{
    bin_float, bin_int, bin_int_signed_aware, cmp, compare, convert, pop, pop_n, underflow,
    Ordering,
};
use state::{ExecutionFrame, ExecutionOutcome, Interp, RootLink};
use runner::run_method_with;
pub use runner::{
    GeneratorIteratorResult, GeneratorResult, GeneratorSession, OwnedGeneratorSession,
    ResumableSession, ResumeResult,
};

/// Read the result of a settled managed Promise for the synchronous await
/// fast path. Pending promises deliberately return `None` and must be routed
/// through the future continuation/state-machine path.
fn settled_await_value(promise: zun_gc::Obj) -> Result<Option<Value>, RunError> {
    zun_gc::with_current_heap(|heap| {
        match heap.promise_state(promise) {
            Some(zun_core::ManagedPromiseState::Pending) => Ok(None),
            Some(zun_core::ManagedPromiseState::Resolved) => Ok(heap
                .promise_result(promise)
                .and_then(|value| value::Value::unbox_any(value, heap))),
            Some(zun_core::ManagedPromiseState::Rejected) => Err(RunError::Thrown(
                Value::Str(Rc::from(
                    heap.promise_error(promise)
                        .unwrap_or_else(|| "Promise rejected".into()),
                )),
            )),
            None => Err(RunError::BadToken),
        }
    })
}
use std::rc::Rc;
use value::Value;
use zun_il::{decode_one, Opcode, Operand};
use zun_metadata::TableKind;
use zun_module::{read_body, Module};

/// Sink for program output: the native modules' [`Sink`](zun_natives::Sink),
/// re-exported so callers keep using `Output`.
pub use zun_natives::Sink as Output;
pub use output::{CaptureOutput, StdoutOutput};
pub use hooks::CallHook;
pub use errors::RunError;

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
    run_method_with(module, entry.rid(), Vec::new(), output, hook, None)
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
    run_method_with(module, method_rid, args, output, None, None)
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

/// Execute a method from a loaded module set, allowing top-level MemberRef calls.
pub fn run_method_loaded(
    scope: &zun_loader::ModuleScope,
    loader: &zun_loader::ModuleLoader,
    method_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
) -> Result<Value, RunError> {
    run_method_with(scope.module(), method_rid, args, output, None, Some(loader))
}

impl Interp<'_> {
    /// Create the managed identity of a generator invocation. The execution
    /// body is deliberately not entered here; `generatorIteratorNext` owns
    /// resume and stores the suspended state after the first yield.
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
            args.iter().map(|value| value.box_any(heap)).collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_generator(0, method_rid, boxed));
        Ok(Value::Object(object))
    }

    /// Call the method at `method_rid` (1-based) with `args`, returning its
    /// result ([`Value::Void`] for a void method).
    fn call(
        &mut self,
        method_rid: u32,
        args: Vec<Value>,
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

        let mut frame = if let Some(frame) = self.resume_frame.take() {
            if frame.method_rid != method_rid {
                return Err(RunError::Malformed(
                    "resumed frame method does not match dispatch target".into(),
                ));
            }
            frame
        } else {
            let mut locals: Vec<Value> = Vec::with_capacity(body.local_count as usize);
            locals.extend(args);
            locals.resize(body.local_count as usize, Value::Void);
            ExecutionFrame::new(method_rid, locals)
        };
        let mut stack = &mut frame.stack;
        let locals = &mut frame.locals;
        let code = body.code;
        let mut pc = &mut frame.pc;

        while *pc < code.len() {
            let (instr, size) =
                decode_one(&code[*pc..]).map_err(|e| RunError::Malformed(e.to_string()))?;
            let next = *pc + size;
            *pc = next;

            match instr.opcode {
                Opcode::Nop => {}
                Opcode::LoadStatic => {
                    let key = self.static_field_key(operand_token(&instr.operand))?;
                    stack.push(self.static_fields.entry(key).or_insert(Value::Void).clone());
                }
                Opcode::StoreStatic => {
                    let key = self.static_field_key(operand_token(&instr.operand))?;
                    let value = pop(&mut stack)?;
                    self.static_fields.insert(key, value.clone());
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
                    let environment = pop(&mut stack)?;
                    let Value::Object(environment) = environment else {
                        return Err(RunError::BadToken);
                    };
                    stack.push(Value::Function(zun_core::FunctionHandle::new(
                        rid,
                        Some(environment.addr() as usize),
                    )));
                }
                Opcode::BoxAny => {
                    let value = pop(&mut stack)?;
                    let any = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    stack.push(Value::Any(any));
                }
                Opcode::UnboxAny => {
                    let value = pop(&mut stack)?;
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
                    let token = match instr.operand {
                        Operand::Token(token)
                            if token.table() == Some(zun_metadata::TableKind::TypeDef) =>
                        {
                            token
                        }
                        _ => return Err(RunError::BadToken),
                    };
                    let value = pop(&mut stack)?;
                    let value = if let Value::Any(any) = value {
                        zun_gc::with_current_heap(|heap| value::Value::unbox_any(any, heap))
                            .ok_or(RunError::BadToken)?
                    } else {
                        value
                    };
                    match value {
                        Value::Null => stack.push(Value::Null),
                        Value::Object(object)
                            if self.types.is_assignable(object.type_rid(), token.rid()) =>
                        {
                            stack.push(Value::Object(object));
                        }
                        _ => return Err(RunError::BadToken),
                    }
                }
                Opcode::NewDynamic => {
                    let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                    stack.push(Value::Object(object));
                }
                Opcode::LoadDynamic => {
                    let name = self.dynamic_name(&instr.operand)?;
                    let object = pop(&mut stack)?.as_object();
                    let value = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get(object, name)
                            .map(Value::Any)
                            .unwrap_or(Value::Void)
                    });
                    stack.push(value);
                }
                Opcode::StoreDynamic => {
                    let name = self.dynamic_name(&instr.operand)?.to_owned();
                    let value = pop(&mut stack)?;
                    let Value::Any(any) = value else {
                        return Err(RunError::BadToken);
                    };
                    let object = pop(&mut stack)?.as_object();
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(object, name, any);
                    });
                    stack.push(Value::Any(any));
                }
                Opcode::LoadDynamicValue => {
                    let key = match pop(&mut stack)? {
                        Value::Str(key) => key,
                        _ => return Err(RunError::BadToken),
                    };
                    let object = pop(&mut stack)?.as_object();
                    let value = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get(object, &key)
                            .map(Value::Any)
                            .unwrap_or(Value::Void)
                    });
                    stack.push(value);
                }
                Opcode::StoreDynamicValue => {
                    let value = pop(&mut stack)?;
                    let Value::Any(any) = value else {
                        return Err(RunError::BadToken);
                    };
                    let key = match pop(&mut stack)? {
                        Value::Str(key) => key,
                        _ => return Err(RunError::BadToken),
                    };
                    let object = pop(&mut stack)?.as_object();
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(object, key.as_ref().to_owned(), any);
                    });
                    stack.push(Value::Any(any));
                }
                Opcode::NewArray => {
                    let length = pop(&mut stack)?.as_int();
                    if length < 0 {
                        return Err(RunError::Malformed("negative array length".into()));
                    }
                    let array =
                        zun_gc::with_current_heap_mut(|heap| heap.alloc_array(length as usize));
                    stack.push(Value::Object(array));
                }
                Opcode::LoadArray => {
                    let index = pop(&mut stack)?.as_int();
                    let array = pop(&mut stack)?.as_object();
                    if index < 0 {
                        return Err(RunError::Malformed("negative array index".into()));
                    }
                    let value = zun_gc::with_current_heap(|heap| {
                        heap.array_get(array, index as usize)
                            .and_then(|any| value::Value::unbox_any(any, heap))
                    })
                    .ok_or(RunError::BadToken)?;
                    stack.push(value);
                }
                Opcode::StoreArray => {
                    let value = pop(&mut stack)?;
                    let index = pop(&mut stack)?.as_int();
                    let array = pop(&mut stack)?.as_object();
                    if index < 0 {
                        return Err(RunError::Malformed("negative array index".into()));
                    }
                    let any = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.array_set(array, index as usize, any)
                    })
                    .ok_or(RunError::BadToken)?;
                    stack.push(value);
                }
                Opcode::ArrayLength => {
                    let array = pop(&mut stack)?.as_object();
                    let length = zun_gc::with_current_heap(|heap| heap.array_len(array))
                        .ok_or(RunError::BadToken)?;
                    stack.push(Value::Int(length as i64, zun_il::NumKind::U64));
                }
                Opcode::ArrayPop => {
                    let array = pop(&mut stack)?.as_object();
                    let value = zun_gc::with_current_heap_mut(|heap| {
                        heap.array_pop(array)
                            .and_then(|any| value::Value::unbox_any(any, heap))
                    })
                    .ok_or(RunError::BadToken)?;
                    stack.push(value);
                }
                Opcode::ArrayPush => {
                    let value = pop(&mut stack)?;
                    let array = pop(&mut stack)?.as_object();
                    let any = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    let length = zun_gc::with_current_heap_mut(|heap| heap.array_push(array, any))
                        .ok_or(RunError::BadToken)?;
                    stack.push(Value::Int(length as i64, zun_il::NumKind::U64));
                }
                Opcode::LoadLocal => stack.push(locals[operand_local(&instr.operand)].clone()),
                Opcode::StoreLocal => {
                    let v = pop(&mut stack)?;
                    locals[operand_local(&instr.operand)] = v;
                }
                Opcode::Pop => {
                    pop(&mut stack)?;
                }
                Opcode::Dup => {
                    let top = stack.last().cloned().ok_or_else(underflow)?;
                    stack.push(top);
                }
                Opcode::AddF64 => bin_float(&mut stack, |a, b| a + b)?,
                Opcode::SubF64 => bin_float(&mut stack, |a, b| a - b)?,
                Opcode::MulF64 => bin_float(&mut stack, |a, b| a * b)?,
                Opcode::DivF64 => bin_float(&mut stack, |a, b| a / b)?,
                Opcode::RemF64 => bin_float(&mut stack, |a, b| a % b)?,
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
                Opcode::AddI32 => bin_int(&mut stack, |a, b| a.wrapping_add(b))?,
                Opcode::SubI32 => bin_int(&mut stack, |a, b| a.wrapping_sub(b))?,
                Opcode::MulI32 => bin_int(&mut stack, |a, b| a.wrapping_mul(b))?,
                Opcode::DivI32 => bin_int_signed_aware(&mut stack, true)?,
                Opcode::RemI32 => bin_int_signed_aware(&mut stack, false)?,
                Opcode::NegI32 => {
                    let v = pop(&mut stack)?;
                    let k = v.int_kind();
                    stack.push(Value::Int(value::wrap_int(v.as_int().wrapping_neg(), k), k));
                }
                Opcode::Conv => {
                    let target = zun_il::NumKind::from_imm(operand_i32(&instr.operand))
                        .ok_or_else(|| RunError::Malformed("bad conv kind".into()))?;
                    let v = pop(&mut stack)?;
                    stack.push(convert(&v, target)?);
                }
                Opcode::Concat => {
                    let b = pop(&mut stack)?;
                    let a = pop(&mut stack)?;
                    stack.push(Value::Str(format!("{}{}", a.as_str(), b.as_str()).into()));
                }
                Opcode::CmpEq => cmp(&mut stack, true)?,
                Opcode::CmpNe => cmp(&mut stack, false)?,
                Opcode::CmpLt => compare(&mut stack, Ordering::Lt)?,
                Opcode::CmpLe => compare(&mut stack, Ordering::Le)?,
                Opcode::CmpGt => compare(&mut stack, Ordering::Gt)?,
                Opcode::CmpGe => compare(&mut stack, Ordering::Ge)?,
                Opcode::Branch => {
                    let target = branch_target(next, &instr.operand);
                    if target < *pc {
                        if let Some(hook) = self.hook {
                            hook.on_backedge_state(method_rid, target, *pc, &stack, &locals);
                        }
                        self.poll_gc(&mut stack, locals, parent);
                    }
                    *pc = target;
                }
                Opcode::BranchTrue => {
                    if pop(&mut stack)?.as_bool() {
                        let target = branch_target(next, &instr.operand);
                        if target < *pc {
                            if let Some(hook) = self.hook {
                                hook.on_backedge_state(method_rid, target, *pc, &stack, &locals);
                            }
                            self.poll_gc(&mut stack, locals, parent);
                        }
                        *pc = target;
                    }
                }
                Opcode::BranchFalse => {
                    if !pop(&mut stack)?.as_bool() {
                        let target = branch_target(next, &instr.operand);
                        if target < *pc {
                            if let Some(hook) = self.hook {
                                hook.on_backedge_state(method_rid, target, *pc, &stack, &locals);
                            }
                            self.poll_gc(&mut stack, locals, parent);
                        }
                        *pc = target;
                    }
                }
                Opcode::Ret => {
                    return Ok(stack.pop().unwrap_or(Value::Void));
                }
                Opcode::Throw => {
                    let exception = pop(&mut stack)?;
                    let throw_pc = *pc - size;
                    let handler = body
                        .handlers
                        .iter()
                        .filter(|handler| {
                            handler.try_start as usize <= throw_pc
                                && throw_pc < handler.try_end as usize
                        })
                        .filter(|handler| match handler.kind {
                            zun_module::HandlerKind::Finally => true,
                            zun_module::HandlerKind::Catch => {
                                match (handler.catch_type, &exception) {
                                    (Some(token), Value::Object(object)) => {
                                        self.types.is_assignable(object.type_rid(), token.rid())
                                    }
                                    _ => false,
                                }
                            }
                        })
                        .min_by_key(|handler| handler.try_end - handler.try_start);
                    if let Some(handler) = handler {
                        stack.clear();
                        stack.push(exception);
                        *pc = handler.handler_start as usize;
                    } else {
                        return Err(RunError::Thrown(exception));
                    }
                }
                Opcode::Rethrow => {
                    let exception = pop(&mut stack)?;
                    if self.route_exception(&body, &mut stack, &mut pc, next - size, &exception) {
                        continue;
                    }
                    return Err(RunError::Thrown(exception));
                }
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
                        let args = pop_n(&mut stack, body.param_count as usize)?;
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
                                    &mut stack,
                                    &mut pc,
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
                    let args = pop_n(&mut stack, callee_body.param_count as usize)?;
                    let link = RootLink {
                        stack: &stack,
                        locals: &locals,
                        parent,
                    };
                    let result = match self.dispatch_outcome(callee_rid, args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                &mut stack,
                                &mut pc,
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
                                ExecutionFrame::snapshot(method_rid, *pc, &stack, &locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::CallRest => {
                    let (token, argument_count) = operand_token_imm32(&instr.operand);
                    if token.table() != Some(TableKind::MethodDef) || argument_count < 0 {
                        return Err(RunError::BadToken);
                    }
                    let callee_rid = token.rid();
                    let callee = self
                        .module
                        .tables
                        .method_defs
                        .get(callee_rid as usize - 1)
                        .ok_or(RunError::BadToken)?;
                    let callee_body = read_body(&self.module.code, callee.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    let rest_index = callee_body
                        .param_count
                        .checked_sub(1)
                        .ok_or(RunError::BadToken)?;
                    let argument_count = argument_count as usize;
                    if argument_count < rest_index as usize {
                        return Err(RunError::BadToken);
                    }
                    let values = pop_n(&mut stack, argument_count)?;
                    let rest_values = &values[rest_index as usize..];
                    let rest_array =
                        zun_gc::with_current_heap_mut(|heap| -> Result<_, RunError> {
                            let array = heap.alloc_array(rest_values.len());
                            for (index, value) in rest_values.iter().enumerate() {
                                let any = value.box_any(heap).ok_or(RunError::BadToken)?;
                                let _ = heap.array_set(array, index, any);
                            }
                            Ok(array)
                        })?;
                    let mut args = values[..rest_index as usize].to_vec();
                    args.push(Value::Object(rest_array));
                    let link = RootLink {
                        stack: &stack,
                        locals: &locals,
                        parent,
                    };
                    let result = match self.dispatch_outcome(callee_rid, args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                &mut stack,
                                &mut pc,
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
                                ExecutionFrame::snapshot(method_rid, *pc, &stack, &locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::CallRestArray => {
                    let token = operand_token(&instr.operand);
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
                    let callee_body = read_body(&self.module.code, callee.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    if callee_body.param_count != 1 {
                        return Err(RunError::BadToken);
                    }
                    let array = pop(&mut stack)?.as_object();
                    let args = vec![Value::Object(array)];
                    let link = RootLink {
                        stack: &stack,
                        locals: &locals,
                        parent,
                    };
                    let result = match self.dispatch_outcome(callee_rid, args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                &mut stack,
                                &mut pc,
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
                                ExecutionFrame::snapshot(method_rid, *pc, &stack, &locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::CallRestArrayFixed => {
                    let (token, fixed_count) = operand_token_imm32(&instr.operand);
                    if token.table() != Some(TableKind::MethodDef) || fixed_count < 0 {
                        return Err(RunError::BadToken);
                    }
                    let callee_rid = token.rid();
                    let callee = self
                        .module
                        .tables
                        .method_defs
                        .get(callee_rid as usize - 1)
                        .ok_or(RunError::BadToken)?;
                    let callee_body = read_body(&self.module.code, callee.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    let fixed_count = fixed_count as usize;
                    if callee_body.param_count != fixed_count as u16 + 1 {
                        return Err(RunError::BadToken);
                    }
                    let mut args = pop_n(&mut stack, fixed_count + 1)?;
                    let array = args.pop().ok_or(RunError::BadToken)?.as_object();
                    args.push(Value::Object(array));
                    let link = RootLink {
                        stack: &stack,
                        locals: &locals,
                        parent,
                    };
                    let result = match self.dispatch_outcome(callee_rid, args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                &mut stack,
                                &mut pc,
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
                                ExecutionFrame::snapshot(method_rid, *pc, &stack, &locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::CallIndirect => {
                    let token = operand_token(&instr.operand);
                    if !matches!(
                        token.table(),
                        Some(TableKind::MethodDef | TableKind::MemberRef)
                    ) {
                        return Err(RunError::BadToken);
                    }
                    let dynamic = token.rid() == 0;
                    let reducer = dynamic && token.table() == Some(TableKind::MemberRef);
                    let expected_params = if dynamic {
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
                    let args = pop_n(&mut stack, expected_params)?;
                    let function = pop(&mut stack)?;
                    let Value::Function(handle) = function else {
                        return Err(RunError::BadToken);
                    };
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
                    let target_body = read_body(&self.module.code, target.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    let hidden_environment = environment.is_some();
                    if dynamic
                        && target_body.param_count
                            != (expected_params + if hidden_environment { 1 } else { 0 }) as u16
                    {
                        return Err(RunError::BadToken);
                    }
                    let call_args = if let Some(environment) = environment {
                        let mut closure_args = Vec::with_capacity(args.len() + 1);
                        closure_args.push(Value::Object(environment));
                        closure_args.extend(args);
                        closure_args
                    } else {
                        args
                    };
                    let link = RootLink {
                        stack: &stack,
                        locals: &locals,
                        parent,
                    };
                    let result = match self.dispatch_outcome(handle.method_rid(), call_args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                &mut stack,
                                &mut pc,
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
                                ExecutionFrame::snapshot(method_rid, *pc, &stack, &locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::CallIndirectRest => {
                    let (token, argument_count) = operand_token_imm32(&instr.operand);
                    if token.table() != Some(TableKind::MethodDef) || argument_count < 0 {
                        return Err(RunError::BadToken);
                    }
                    let argument_count = argument_count as usize;
                    let values = pop_n(&mut stack, argument_count)?;
                    let function = pop(&mut stack)?;
                    let Value::Function(handle) = function else {
                        return Err(RunError::BadToken);
                    };
                    if handle.method_rid() != token.rid() {
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
                    let target_body = read_body(&self.module.code, target.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    let hidden = usize::from(environment.is_some());
                    let rest_index = target_body
                        .param_count
                        .checked_sub((hidden + 1) as u16)
                        .ok_or(RunError::BadToken)? as usize;
                    if argument_count < rest_index {
                        return Err(RunError::BadToken);
                    }
                    let rest_values = &values[rest_index..];
                    let rest_array = zun_gc::with_current_heap_mut(|heap| {
                        let array = heap.alloc_array(rest_values.len());
                        for (index, value) in rest_values.iter().enumerate() {
                            let any = value.box_any(heap).ok_or(RunError::BadToken)?;
                            let _ = heap.array_set(array, index, any);
                        }
                        Ok::<_, RunError>(array)
                    })?;
                    let mut call_args = Vec::with_capacity(hidden + rest_index + 1);
                    if let Some(environment) = environment {
                        call_args.push(Value::Object(environment));
                    }
                    call_args.extend(values[..rest_index].iter().cloned());
                    call_args.push(Value::Object(rest_array));
                    let link = RootLink {
                        stack: &stack,
                        locals: &locals,
                        parent,
                    };
                    let result = match self.dispatch_outcome(handle.method_rid(), call_args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                &mut stack,
                                &mut pc,
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
                                ExecutionFrame::snapshot(method_rid, *pc, &stack, &locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::CallBuiltin => {
                    let id = operand_i32(&instr.operand);
                    builtins::call_builtin(self, id, &mut stack)?;
                }
                Opcode::NewObj => {
                    // Safepoint: only the interpreter triggers GC, and only
                    // here, where every live object reference is reachable from
                    // the current frame plus the ancestor root chain (no JIT
                    // frame is executing at this point).
                    self.allocs += 1;
                    self.poll_gc(&mut stack, locals, parent);
                    let token = operand_token(&instr.operand);
                    let (field_start, field_count) = self.type_field_range(token)?;
                    let args = pop_n(&mut stack, field_count)?;
                    let kinds: Vec<u8> = (0..field_count as u32)
                        .map(|i| self.field_kind_at(field_start + i))
                        .collect::<Result<_, _>>()?;
                    let obj = zun_gc::with_current_heap_mut(|h| {
                        let obj = h.alloc(token.rid(), field_count);
                        for (i, arg) in args.iter().enumerate() {
                            let raw = value::encode_field(kinds[i], arg, h);
                            h.store_field(obj, i, raw, kinds[i] == value::FIELD_OBJECT);
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
                    let obj = pop(&mut stack)?.as_object();
                    let v = zun_gc::with_current_heap(|h| {
                        value::decode_field(kind, obj.field(slot), h)
                    });
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
                    let value = pop(&mut stack)?;
                    let obj = pop(&mut stack)?.as_object();
                    zun_gc::with_current_heap_mut(|h| {
                        let raw = value::encode_field(kind, &value, h);
                        h.store_field(obj, slot, raw, kind == value::FIELD_OBJECT);
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
                    let args = pop_n(&mut stack, static_body.param_count as usize)?;
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
                        stack: &stack,
                        locals: &locals,
                        parent,
                    };
                    let result = match self.dispatch_outcome(target, args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                &mut stack,
                                &mut pc,
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
                                ExecutionFrame::snapshot(method_rid, *pc, &stack, &locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::CallVirtRest => {
                    let (token, argument_count) = operand_token_imm32(&instr.operand);
                    if token.table() != Some(TableKind::MethodDef) || argument_count < 0 {
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
                    let rest_index = static_body
                        .param_count
                        .checked_sub(1)
                        .ok_or(RunError::BadToken)? as usize;
                    let argument_count = argument_count as usize;
                    if argument_count < rest_index {
                        return Err(RunError::BadToken);
                    }
                    let values = pop_n(&mut stack, argument_count)?;
                    let primitive_receiver = matches!(
                        values.first(),
                        Some(Value::Str(_) | Value::Float(_, _))
                    ) || matches!(
                        values.first(),
                        Some(Value::Object(object))
                            if zun_gc::with_current_heap(|heap| heap.array_len(*object).is_some())
                    );
                    let target = if primitive_receiver {
                        token.rid()
                    } else {
                        let type_rid = values
                            .first()
                            .ok_or(RunError::BadToken)?
                            .as_object()
                            .type_rid();
                        self.types
                            .resolve_virtual(type_rid, token.rid())
                            .ok_or(RunError::BadToken)?
                    };
                    let rest_values = &values[rest_index..];
                    let rest_array =
                        zun_gc::with_current_heap_mut(|heap| -> Result<_, RunError> {
                            let array = heap.alloc_array(rest_values.len());
                            for (index, value) in rest_values.iter().enumerate() {
                                let any = value.box_any(heap).ok_or(RunError::BadToken)?;
                                let _ = heap.array_set(array, index, any);
                            }
                            Ok(array)
                        })?;
                    let mut args = values[..rest_index].to_vec();
                    args.push(Value::Object(rest_array));
                    let link = RootLink {
                        stack: &stack,
                        locals: &locals,
                        parent,
                    };
                    let result = match self.dispatch_outcome(target, args, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                &mut stack,
                                &mut pc,
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
                                ExecutionFrame::snapshot(method_rid, *pc, &stack, &locals),
                            ));
                        }
                    };
                    stack.push(result);
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
                    let mut values = pop_n(&mut stack, fixed_count + 2)?;
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
                        stack: &stack,
                        locals: &locals,
                        parent,
                    };
                    let result = match self.dispatch_outcome(target, values, Some(&link)) {
                        ExecutionOutcome::Complete(value) => value,
                        ExecutionOutcome::Thrown(exception) => {
                            if self.route_exception(
                                &body,
                                &mut stack,
                                &mut pc,
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
                                ExecutionFrame::snapshot(method_rid, *pc, &stack, &locals),
                            ));
                        }
                    };
                    stack.push(result);
                }
                Opcode::Await => {
                    let promise = pop(&mut stack)?.as_object();
                    let settled = settled_await_value(promise)?;
                    let Some(value) = settled else {
                        let await_pc = (*pc).saturating_sub(size);
                        let mut suspended = ExecutionFrame::new(method_rid, locals.to_vec());
                        suspended.stack = stack.to_vec();
                        suspended.stack.push(Value::Object(promise));
                        suspended.pc = await_pc;
                        suspended.suspend(promise, await_pc);
                        self.suspended = Some(suspended);
                        return Err(RunError::Suspended);
                    };
                    stack.push(value);
                }
                Opcode::Yield => {
                    let value = pop(&mut stack)?;
                    self.yielded = Some(value);
                    self.suspended = Some(ExecutionFrame::snapshot(
                        method_rid,
                        *pc,
                        &stack,
                        &locals,
                    ));
                    return Err(RunError::Suspended);
                }
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
    fn collect(&mut self, stack: &mut [Value], locals: &mut [Value], parent: Option<&RootLink>, full: bool) -> usize {
        let mut extra_roots = Vec::new();
        let static_values: Vec<Value> = self.static_fields.values().cloned().collect();
        roots::collect_frame_roots(&static_values, &[], &mut extra_roots);
        if let Some(hook) = self.hook {
            hook.append_roots(&mut extra_roots);
        }
        if full && parent.is_none() {
            let hook = self.hook;
            roots::collect_and_compact_current(
                &self.types,
                stack,
                locals,
                &extra_roots,
                &mut |relocation| {
                    for value in self.static_fields.values_mut() {
                        Value::rewrite_managed_references(std::slice::from_mut(value), relocation);
                    }
                    if let Some(hook) = hook {
                        hook.rewrite_managed_roots(relocation);
                    }
                },
            )
        } else if full {
            roots::collect(&self.types, stack, locals, parent, &extra_roots)
        } else {
            roots::collect_young(&self.types, stack, locals, parent, &extra_roots)
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
mod method_tests {
    use super::*;
    use crate::roots::{collect as collect_roots, collect_frame_roots};

    #[test]
    fn full_interpreter_gc_compacts_and_rewrites_current_locals() {
        let module = zun_compiler::compile(
            "class Box { value: int; } function main(): int { let live = new Box(7); let i: u64 = 0; while (i < 2300) { let garbage = new Box(1); i = i + 1; } return live.value; }",
        )
        .expect("compile compaction workload");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run compaction workload");
        assert!(matches!(result, Value::Int(value, _) if value == 7));
    }

    #[test]
    fn nested_interpreter_gc_preserves_caller_roots() {
        let module = zun_compiler::compile(
            "class Box { value: int; } function allocate(): int { let i: u64 = 0; while (i < 2300) { let garbage = new Box(1); i = i + 1; } return 0; } function main(): int { let live = new Box(9); allocate(); return live.value; }",
        )
        .expect("compile nested compaction workload");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run nested compaction workload");
        assert!(matches!(result, Value::Int(value, _) if value == 9));
    }

    #[test]
    fn extra_osr_roots_keep_managed_objects_alive() {
        let module = zun_compiler::compile("class Box { value: int; } function main() { }")
            .expect("compile root test module");
        let types = zun_types::TypeSystem::from_module(&module).expect("build type system");
        let mut heap = zun_gc::Heap::new();
        let object = heap.alloc(1, 1);
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        assert_eq!(
            collect_roots(&types, &[], &[], None, &[object]),
            0,
            "an object retained by an OSR snapshot must remain live"
        );
        assert_eq!(
            collect_roots(&types, &[], &[], None, &[]),
            1,
            "the object should be reclaimed after the snapshot root is consumed"
        );
    }

    #[test]
    fn resumable_session_completes_synchronous_async_function() {
        let module = zun_compiler::compile(
            "async function main(): Promise { return new Promise(); }",
        )
        .expect("compile synchronous async function");
        let entry = module.entry_point.expect("entry point");
        let mut output = CaptureOutput::default();
        let mut session = ResumableSession::new(
            &module,
            entry.rid(),
            Vec::new(),
            &mut output,
            None,
            None,
        )
        .expect("create resumable session");
        assert!(matches!(
            session.poll().expect("poll session"),
            ResumeResult::Complete(Value::Object(_))
        ));
    }

    #[test]
    fn generator_session_exposes_iterator_result_contract() {
        let module = zun_compiler::compile(
            "function* main(): int { yield 7; return 9; }",
        )
        .expect("compile generator function");
        let entry = module.entry_point.expect("entry point");
        let mut output = CaptureOutput::default();
        let mut generator = GeneratorSession::new(&module, entry.rid(), Vec::new(), &mut output)
            .expect("create generator session");

        let first = generator.next_result().expect("first generator result");
        assert!(matches!(first.value, Value::Float(value, zun_il::NumKind::F64) if value == 7.0));
        assert!(!first.done);

        let second = generator.next_result().expect("completion generator result");
        assert!(matches!(second.value, Value::Int(9, zun_il::NumKind::I32)));
        assert!(second.done);

        let terminal = generator.next_result().expect("sticky completion result");
        assert!(matches!(terminal.value, Value::Int(9, zun_il::NumKind::I32)));
        assert!(terminal.done);
    }

    #[test]
    fn generator_session_contextual_next_uses_owned_state() {
        let module = zun_compiler::compile("function* main(): int { yield 3; return 4; }")
            .expect("compile generator function");
        let entry = module.entry_point.expect("entry point");
        let mut legacy_output = CaptureOutput::default();
        let mut generator = GeneratorSession::new(&module, entry.rid(), Vec::new(), &mut legacy_output)
            .expect("create generator session");
        let mut first_output = CaptureOutput::default();
        assert!(matches!(
            generator.next_result_with_context(&mut first_output, None, None).expect("first next"),
            GeneratorIteratorResult { done: false, .. }
        ));
        let mut second_output = CaptureOutput::default();
        assert!(matches!(
            generator.next_result_with_context(&mut second_output, None, None).expect("second next"),
            GeneratorIteratorResult { done: true, .. }
        ));
    }

    #[test]
    fn runs_generator_call_through_for_of() {
        let module = zun_compiler::compile(
            "function* values(): int { yield 7; return 9; } function main(): any { for (let value of values()) { return value; } return 0; }",
        )
        .expect("compile generator for-of");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run generator for-of");
        assert!(matches!(result, Value::Float(7.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn resumes_generator_for_each_yield_and_skips_return_value() {
        let module = zun_compiler::compile(
            "function* values(seed: number): int { yield seed; yield seed + 1; return 9; } function main(): any { let total: number = 0.0; for (let value of values(2)) { total = total + (value as number); } return total; }",
        )
        .expect("compile multi-yield generator");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run multi-yield generator");
        assert!(matches!(result, Value::Float(5.0, zun_il::NumKind::F64)), "{result:?}");
    }

    #[test]
    fn owned_generator_session_injects_context_per_resume() {
        let module = zun_compiler::compile("function* main(): int { yield 7; return 9; }")
            .expect("compile generator function");
        let entry = module.entry_point.expect("entry point");
        let mut session = OwnedGeneratorSession::new(&module, entry.rid(), Vec::new())
            .expect("create owned generator session");
        let mut first_output = CaptureOutput::default();
        assert!(matches!(
            session.resume(&mut first_output, None, None).expect("first resume"),
            ResumeResult::Yielded(Value::Float(value, zun_il::NumKind::F64)) if value == 7.0
        ));
        let mut second_output = CaptureOutput::default();
        assert!(matches!(
            session.resume(&mut second_output, None, None).expect("second resume"),
            ResumeResult::Complete(Value::Int(value, zun_il::NumKind::I32)) if value == 9
        ));
    }

    #[test]
    fn generator_compaction_preserves_pending_object_state() {
        let module = zun_compiler::compile(
            "class Box { value: int; } function* main(): Box { let value = new Box(7); yield value; return value; }",
        )
        .expect("compile generator object state");
        let entry = module.entry_point.expect("entry point");
        let mut session = GeneratorSession::new(
            &module,
            entry.rid(),
            Vec::new(),
            &mut CaptureOutput::default(),
        )
        .expect("create generator session");
        let yielded = session.next().expect("yielded object");
        assert!(matches!(yielded, GeneratorResult::Yielded(Value::Object(_))));
        let compacted = session
            .compact_pending_frames()
            .expect("pending frame compaction");
        assert_eq!(compacted.moved_objects, 1);
        assert!(matches!(
            session.next().expect("completed object"),
            GeneratorResult::Complete(Value::Object(_))
        ));
    }

    #[test]
    fn resumable_session_suspends_and_resumes_on_promise_settlement() {
        let module = zun_compiler::compile(
            "async function main(): Promise { let p = new Promise(); await p; return p; }",
        )
        .expect("compile async await function");
        let entry = module.entry_point.expect("entry point");
        let mut output = CaptureOutput::default();
        let mut session = ResumableSession::new(
            &module,
            entry.rid(),
            Vec::new(),
            &mut output,
            None,
            None,
        )
        .expect("create resumable session");
        assert!(matches!(session.poll().expect("initial poll"), ResumeResult::Suspended));
        let compacted = session
            .collect_and_compact_pending_frame()
            .expect("compact pending async frame");
        assert_eq!(compacted.moved_objects, 1);
        let promise = session.awaited_promise().expect("relocated awaited promise");
        let value = Value::Int(42, zun_il::NumKind::I32);
        let boxed = value.box_any(session.heap_mut()).expect("box result");
        assert_eq!(session.heap_mut().promise_resolve(promise, boxed), Some(true));
        assert!(matches!(session.poll().expect("resumed poll"), ResumeResult::Complete(Value::Object(_))));
    }

    #[test]
    fn resumable_session_propagates_rejected_await() {
        let module = zun_compiler::compile(
            "async function main(): Promise { let p = new Promise(); await p; return p; }",
        )
        .expect("compile async await function");
        let entry = module.entry_point.expect("entry point");
        let mut output = CaptureOutput::default();
        let mut session = ResumableSession::new(
            &module,
            entry.rid(),
            Vec::new(),
            &mut output,
            None,
            None,
        )
        .expect("create resumable session");
        assert!(matches!(session.poll().expect("initial poll"), ResumeResult::Suspended));
        let promise = session.awaited_promise().expect("awaited promise");
        assert_eq!(session.heap_mut().promise_reject(promise, "boom"), Some(true));
        assert!(matches!(
            session.poll(),
            Err(RunError::Thrown(Value::Str(_)))
        ));
    }

    #[test]
    fn resumable_session_resumes_nested_async_callers() {
        let module = zun_compiler::compile(
            "async function child(): Promise { let p = new Promise(); await p; return p; } async function main(): Promise { let q = child(); await q; return q; }",
        )
        .expect("compile nested async functions");
        let entry = module.entry_point.expect("entry point");
        let mut output = CaptureOutput::default();
        let mut session = ResumableSession::new(
            &module,
            entry.rid(),
            Vec::new(),
            &mut output,
            None,
            None,
        )
        .expect("create resumable session");
        assert!(matches!(session.poll().expect("initial poll"), ResumeResult::Suspended));
        let promise = session.awaited_promise().expect("child awaited promise");
        let boxed = Value::Int(7, zun_il::NumKind::I32)
            .box_any(session.heap_mut())
            .expect("box child result");
        assert_eq!(session.heap_mut().promise_resolve(promise, boxed), Some(true));
        assert!(matches!(session.poll().expect("resume nested call"), ResumeResult::Complete(Value::Object(_))));
    }

    #[test]
    fn settled_await_reads_the_managed_promise_result() {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        let promise = heap.alloc_promise();
        let value = Value::Int(7, zun_il::NumKind::I32);
        let boxed = value.box_any(&mut heap).expect("box await result");
        assert_eq!(heap.promise_resolve(promise, boxed), Some(true));
        assert_eq!(
            settled_await_value(promise)
                .unwrap()
                .map(|resolved| resolved.as_int()),
            Some(7)
        );
    }

    #[test]
    fn executes_do_while_at_least_once() {
        let module = zun_compiler::compile(
            "function main(): int { let count: int = 0; do { count = count + 1; } while (false); return count; }",
        )
        .expect("compile do-while");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run do-while");
        assert_eq!(value.as_int(), 1);
    }

    #[test]
    fn do_while_continue_checks_the_condition() {
        let module = zun_compiler::compile(
            "function main(): int { let count: int = 0; do { count = count + 1; continue; } while (count < 3); return count; }",
        )
        .expect("compile do-while continue");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run do-while continue");
        assert_eq!(value.as_int(), 3);
    }

    #[test]
    fn executes_c_style_for_loop_and_continue() {
        let module = zun_compiler::compile(
            "function main(): int { let total: int = 0; for (let i: int = 0; i < 4; i = i + 1) { if (i == 2) { continue; } total = total + i; } return total; }",
        )
        .expect("compile c-style for");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run c-style for");
        assert_eq!(value.as_int(), 4);
    }

    #[test]
    fn short_circuit_logical_operators_return_boolean_values() {
        let module =
            zun_compiler::compile("function main(): boolean { return true || false && false; }")
                .expect("compile logical operators");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run logical operators");
        assert!(matches!(value, Value::Bool(true)));
    }

    #[test]
    fn conditional_expression_selects_one_branch() {
        let module = zun_compiler::compile(
            "function main(): number { let choose: boolean = false; return choose ? 1.0 : 2.0; }",
        )
        .expect("compile conditional expression");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run conditional expression");
        assert!(matches!(value, Value::Float(2.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn null_literal_is_preserved_by_tier_zero() {
        let module = zun_compiler::compile("function main(): any { return null; }")
            .expect("compile null literal");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("null should execute");
        assert!(matches!(value, Value::Null));
    }

    #[test]
    fn nullish_coalescing_selects_the_fallback_value() {
        let module = zun_compiler::compile(
            "function main(): number { let value: any = null; return value ?? 42.0; }",
        )
        .expect("compile nullish coalescing");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run nullish coalescing");
        assert!(matches!(value, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn optional_dynamic_access_short_circuits_null() {
        let module = zun_compiler::compile(
            "function main(): any { let value: any = null; return value?.answer; }",
        )
        .expect("compile optional access");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run optional access");
        assert!(matches!(value, Value::Null));
    }

    #[test]
    fn optional_dynamic_access_reads_non_null_property() {
        let module = zun_compiler::compile(
            "function main(): any { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); let value: any = object; return value?.answer; }",
        )
        .expect("compile optional property read");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run optional property read");
        assert!(matches!(value, Value::Any(any) if any.as_float() == Some(41.0)));
    }

    #[test]
    fn optional_managed_method_calls_non_null_receiver() {
        let module = zun_compiler::compile(
            "function main(): u64 { let value = new Map(); value.set(\"answer\", 41); return value?.size(); }",
        )
        .expect("compile optional managed method");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run optional managed method");
        assert!(matches!(value, Value::Int(1, zun_il::NumKind::U64)));
    }

    #[test]
    fn default_parameter_is_used_when_argument_is_omitted() {
        let module = zun_compiler::compile(
            "function add(value: number = 2.0): number { return value; } function main(): number { return add(); }",
        )
        .expect("compile default parameter");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run default parameter");
        assert!(matches!(value, Value::Float(2.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn rest_parameter_materializes_extra_arguments_as_an_array() {
        let module = zun_compiler::compile(
            "function count(...values: Array): u64 { return values.length; } function main(): u64 { return count(1, 2, 3); }",
        )
        .expect("compile rest parameter");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run rest parameter");
        assert!(matches!(value, Value::Int(3, zun_il::NumKind::U64)));
    }

    #[test]
    fn spread_array_feeds_a_zero_fixed_rest_function() {
        let module = zun_compiler::compile(
            "function count(...values: Array): u64 { return values.length; } function main(): u64 { return count(...new Array(3)); }",
        )
        .expect("compile spread call");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run spread call");
        assert!(matches!(value, Value::Int(3, zun_il::NumKind::U64)));
    }

    #[test]
    fn spread_array_follows_fixed_arguments_before_rest() {
        let module = zun_compiler::compile(
            "function count(prefix: u64, ...values: Array): u64 { return values.length; } function main(): u64 { return count(7, ...new Array(3)); }",
        )
        .expect("compile fixed spread call");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run fixed spread call");
        assert!(matches!(value, Value::Int(3, zun_il::NumKind::U64)));
    }

    #[test]
    fn array_destructuring_reads_elements_by_index() {
        let module = zun_compiler::compile(
            "function main(): number { let values = new Array(2); values[0] = 4; values[1] = 8; let [first, second] = values; return first as number + second as number; }",
        )
        .expect("compile array destructuring");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run array destructuring");
        assert!(matches!(value, Value::Float(12.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn object_destructuring_reads_dynamic_properties() {
        let module = zun_compiler::compile(
            "function main(): number { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); let {answer} = object; return answer as number; }",
        )
        .expect("compile object destructuring");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run object destructuring");
        assert!(matches!(value, Value::Float(41.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn method_default_parameter_preserves_the_receiver() {
        let module = zun_compiler::compile(
            "class Box { value(factor: number = 3.0): number { return factor; } } function main(): number { let box_value = new Box(); return box_value.value(); }",
        )
        .expect("compile method default parameter");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("run method default parameter");
        assert!(matches!(value, Value::Float(3.0, zun_il::NumKind::F64)));
    }
    use zun_il::{encode, Instruction, Operand};
    use zun_metadata::{FieldDef, MethodDef, TableKind, Token, TypeDef};
    use zun_module::write_body;

    #[test]
    fn catches_a_thrown_error_in_the_current_frame() {
        let module = zun_compiler::compile(
            "class Error {} function main(): int { try { throw new Error(); } catch (error: Error) { return 42; } }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("catch should handle throw");
        assert_eq!(value.as_int(), 42);
    }

    #[test]
    fn finally_runs_before_a_normal_return() {
        let module = zun_compiler::compile(
            "function main(): int { try { return 1; } finally { print(2); } }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("return should succeed");
        assert_eq!(value.as_int(), 1);
        assert_eq!(output.lines, vec!["2"]);
    }

    #[test]
    fn finally_runs_before_an_unhandled_rethrow() {
        let module = zun_compiler::compile(
            "class Error {} function main() { try { throw new Error(); } finally { print(2); } }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output);
        assert!(matches!(result, Err(RunError::Thrown(_))));
        assert_eq!(output.lines, vec!["2"]);
    }

    #[test]
    fn catches_an_exception_from_a_called_method() {
        let module = zun_compiler::compile(
            "class Error {} function fail(): int { throw new Error(); } function main(): int { try { return fail(); } catch (error: Error) { return 7; } }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("caller should catch child throw");
        assert_eq!(value.as_int(), 7);
    }

    #[test]
    fn nested_catch_can_rethrow_to_an_outer_handler() {
        let module = zun_compiler::compile(
            "class Error {} function main(): int { try { try { throw new Error(); } catch (inner: Error) { throw inner; } } catch (outer: Error) { return 9; } }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("outer catch should handle rethrow");
        assert_eq!(value.as_int(), 9);
    }

    #[test]
    fn inner_finally_rethrows_to_an_outer_catch_in_the_same_frame() {
        let module = zun_compiler::compile(
            "class Error {} function main(): int { try { try { throw new Error(); } finally { print(1); } } catch (outer: Error) { return 42; } }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("outer catch should receive rethrow");
        assert_eq!(value.as_int(), 42);
        assert_eq!(output.lines, vec!["1"]);
    }

    #[test]
    fn callee_finally_rethrows_to_a_caller_catch() {
        let module = zun_compiler::compile(
            "class Error {} function fail(): int { try { throw new Error(); } finally { print(1); } } function main(): int { try { return fail(); } catch (outer: Error) { return 42; } }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("caller catch should receive rethrow");
        assert_eq!(value.as_int(), 42);
        assert_eq!(output.lines, vec!["1"]);
    }

    #[test]
    fn return_inside_inner_try_runs_finally_before_outer_catch_path() {
        let module = zun_compiler::compile(
            "class Error {} function main(): int { try { try { return 1; } finally { print(1); } } catch (outer: Error) { return 42; } }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let value = run(&module, &mut output).expect("normal return should survive cleanup");
        assert_eq!(value.as_int(), 1);
        assert_eq!(output.lines, vec!["1"]);
    }

    #[test]
    fn runs_non_entry_method_with_arguments() {
        let module = zun_compiler::compile(
            "function add_one(value: int): int { return value + 1; } function main(): int { return 0; }",
        )
        .expect("compile");
        let mut output = CaptureOutput::default();
        let result = run_method(
            &module,
            1,
            vec![Value::Int(41, zun_il::NumKind::I32)],
            &mut output,
        )
        .expect("run method");
        assert!(matches!(result, Value::Int(42, zun_il::NumKind::I32)));
    }

    #[test]
    fn executes_indirect_call_for_non_capturing_function() {
        let mut module = zun_module::Module::new();
        let add_body = [
            Instruction::new(Opcode::LoadLocal, Operand::Local(0)),
            Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
            Instruction::simple(Opcode::AddF64),
            Instruction::simple(Opcode::Ret),
        ];
        let mut add_code = Vec::new();
        for instruction in add_body {
            encode(&instruction, &mut add_code);
        }
        let add_offset = write_body(
            &mut module.code,
            1,
            1,
            true,
            zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
            &[zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte()],
            &add_code,
        );
        module.tables.add_method_def(MethodDef {
            name: module.strings.intern("add_one"),
            signature: zun_metadata::BlobIndex(0),
            body_offset: add_offset,
            is_static: false,
        });
        let main_body = [
            Instruction::new(
                Opcode::LoadFunction,
                Operand::Token(Token::new(TableKind::MethodDef, 1)),
            ),
            Instruction::new(Opcode::ConstF64, Operand::Float64(41.0)),
            Instruction::new(
                Opcode::CallIndirect,
                Operand::Token(Token::new(TableKind::MethodDef, 1)),
            ),
            Instruction::simple(Opcode::Ret),
        ];
        let mut main_code = Vec::new();
        for instruction in main_body {
            encode(&instruction, &mut main_code);
        }
        let main_offset = write_body(
            &mut module.code,
            0,
            0,
            true,
            zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
            &[],
            &main_code,
        );
        module.tables.add_method_def(MethodDef {
            name: module.strings.intern("main"),
            signature: zun_metadata::BlobIndex(0),
            body_offset: main_offset,
            is_static: false,
        });
        module.entry_point = Some(Token::new(TableKind::MethodDef, 2));
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run indirect call");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_indirect_call_with_closure_environment() {
        let mut module = zun_module::Module::new();
        let environment_field = module.tables.add_field_def(FieldDef {
            name: module.strings.intern("captured"),
            signature: zun_metadata::BlobIndex(0),
            kind: zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
            is_static: false,
            is_private: false,
        });
        let environment_type = module.tables.add_type_def(TypeDef {
            name: module.strings.intern("ClosureEnvironment"),
            namespace: zun_metadata::StringIndex(0),
            base: Token::null(TableKind::TypeDef),
            field_start: environment_field.rid(),
            field_end: environment_field.rid() + 1,
            method_start: 1,
            method_end: 2,
        });
        let scratch_type = module.tables.add_type_def(TypeDef {
            name: module.strings.intern("Scratch"),
            namespace: zun_metadata::StringIndex(0),
            base: Token::null(TableKind::TypeDef),
            field_start: 0,
            field_end: 0,
            method_start: 0,
            method_end: 0,
        });
        let body = [
            Instruction::new(Opcode::LoadLocal, Operand::Local(1)),
            Instruction::new(Opcode::LoadLocal, Operand::Local(0)),
            Instruction::new(Opcode::LoadField, Operand::Token(environment_field)),
            Instruction::new(Opcode::ConstF64, Operand::Float64(1.0)),
            Instruction::simple(Opcode::AddF64),
            Instruction::simple(Opcode::AddF64),
            Instruction::simple(Opcode::Ret),
        ];
        let mut code = Vec::new();
        for instruction in body {
            encode(&instruction, &mut code);
        }
        let target_offset = write_body(
            &mut module.code,
            2,
            2,
            true,
            zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
            &[
                zun_il::JitKind::Object.to_byte(),
                zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
            ],
            &code,
        );
        module.tables.add_method_def(MethodDef {
            name: module.strings.intern("capturedAddOne"),
            signature: zun_metadata::BlobIndex(0),
            body_offset: target_offset,
            is_static: false,
        });
        let mut main_body = vec![
            Instruction::new(Opcode::ConstF64, Operand::Float64(41.0)),
            Instruction::new(Opcode::NewObj, Operand::Token(environment_type)),
            Instruction::new(
                Opcode::MakeClosure,
                Operand::Token(Token::new(TableKind::MethodDef, 1)),
            ),
        ];
        for _ in 0..300 {
            main_body.push(Instruction::new(
                Opcode::NewObj,
                Operand::Token(scratch_type),
            ));
            main_body.push(Instruction::simple(Opcode::Pop));
        }
        main_body.extend([
            Instruction::new(Opcode::ConstF64, Operand::Float64(0.0)),
            Instruction::new(
                Opcode::CallIndirect,
                Operand::Token(Token::null(TableKind::MethodDef)),
            ),
            Instruction::simple(Opcode::Ret),
        ]);
        let mut main_code = Vec::new();
        for instruction in main_body {
            encode(&instruction, &mut main_code);
        }
        let main_offset = write_body(
            &mut module.code,
            0,
            0,
            true,
            zun_il::JitKind::Num(zun_il::NumKind::F64).to_byte(),
            &[],
            &main_code,
        );
        module.tables.add_method_def(MethodDef {
            name: module.strings.intern("main"),
            signature: zun_metadata::BlobIndex(0),
            body_offset: main_offset,
            is_static: false,
        });
        module.entry_point = Some(Token::new(TableKind::MethodDef, 2));
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run closure environment call");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn closure_environment_is_an_interpreter_gc_root() {
        let mut heap = zun_gc::Heap::new();
        let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
        let environment = heap.alloc(1, 0);
        let function = Value::Function(zun_core::FunctionHandle::new(
            1,
            Some(environment.addr() as usize),
        ));
        let mut roots = Vec::new();
        collect_frame_roots(&[function], &[], &mut roots);
        assert_eq!(roots, vec![environment]);
    }

    #[test]
    fn compiles_named_function_reference_and_indirect_call() {
        let module = zun_compiler::compile(
            "function add_one(value: number): number { return value + 1.0; }\n\
             function main(): number { let f = add_one; return f(41.0); }",
        )
        .expect("compile function value source");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run function value source");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn passes_named_function_reference_through_typed_parameter() {
        let module = zun_compiler::compile(
            "function add_one(value: number): number { return value + 1.0; }\n\
             function apply(f: add_one, value: number): number { return f(value); }\n\
             function main(): number { return apply(add_one, 41.0); }",
        )
        .expect("compile typed function parameter");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run typed function parameter");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn resolves_forward_function_type_annotations() {
        let module = zun_compiler::compile(
            "function apply(f: add_one, value: number): number { return f(value); }\n\
             function main(): number { return apply(add_one, 41.0); }\n\
             function add_one(value: number): number { return value + 1.0; }",
        )
        .expect("compile forward function type");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run forward function type");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn invokes_generic_function_callback() {
        let module = zun_compiler::compile(
            "function add_one(value: number): number { return value + 1.0; }\n\
             function apply(f: Function, value: number): any { return f(value); }\n\
             function main(): any { return apply(add_one, 41.0); }",
        )
        .expect("compile generic function callback");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run generic function callback");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn finalization_registry_registers_and_unregisters_a_callback() {
        let module = zun_compiler::compile(
            "function cleanup(value: any): void { }\n\
             class Target {}\n\
             function main(): boolean { let registry = new FinalizationRegistry(); let target = new Target(); let id = registry.register(target, cleanup); return registry.unregister(id); }",
        )
        .expect("compile finalization registry");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run finalization registry");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn reflect_aliases_use_dynamic_object_storage() {
        let module = zun_compiler::compile(
            "function main(): any { let object = dynamic_new(); let key = \"answer\"; reflect_set(object, key, box_any(41)); return reflect_get(object, key); }",
        )
        .expect("compile reflect aliases");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run reflect aliases");
        assert!(matches!(result, Value::Any(value) if value.as_float() == Some(41.0)));
    }

    #[test]
    fn reflect_has_and_delete_use_dynamic_object_storage() {
        let module = zun_compiler::compile(
            "import { reflectHas, reflectDelete } from \"zun:collections\"; function main(): boolean { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); if (!reflectHas(object, \"answer\")) { return false; } if (!reflectDelete(object, \"answer\")) { return false; } return !reflectHas(object, \"answer\"); }",
        )
        .expect("compile reflect has/delete");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run reflect has/delete");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn constructs_proxy_with_managed_target_and_handler() {
        let module = zun_compiler::compile(
            "function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); return true; }",
        )
        .expect("compile proxy construction");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run proxy construction");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn proxy_get_falls_back_to_target_dynamic_properties() {
        let module = zun_compiler::compile(
            "import { proxyGet } from \"zun:collections\"; function main(): any { let target = dynamic_new(); dynamic_set(target, \"answer\", box_any(41)); let handler = dynamic_new(); let proxy = new Proxy(target, handler); return proxyGet(proxy, \"answer\"); }",
        )
        .expect("compile proxy get");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run proxy get");
        assert!(matches!(result, Value::Any(value) if value.as_float() == Some(41.0)));
    }

    #[test]
    fn proxy_set_falls_back_to_target_dynamic_properties() {
        let module = zun_compiler::compile(
            "import { proxySet, proxyGet } from \"zun:collections\"; function main(): any { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); proxySet(proxy, \"answer\", box_any(41)); return proxyGet(proxy, \"answer\"); }",
        )
        .expect("compile proxy set");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run proxy set");
        assert!(matches!(result, Value::Any(value) if value.as_float() == Some(41.0)));
    }

    #[test]
    fn proxy_has_and_delete_fall_back_to_target_dynamic_properties() {
        let module = zun_compiler::compile(
            "import { proxyHas, proxySet, proxyDelete } from \"zun:collections\"; function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); let proxy = new Proxy(target, handler); proxySet(proxy, \"answer\", box_any(41)); if (!proxyHas(proxy, \"answer\")) { return false; } return proxyDelete(proxy, \"answer\") && !proxyHas(proxy, \"answer\"); }",
        )
        .expect("compile proxy has/delete");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run proxy has/delete");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn proxy_get_invokes_a_named_handler_trap() {
        let module = zun_compiler::compile(
            "import { dynamicSetFunction } from \"zun:collections\"; import { proxyGet } from \"zun:collections\"; function getTrap(target: any, key: string): any { return box_any(99); } function main(): any { let target = dynamic_new(); let handler = dynamic_new(); dynamicSetFunction(handler, \"get\", getTrap); let proxy = new Proxy(target, handler); return proxyGet(proxy, \"answer\"); }",
        )
        .expect("compile proxy trap");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run proxy trap");
        assert!(matches!(result, Value::Any(value) if value.as_float() == Some(99.0)));
    }

    #[test]
    fn proxy_property_traps_invoke_named_handlers() {
        let module = zun_compiler::compile(
            "import { dynamicSetFunction, proxySet, proxyHas, proxyDelete } from \"zun:collections\"; function setTrap(target: any, key: string, value: any): boolean { return true; } function hasTrap(target: any, key: string): boolean { return true; } function deleteTrap(target: any, key: string): boolean { return true; } function main(): boolean { let target = dynamic_new(); let handler = dynamic_new(); dynamicSetFunction(handler, \"set\", setTrap); dynamicSetFunction(handler, \"has\", hasTrap); dynamicSetFunction(handler, \"deleteProperty\", deleteTrap); let proxy = new Proxy(target, handler); return proxySet(proxy, \"answer\", box_any(41)) && proxyHas(proxy, \"answer\") && proxyDelete(proxy, \"answer\"); }",
        )
        .expect("compile proxy property traps");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run proxy property traps");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn host_function_entry_reuses_callable_handle_dispatch() {
        let module = zun_compiler::compile(
            "function add_one(value: number): number { return value + 1.0; }\n\
             function main(): number { return 0.0; }",
        )
        .expect("compile host function callback");
        let rid = module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == "add_one")
            .map(|index| index as u32 + 1)
            .expect("add_one method");
        let mut output = CaptureOutput::default();
        let result = run_function(
            &module,
            zun_core::FunctionHandle::new(rid, None),
            vec![Value::Float(41.0, zun_il::NumKind::F64)],
            &mut output,
        )
        .expect("run host callback");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn invokes_a_non_capturing_arrow_closure() {
        let module =
            zun_compiler::compile("function main(): any { let f = x => x * 2.0; return f(21.0); }")
                .expect("compile arrow closure");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run arrow closure");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn invokes_an_arrow_closure_with_a_read_only_capture() {
        let module = zun_compiler::compile(
            "function main(): any { let k = 10.0; let f = x => x + k; return f(32.0); }",
        )
        .expect("compile captured arrow closure");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run captured arrow closure");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn mutable_capture_observes_binding_updates() {
        let module = zun_compiler::compile(
            "function main(): any { let k = 10.0; let f = x => x + k; k = 20.0; return f(22.0); }",
        )
        .expect("compile mutable captured arrow closure");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run mutable captured arrow closure");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn nested_arrow_closures_share_their_outer_environment() {
        let module = zun_compiler::compile(
            "function main(): any { let k = 10.0; let outer = x => { let inner = y => x + y + k; return inner(1.0); }; return outer(31.0); }",
        )
        .expect("compile nested arrow closures");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run nested arrow closures");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn iterates_an_array_with_for_of() {
        let module = zun_compiler::compile(
            "function main(): any { let values = new Array(2); values[0] = 1.0; values[1] = 2.0; for (let value of values) { print(value); } return 42.0; }",
        )
        .expect("compile array for-of");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array for-of");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
        assert_eq!(output.lines, vec!["1", "2"]);
    }

    #[test]
    fn invokes_array_symbol_iterator_from_source() {
        let module = zun_compiler::compile(
            "import { arrayIteratorNext } from \"zun:buffer\"; function main(): u64 { let values = new Array(1); values[0] = 7.0; let iterator = values[Symbol.iterator](); return arrayIteratorNext(iterator) as u64; }",
        )
        .expect("compile Symbol.iterator call");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Symbol.iterator call");
        assert!(matches!(result, Value::Int(7, zun_il::NumKind::U64)));
    }

    #[test]
    fn invokes_map_and_set_symbol_iterators_from_source() {
        let module = zun_compiler::compile(
            "import { mapIteratorNext, setIteratorNext } from \"zun:buffer\"; function main(): u64 { let map = new Map(); map.set(1.0, 10.0); let map_iterator = map[Symbol.iterator](); let pair: Array = mapIteratorNext(map_iterator) as Array; let set = new Set(); set.add(32.0); let set_iterator = set[Symbol.iterator](); let value = setIteratorNext(set_iterator); return pair[1] as u64 + value as u64; }",
        )
        .expect("compile collection Symbol.iterator calls");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run collection Symbol.iterator calls");
        assert!(matches!(result, Value::Int(42, zun_il::NumKind::U64)));
    }

    #[test]
    fn executes_switch_cases_and_default() {
        let module = zun_compiler::compile(
            "function main(): u64 { let value: u64 = 2; switch (value) { case 1: return 10; case 2: return 20; default: return 30; } }",
        )
        .expect("compile switch");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run switch");
        assert!(matches!(result, Value::Int(20, zun_il::NumKind::U64)));
    }

    #[test]
    fn iterates_map_and_set_values_in_insertion_order() {
        let module = zun_compiler::compile(
            "function main(): any { let map = new Map(); map.set(1.0, 10.0); map.set(2.0, 20.0); let set = new Set(); set.add(3.0); set.add(4.0); for (let value of map) { let pair: Array = value as Array; print(pair[1]); } for (let value of set) { print(value); } return 42.0; }",
        )
        .expect("compile managed collection for-of");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run managed collection for-of");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
        assert_eq!(output.lines, vec!["10", "20", "3", "4"]);
    }

    #[test]
    fn accesses_gc_backed_byte_buffers_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, bufferSetU8, bufferGetU8, bufferLength } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(4); bufferSetU8(buffer, 2, 127); if (bufferLength(buffer) == 4) { return bufferGetU8(buffer, 2) as u64; } return 0; }",
        )
        .expect("compile buffer intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run buffer intrinsics");
        assert!(matches!(result, Value::Int(127, zun_il::NumKind::U64)));
    }

    #[test]
    fn accesses_numeric_values_in_gc_backed_buffers_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, bufferWriteU32, bufferReadU32, bufferWriteF64, bufferReadF64 } from \"zun:buffer\"; function main(): number { let buffer = bufferNew(16); bufferWriteU32(buffer, 0, 16909060); bufferWriteF64(buffer, 8, 2.5); if (bufferReadU32(buffer, 0) == 16909060) { return bufferReadF64(buffer, 8); } return 0.0; }",
        )
        .expect("compile numeric buffer intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run numeric buffer intrinsics");
        assert!(matches!(result, Value::Float(2.5, zun_il::NumKind::F64)));
    }

    #[test]
    fn accesses_data_views_over_gc_backed_buffers_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, dataViewNew, dataViewSetU32, dataViewGetU32 } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(8); let view = dataViewNew(buffer, 2, 4); dataViewSetU32(view, 0, 16909060, true); return dataViewGetU32(view, 0, true) as u64; }",
        )
        .expect("compile DataView intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run DataView intrinsics");
        assert!(matches!(result, Value::Int(16909060, zun_il::NumKind::U64)));
    }

    #[test]
    fn accesses_data_view_u16_and_f64_operations_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, dataViewNew, dataViewSetU16, dataViewGetU16, dataViewSetF64, dataViewGetF64 } from \"zun:buffer\"; function main(): number { let buffer = bufferNew(16); let view = dataViewNew(buffer, 0, 16); dataViewSetU16(view, 0, 4660, false); dataViewSetF64(view, 8, 2.5, true); if (dataViewGetU16(view, 0, false) == 4660) { return dataViewGetF64(view, 8, true); } return 0.0; }",
        )
        .expect("compile extended DataView intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run extended DataView intrinsics");
        assert!(matches!(result, Value::Float(2.5, zun_il::NumKind::F64)));
    }

    #[test]
    fn accesses_gc_backed_u32_typed_arrays_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, u32ArrayNew, u32ArraySet, u32ArrayGet } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(16); let values = u32ArrayNew(buffer, 0, 16); u32ArraySet(values, 2, 16909060); return u32ArrayGet(values, 2) as u64; }",
        )
        .expect("compile typed-array intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run typed-array intrinsics");
        assert!(matches!(result, Value::Int(16909060, zun_il::NumKind::U64)));
    }

    #[test]
    fn accesses_gc_backed_u8_typed_arrays_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, u8ArrayNew, u8ArraySet, u8ArrayGet } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(4); let values = u8ArrayNew(buffer, 0, 4); u8ArraySet(values, 1, 255); return u8ArrayGet(values, 1) as u64; }",
        )
        .expect("compile Uint8Array intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Uint8Array intrinsics");
        assert!(matches!(result, Value::Int(255, zun_il::NumKind::U64)));
    }

    #[test]
    fn accesses_gc_backed_f64_typed_arrays_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, f64ArrayNew, f64ArraySet, f64ArrayGet } from \"zun:buffer\"; function main(): number { let buffer = bufferNew(16); let values = f64ArrayNew(buffer, 0, 16); f64ArraySet(values, 1, 2.5); return f64ArrayGet(values, 1); }",
        )
        .expect("compile Float64Array intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Float64Array intrinsics");
        assert!(matches!(result, Value::Float(2.5, zun_il::NumKind::F64)));
    }

    #[test]
    fn accesses_gc_backed_u16_typed_arrays_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, u16ArrayNew, u16ArraySet, u16ArrayGet } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(8); let values = u16ArrayNew(buffer, 0, 8); u16ArraySet(values, 2, 4660); return u16ArrayGet(values, 2) as u64; }",
        )
        .expect("compile Uint16Array intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Uint16Array intrinsics");
        assert!(matches!(result, Value::Int(4660, zun_il::NumKind::U64)));
    }

    #[test]
    fn constructs_typed_array_views_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, u8ArraySet, u8ArrayGet } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(4); let values = new Uint8Array(buffer, 0, 4); u8ArraySet(values, 1, 255); return u8ArrayGet(values, 1) as u64; }",
        )
        .expect("compile typed-array constructor");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run typed-array constructor");
        assert!(matches!(result, Value::Int(255, zun_il::NumKind::U64)));
    }

    #[test]
    fn typed_array_views_expose_instance_accessors() {
        let module = zun_compiler::compile(
            "import { bufferNew } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(4); let values = new Uint8Array(buffer, 0, 4); values.set(1, 255); return values.get(1) as u64; }",
        )
        .expect("compile typed-array instance accessors");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run typed-array instance accessors");
        assert!(matches!(result, Value::Int(255, zun_il::NumKind::U64)));
    }

    #[test]
    fn constructs_data_views_from_source() {
        let module = zun_compiler::compile(
            "import { bufferNew, dataViewSetU32, dataViewGetU32 } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(8); let view = new DataView(buffer, 0, 8); dataViewSetU32(view, 0, 16909060, true); return dataViewGetU32(view, 0, true) as u64; }",
        )
        .expect("compile DataView constructor");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run DataView constructor");
        assert!(matches!(result, Value::Int(16909060, zun_il::NumKind::U64)));
    }

    #[test]
    fn data_views_expose_instance_accessors() {
        let module = zun_compiler::compile(
            "import { bufferNew } from \"zun:buffer\"; function main(): u64 { let buffer = bufferNew(8); let view = new DataView(buffer, 0, 8); view.setUint32(0, 16909060, true); return view.getUint32(0, true) as u64; }",
        )
        .expect("compile DataView instance accessors");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run DataView instance accessors");
        assert!(matches!(result, Value::Int(16909060, zun_il::NumKind::U64)));
    }

    #[test]
    fn constructs_array_buffers_from_source() {
        let module = zun_compiler::compile(
            "import { bufferLength } from \"zun:buffer\"; function main(): u64 { let buffer = new ArrayBuffer(4); return bufferLength(buffer); }",
        )
        .expect("compile ArrayBuffer constructor");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run ArrayBuffer constructor");
        assert!(matches!(result, Value::Int(4, zun_il::NumKind::U64)));
    }

    #[test]
    fn exposes_array_iterator_handles_to_source() {
        let module = zun_compiler::compile(
            "import { arrayIteratorNew, arrayIteratorNext } from \"zun:buffer\"; function main(): any { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; let iterator = arrayIteratorNew(values); print(arrayIteratorNext(iterator)); print(arrayIteratorNext(iterator)); return arrayIteratorNext(iterator); }",
        )
        .expect("compile array iterator intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array iterator intrinsics");
        assert!(matches!(result, Value::Void));
        assert_eq!(output.lines, vec!["7", "9"]);
    }

    #[test]
    fn exposes_set_iterator_handles_to_source() {
        let module = zun_compiler::compile(
            "import { setIteratorNew, setIteratorNext } from \"zun:buffer\"; function main(): any { let values = new Set(); values.add(7.0); values.add(9.0); let iterator = setIteratorNew(values); print(setIteratorNext(iterator)); print(setIteratorNext(iterator)); return setIteratorNext(iterator); }",
        )
        .expect("compile set iterator intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run set iterator intrinsics");
        assert!(matches!(result, Value::Void));
        assert_eq!(output.lines, vec!["7", "9"]);
    }

    #[test]
    fn exposes_map_iterator_entry_pairs_to_source() {
        let module = zun_compiler::compile(
            "import { mapIteratorNew, mapIteratorNext } from \"zun:buffer\"; function main(): any { let values = new Map(); values.set(7.0, 70.0); values.set(9.0, 90.0); let iterator = mapIteratorNew(values); print(mapIteratorNext(iterator)); return mapIteratorNext(iterator); }",
        )
        .expect("compile map iterator intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run map iterator intrinsics");
        assert!(matches!(result, Value::Object(_)));
        assert_eq!(output.lines, vec!["[any:Object]"]);
    }

    #[test]
    fn uses_runtime_backed_map_and_set_intrinsics() {
        let module = zun_compiler::compile(
            "import { mapNew, mapSet, mapHas, mapSize, setNew, setAdd, setHas, setSize } from \"zun:collections\"; function main(): any { let map = mapNew(); mapSet(map, 7.0, 42.0); let set = setNew(); setAdd(set, 7.0); if (mapHas(map, 7.0)) { if (setHas(set, 7.0)) { return mapSize(map) + setSize(set); } } return 0.0; }",
        )
        .expect("compile collection intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run collection intrinsics");
        assert!(matches!(result, Value::Int(2, _)));
    }

    #[test]
    fn constructs_managed_collections_from_source() {
        let module = zun_compiler::compile(
            "import { mapSet, mapHas, setAdd, setHas, weakMapSet, weakMapHas } from \"zun:collections\"; class Key {} function main(): int { let map = new Map(); mapSet(map, 1.0, 2.0); let set = new Set(); setAdd(set, 1.0); let key = new Key(); let value = new Key(); let weak = new WeakMap(); weakMapSet(weak, key, value); if (mapHas(map, 1.0)) { if (setHas(set, 1.0)) { if (weakMapHas(weak, key)) { return 1; } } } return 0; }",
        )
        .expect("compile managed collection constructors");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run managed collection constructors");
        assert!(matches!(result, Value::Int(1, _)));
    }

    #[test]
    fn calls_managed_collection_methods_with_a_receiver() {
        let module = zun_compiler::compile(
            "class Key {} function main(): int { let map = new Map(); map.set(1.0, 2.0); let set = new Set(); set.add(1.0); let key = new Key(); let value = new Key(); let weak = new WeakMap(); weak.set(key, value); if (map.has(1.0)) { if (set.has(1.0)) { if (weak.has(key)) { if (map.size() == 1) { if (set.size() == 1) { map.clear(); set.clear(); return 1; } } } } } return 0; }",
        )
        .expect("compile managed collection methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run managed collection methods");
        assert!(matches!(result, Value::Int(1, _)));
    }

    #[test]
    fn removes_and_clears_runtime_backed_collections() {
        let module = zun_compiler::compile(
            "import { mapNew, mapSet, mapHas, mapDelete, mapClear, setNew, setAdd, setHas, setDelete, setClear } from \"zun:collections\"; function main(): int { let map = mapNew(); mapSet(map, 1.0, 2.0); mapDelete(map, 1.0); if (mapHas(map, 1.0)) { return 1; } mapSet(map, 1.0, 2.0); mapClear(map); let set = setNew(); setAdd(set, 1.0); setDelete(set, 1.0); if (setHas(set, 1.0)) { return 2; } setAdd(set, 1.0); setClear(set); if (setHas(set, 1.0)) { return 3; } return 0; }",
        )
        .expect("compile collection removal intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run collection removal intrinsics");
        assert!(matches!(result, Value::Int(0, _)));
    }

    #[test]
    fn uses_gc_backed_weak_map_handles() {
        let module = zun_compiler::compile(
            "import { weakMapNew, weakMapSet, weakMapHas } from \"zun:collections\"; class Key {} function main(): int { let key = new Key(); let value = new Key(); let key2 = new Key(); let value2 = new Key(); let map = weakMapNew(); weakMapSet(map, key, value); weakMapSet(map, key2, value2); if (weakMapHas(map, key)) { if (weakMapHas(map, key2)) { return 1; } } return 0; }",
        )
        .expect("compile weak map intrinsics");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run weak map intrinsics");
        assert!(matches!(result, Value::Int(1, _)));
    }

    #[test]
    fn executes_dynamic_property_il() {
        let mut module = zun_module::Module::new();
        let property = module.strings.intern("answer");
        let mut code = Vec::new();
        for instruction in [
            Instruction::simple(Opcode::NewDynamic),
            Instruction::simple(Opcode::Dup),
            Instruction::new(Opcode::ConstI32, Operand::Imm32(41)),
            Instruction::simple(Opcode::BoxAny),
            Instruction::new(Opcode::StoreDynamic, Operand::StringRef(property.0)),
            Instruction::simple(Opcode::Pop),
            Instruction::new(Opcode::LoadDynamic, Operand::StringRef(property.0)),
            Instruction::simple(Opcode::UnboxAny),
            Instruction::simple(Opcode::Ret),
        ] {
            encode(&instruction, &mut code);
        }
        let offset = write_body(
            &mut module.code,
            0,
            0,
            true,
            zun_il::JitKind::Num(zun_il::NumKind::I32).to_byte(),
            &[],
            &code,
        );
        module.tables.add_method_def(MethodDef {
            name: module.strings.intern("main"),
            signature: zun_metadata::BlobIndex(0),
            body_offset: offset,
            is_static: false,
        });
        module.entry_point = Some(Token::new(TableKind::MethodDef, 1));

        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("dynamic property IL");
        assert!(matches!(result, Value::Int(41, zun_il::NumKind::I64)));
    }

    #[test]
    fn executes_dynamic_property_source() {
        let module = zun_compiler::compile(
            "function main(): any { let object = dynamic_new(); dynamic_set(object, \"answer\", box_any(41)); return dynamic_get(object, \"answer\"); }",
        )
        .expect("compile dynamic source");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run dynamic source");
        assert!(matches!(result, Value::Any(any) if any.as_float() == Some(41.0)));
    }

    #[test]
    fn executes_extern_method_with_receiver() {
        let module = zun_compiler::compile(
            "import { receiverValue } from \"zun:math\";\n\
             class Service { extern receiverValue(value: number): number; }\n\
             function main(): number { let service = new Service(); return service.receiverValue(7.5); }",
        )
        .expect("compile extern method");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run extern method");
        assert!(matches!(result, Value::Float(value, _) if value == 7.5));
    }

    #[test]
    fn executes_array_il_with_bounds_checked_storage() {
        let mut module = zun_module::Module::new();
        let mut code = Vec::new();
        for instruction in [
            Instruction::new(Opcode::ConstI32, Operand::Imm32(1)),
            Instruction::simple(Opcode::NewArray),
            Instruction::simple(Opcode::Dup),
            Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
            Instruction::new(Opcode::ConstI32, Operand::Imm32(41)),
            Instruction::simple(Opcode::StoreArray),
            Instruction::simple(Opcode::Pop),
            Instruction::new(Opcode::ConstI32, Operand::Imm32(0)),
            Instruction::simple(Opcode::LoadArray),
            Instruction::simple(Opcode::Ret),
        ] {
            encode(&instruction, &mut code);
        }
        let offset = write_body(
            &mut module.code,
            0,
            0,
            true,
            zun_il::JitKind::Num(zun_il::NumKind::I32).to_byte(),
            &[],
            &code,
        );
        module.tables.add_method_def(MethodDef {
            name: module.strings.intern("main"),
            signature: zun_metadata::BlobIndex(0),
            body_offset: offset,
            is_static: false,
        });
        module.entry_point = Some(Token::new(TableKind::MethodDef, 1));
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array IL");
        assert!(matches!(result, Value::Int(41, zun_il::NumKind::I64)));
    }

    #[test]
    fn executes_source_array_constructor() {
        let module = zun_compiler::compile("function main(): Array { return new Array(2); }")
            .expect("compile array constructor");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array constructor");
        assert!(matches!(result, Value::Object(_)));
    }

    #[test]
    fn executes_source_array_index_read() {
        let module = zun_compiler::compile(
            "function main(): any { let values = new Array(1); return values[0]; }",
        )
        .expect("compile array index");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array index");
        assert!(matches!(result, Value::Int(0, zun_il::NumKind::I64)));
    }

    #[test]
    fn executes_source_array_index_assignment() {
        let module = zun_compiler::compile(
            "function main(): any { let values = new Array(1); values[0] = 41; return values[0]; }",
        )
        .expect("compile array assignment");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array assignment");
        assert!(
            matches!(result, Value::Float(41.0, zun_il::NumKind::F64)),
            "{result:?}"
        );
    }

    #[test]
    fn executes_array_count_source_method() {
        let module = zun_compiler::compile(
            "class Array { count(value: any): u64 { let total: u64 = 0; let index: u64 = 0; while (index < this.length) { if (this[index] == value) { total = total + 1; } index = index + 1; } return total; } }\n\
             function main(): u64 { let values = new Array(4); values[0] = 7; values[1] = 3; values[2] = 7; values[3] = 7; let needle: any = values[0]; return values.count(needle); }",
        )
        .expect("compile Array count");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array count");
        assert!(matches!(result, Value::Int(value, _) if value == 3));
    }

    #[test]
    fn executes_array_clear_source_method() {
        let module = zun_compiler::compile(
            "class Array { clear(): void { while (this.length > 0) { this.pop(); } } }\n\
             function main(): u64 { let values = new Array(2); values[0] = 7; values[1] = 3; values.clear(); return values.length; }",
        )
        .expect("compile Array clear");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array clear");
        assert!(matches!(result, Value::Int(value, _) if value == 0));
    }

    #[test]
    fn executes_array_average_source_method() {
        let module = zun_compiler::compile(
            "class Array { sum(): number { let total: number = 0.0; let index: u64 = 0; while (index < this.length) { total = total + (this[index] as number); index = index + 1; } return total; } average(): number { if (this.length == 0) { return 0.0; } return this.sum() / (this.length as number); } }\n\
             function main(): number { let values = new Array(2); values[0] = 2.0; values[1] = 6.0; return values.average(); }",
        )
        .expect("compile Array average");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array average");
        assert!(matches!(result, Value::Float(value, _) if value == 4.0));
    }

    #[test]
    fn executes_array_min_and_max_source_methods() {
        let module = zun_compiler::compile(
            "class Array { min(): number { if (this.length == 0) { return 0.0; } let result: number = this[0] as number; let index: u64 = 1; while (index < this.length) { let value: number = this[index] as number; if (value < result) { result = value; } index = index + 1; } return result; } max(): number { if (this.length == 0) { return 0.0; } let result: number = this[0] as number; let index: u64 = 1; while (index < this.length) { let value: number = this[index] as number; if (value > result) { result = value; } index = index + 1; } return result; } }\n\
             function main(): number { let values = new Array(3); values[0] = 9.0; values[1] = 2.0; values[2] = 7.0; return values.min() + values.max(); }",
        )
        .expect("compile Array min and max");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array min and max");
        assert!(matches!(result, Value::Float(value, _) if value == 11.0));
    }

    #[test]
    fn executes_array_sort_source_method() {
        let module = zun_compiler::compile(
            "class Array { sort(): Array { let index: u64 = 0; while (index < this.length) { let smallest: u64 = index; let scan: u64 = index + 1; while (scan < this.length) { let candidate: number = this[scan] as number; let currentSmallest: number = this[smallest] as number; if (candidate < currentSmallest) { smallest = scan; } scan = scan + 1; } let temporary: any = this[index]; this[index] = this[smallest]; this[smallest] = temporary; index = index + 1; } return this; } }\n\
             function main(): number { let values = new Array(3); values[0] = 9.0; values[1] = 2.0; values[2] = 7.0; values.sort(); return values[0] as number; }",
        )
        .expect("compile Array sort");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array sort");
        assert!(matches!(result, Value::Float(value, _) if value == 2.0));
    }

    #[test]
    fn executes_array_find_index_source_method() {
        let module = zun_compiler::compile(
            "class Array { indexOf(value: any): int { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return index as int; } index = index + 1; } return -1; } findIndex(value: any): int { return this.indexOf(value); } }\n\
             function main(): int { let values = new Array(3); values[0] = 4; values[1] = 8; values[2] = 15; let needle: any = values[1]; return values.findIndex(needle); }",
        )
        .expect("compile Array findIndex");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array findIndex");
        assert!(matches!(result, Value::Int(value, _) if value == 1));
    }

    #[test]
    fn executes_array_last_index_of_source_method() {
        let module = zun_compiler::compile(
            "class Array { lastIndexOf(value: any): int { let index: int = this.length as int; while (index > 0) { index = index - 1; if (this[index as u64] == value) { return index; } } return -1; } }\n\
             function main(): int { let values = new Array(3); values[0] = 4; values[1] = 8; values[2] = 8; let needle: any = values[1]; return values.lastIndexOf(needle); }",
        )
        .expect("compile Array lastIndexOf");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array lastIndexOf");
        assert!(matches!(result, Value::Int(value, _) if value == 2));
    }

    #[test]
    fn executes_array_all_source_method() {
        let module = zun_compiler::compile(
            "class Array { all(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] != value) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 4.0; values[1] = 4.0; values[2] = 4.0; let needle: any = values[0]; return values.all(needle); }",
        )
        .expect("compile Array all");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array all");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_none_source_method() {
        let module = zun_compiler::compile(
            "class Array { none(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; let needle: any = values[0]; return values.none(needle); }",
        )
        .expect("compile Array none");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array none");
        assert!(matches!(result, Value::Bool(false)));
    }

    #[test]
    fn executes_array_any_source_method() {
        let module = zun_compiler::compile(
            "class Array { any(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; let needle: any = values[1]; return values.any(needle); }",
        )
        .expect("compile Array any");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array any");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_is_constant_source_method() {
        let module = zun_compiler::compile(
            "class Array { isConstant(): boolean { if (this.length < 2) { return true; } let first: any = this[0]; let index: u64 = 1; while (index < this.length) { if (this[index] != first) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 9.0; values[1] = 9.0; values[2] = 9.0; return values.isConstant(); }",
        )
        .expect("compile Array isConstant");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isConstant");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_is_palindrome_source_method() {
        let module = zun_compiler::compile(
            "class Array { isPalindrome(): boolean { if (this.length < 2) { return true; } let left: u64 = 0; let right: u64 = this.length - 1; while (left < right) { if (this[left] != this[right]) { return false; } left = left + 1; right = right - 1; } return true; } } function main(): boolean { let values = new Array(5); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 2.0; values[4] = 1.0; return values.isPalindrome(); }",
        )
        .expect("compile Array isPalindrome");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isPalindrome");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_is_disjoint_source_method() {
        let module = zun_compiler::compile(
            "class Array { isDisjoint(other: Array): boolean { let left: u64 = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { return false; } right = right + 1; } left = left + 1; } return true; } } function main(): boolean { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 3.0; right[1] = 4.0; return left.isDisjoint(right); }",
        )
        .expect("compile Array isDisjoint");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isDisjoint");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_intersection_source_method() {
        let module = zun_compiler::compile(
            "class Array { intersection(other: Array): Array { let matches: u64 = 0; let left: u64 = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { matches = matches + 1; break; } right = right + 1; } left = left + 1; } let result = new Array(matches); let output: u64 = 0; left = 0; while (left < this.length) { let right: u64 = 0; while (right < other.length) { if (this[left] == other[right]) { result[output] = this[left]; output = output + 1; break; } right = right + 1; } left = left + 1; } return result; } } function main(): number { let left = new Array(3); left[0] = 1.0; left[1] = 2.0; left[2] = 3.0; let right = new Array(2); right[0] = 2.0; right[1] = 4.0; let common = left.intersection(right); return common[0] as number; }",
        )
        .expect("compile Array intersection");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array intersection");
        assert!(matches!(result, Value::Float(2.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_difference_source_method() {
        let module = zun_compiler::compile(
            "class Array { difference(other: Array): Array { let remaining: u64 = 0; let left: u64 = 0; while (left < this.length) { let right: u64 = 0; let found: boolean = false; while (right < other.length) { if (this[left] == other[right]) { found = true; break; } right = right + 1; } if (!found) { remaining = remaining + 1; } left = left + 1; } let result = new Array(remaining); let output: u64 = 0; left = 0; while (left < this.length) { let right: u64 = 0; let found: boolean = false; while (right < other.length) { if (this[left] == other[right]) { found = true; break; } right = right + 1; } if (!found) { result[output] = this[left]; output = output + 1; } left = left + 1; } return result; } } function main(): number { let left = new Array(3); left[0] = 1.0; left[1] = 2.0; left[2] = 3.0; let right = new Array(2); right[0] = 2.0; right[1] = 4.0; let remaining = left.difference(right); return remaining[1] as number; }",
        )
        .expect("compile Array difference");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array difference");
        assert!(matches!(result, Value::Float(3.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_union_source_method() {
        let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } union(other: Array): Array { let total: u64 = this.length; let right: u64 = 0; while (right < other.length) { if (!this.contains(other[right])) { total = total + 1; } right = right + 1; } let result = new Array(total); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } right = 0; while (right < other.length) { if (!this.contains(other[right])) { result[index] = other[right]; index = index + 1; } right = right + 1; } return result; } } function main(): number { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 2.0; right[1] = 3.0; let combined = left.union(right); return combined[2] as number; }",
        )
        .expect("compile Array union");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array union");
        assert!(matches!(result, Value::Float(3.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_symmetric_difference_source_method() {
        let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } difference(other: Array): Array { let remaining: u64 = 0; let left: u64 = 0; while (left < this.length) { if (!other.contains(this[left])) { remaining = remaining + 1; } left = left + 1; } let result = new Array(remaining); let output: u64 = 0; left = 0; while (left < this.length) { if (!other.contains(this[left])) { result[output] = this[left]; output = output + 1; } left = left + 1; } return result; } union(other: Array): Array { let total: u64 = this.length; let right: u64 = 0; while (right < other.length) { if (!this.contains(other[right])) { total = total + 1; } right = right + 1; } let result = new Array(total); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } right = 0; while (right < other.length) { if (!this.contains(other[right])) { result[index] = other[right]; index = index + 1; } right = right + 1; } return result; } symmetricDifference(other: Array): Array { let leftOnly = this.difference(other); let rightOnly = other.difference(this); return leftOnly.union(rightOnly); } } function main(): number { let left = new Array(2); left[0] = 1.0; left[1] = 2.0; let right = new Array(2); right[0] = 2.0; right[1] = 3.0; let result = left.symmetricDifference(right); return result[1] as number; }",
        )
        .expect("compile Array symmetricDifference");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array symmetricDifference");
        assert!(matches!(result, Value::Float(3.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_is_subset_of_source_method() {
        let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } isSubsetOf(other: Array): boolean { let index: u64 = 0; while (index < this.length) { if (!other.contains(this[index])) { return false; } index = index + 1; } return true; } } function main(): boolean { let subset = new Array(2); subset[0] = 1.0; subset[1] = 2.0; let full = new Array(3); full[0] = 1.0; full[1] = 2.0; full[2] = 3.0; return subset.isSubsetOf(full); }",
        )
        .expect("compile Array isSubsetOf");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isSubsetOf");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_is_superset_of_source_method() {
        let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } isSubsetOf(other: Array): boolean { let index: u64 = 0; while (index < this.length) { if (!other.contains(this[index])) { return false; } index = index + 1; } return true; } isSupersetOf(other: Array): boolean { return other.isSubsetOf(this); } } function main(): boolean { let subset = new Array(2); subset[0] = 1.0; subset[1] = 2.0; let full = new Array(3); full[0] = 1.0; full[1] = 2.0; full[2] = 3.0; return full.isSupersetOf(subset); }",
        )
        .expect("compile Array isSupersetOf");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isSupersetOf");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_swap_source_method() {
        let module = zun_compiler::compile(
            "class Array { swap(first: u64, second: u64): Array { let temporary: any = this[first]; this[first] = this[second]; this[second] = temporary; return this; } }\n\
             function main(): number { let values = new Array(2); values[0] = 4.0; values[1] = 8.0; values.swap(0, 1); return values[0] as number; }",
        )
        .expect("compile Array swap");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array swap");
        assert!(matches!(result, Value::Float(value, _) if value == 8.0));
    }

    #[test]
    fn executes_array_rotate_left_source_method() {
        let module = zun_compiler::compile(
            "class Array { rotateLeft(shift: u64): Array { if (this.length == 0) { return this; } let amount: u64 = shift % this.length; let rotation: u64 = 0; while (rotation < amount) { let first: any = this[0]; let index: u64 = 1; while (index < this.length) { this[index - 1] = this[index]; index = index + 1; } this[this.length - 1] = first; rotation = rotation + 1; } return this; } }\n\
             function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values.rotateLeft(1); return values[0] as number; }",
        )
        .expect("compile Array rotateLeft");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array rotateLeft");
        assert!(matches!(result, Value::Float(value, _) if value == 2.0));
    }

    #[test]
    fn executes_array_rotate_right_source_method() {
        let module = zun_compiler::compile(
            "class Array { rotateRight(shift: u64): Array { if (this.length == 0) { return this; } let amount: u64 = shift % this.length; let rotation: u64 = 0; while (rotation < amount) { let last: any = this[this.length - 1]; let index: u64 = this.length - 1; while (index > 0) { this[index] = this[index - 1]; index = index - 1; } this[0] = last; rotation = rotation + 1; } return this; } }\n\
             function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values.rotateRight(1); return values[0] as number; }",
        )
        .expect("compile Array rotateRight");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array rotateRight");
        assert!(matches!(result, Value::Float(value, _) if value == 3.0));
    }

    #[test]
    fn executes_array_take_and_drop_source_methods() {
        let module = zun_compiler::compile(
            "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } take(count: u64): Array { let limit: u64 = count; if (limit > this.length) { limit = this.length; } return this.slice(0, limit); } drop(count: u64): Array { let start: u64 = count; if (start > this.length) { start = this.length; } return this.slice(start, this.length); } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let prefix = values.take(2); let suffix = values.drop(1); return prefix[1] as number + suffix[0] as number; }",
        )
        .expect("compile Array take/drop");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array take/drop");
        assert!(matches!(result, Value::Float(value, _) if value == 40.0));
    }

    #[test]
    fn executes_array_remove_at_source_method() {
        let module = zun_compiler::compile(
            "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let removed: any = values.removeAt(1); return (removed as number) + (values[1] as number); }",
        )
        .expect("compile Array removeAt");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array removeAt");
        assert!(matches!(result, Value::Float(value, _) if value == 50.0));
    }

    #[test]
    fn executes_array_insert_at_source_method() {
        let module = zun_compiler::compile(
            "class Array { insertAt(index: u64, value: any): Array { let cursor: u64 = this.length; this.push(value); while (cursor > index) { this[cursor] = this[cursor - 1]; cursor = cursor - 1; } this[index] = value; return this; } } function main(): number { let values = new Array(2); values[0] = 10.0; values[1] = 30.0; values.insertAt(1, values[0]); return (values[1] as number) + (values[2] as number); }",
        )
        .expect("compile Array insertAt");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array insertAt");
        assert!(matches!(result, Value::Float(value, _) if value == 40.0));
    }

    #[test]
    fn executes_array_remove_first_source_method() {
        let module = zun_compiler::compile(
            "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } removeFirst(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { this.removeAt(index); return true; } index = index + 1; } return false; } } function main(): number { let values = new Array(3); values[0] = 10.0; values[1] = 20.0; values[2] = 30.0; let needle: any = values[1]; if (!values.removeFirst(needle)) { return -1.0; } return values[1] as number; }",
        )
        .expect("compile Array removeFirst");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array removeFirst");
        assert!(matches!(result, Value::Float(value, _) if value == 30.0));
    }

    #[test]
    fn executes_array_remove_all_source_method() {
        let module = zun_compiler::compile(
            "class Array { removeAt(index: u64): any { let removed: any = this[index]; let cursor: u64 = index; while (cursor + 1 < this.length) { this[cursor] = this[cursor + 1]; cursor = cursor + 1; } this.pop(); return removed; } removeAll(value: any): u64 { let removed: u64 = 0; let index: u64 = 0; while (index < this.length) { if (this[index] == value) { this.removeAt(index); removed = removed + 1; } else { index = index + 1; } } return removed; } } function main(): number { let values = new Array(4); values[0] = 7.0; values[1] = 3.0; values[2] = 7.0; values[3] = 5.0; let needle: any = values[0]; return values.removeAll(needle) as number; }",
        )
        .expect("compile Array removeAll");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array removeAll");
        assert!(matches!(result, Value::Float(value, _) if value == 2.0));
    }

    #[test]
    fn executes_array_equals_source_method() {
        let module = zun_compiler::compile(
            "class Array { equals(other: Array): boolean { if (this.length != other.length) { return false; } let index: u64 = 0; while (index < this.length) { if (this[index] != other[index]) { return false; } index = index + 1; } return true; } } function main(): boolean { let left = new Array(2); left[0] = 4.0; left[1] = 8.0; let right = new Array(2); right[0] = 4.0; right[1] = 8.0; return left.equals(right); }",
        )
        .expect("compile Array equals");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array equals");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_is_sorted_source_method() {
        let module = zun_compiler::compile(
            "class Array { isSorted(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current < previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 2.0; return values.isSorted(); }",
        )
        .expect("compile Array isSorted");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isSorted");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_is_strictly_increasing_source_method() {
        let module = zun_compiler::compile(
            "class Array { isStrictlyIncreasing(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current <= previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; return values.isStrictlyIncreasing(); }",
        )
        .expect("compile Array isStrictlyIncreasing");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isStrictlyIncreasing");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_is_strictly_decreasing_source_method() {
        let module = zun_compiler::compile(
            "class Array { isStrictlyDecreasing(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current >= previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 3.0; values[1] = 2.0; values[2] = 1.0; return values.isStrictlyDecreasing(); }",
        )
        .expect("compile Array isStrictlyDecreasing");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isStrictlyDecreasing");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_is_descending_source_method() {
        let module = zun_compiler::compile(
            "class Array { isDescending(): boolean { if (this.length < 2) { return true; } let index: u64 = 1; while (index < this.length) { let previous: number = this[index - 1] as number; let current: number = this[index] as number; if (current > previous) { return false; } index = index + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 3.0; values[1] = 2.0; values[2] = 2.0; return values.isDescending(); }",
        )
        .expect("compile Array isDescending");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isDescending");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_is_unique_source_method() {
        let module = zun_compiler::compile(
            "class Array { isUnique(): boolean { let outer: u64 = 0; while (outer < this.length) { let inner: u64 = outer + 1; while (inner < this.length) { if (this[outer] == this[inner]) { return false; } inner = inner + 1; } outer = outer + 1; } return true; } } function main(): boolean { let values = new Array(3); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; return values.isUnique(); }",
        )
        .expect("compile Array isUnique");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array isUnique");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_transpose_source_method() {
        let module = zun_compiler::compile(
            "class Array { transpose(): Array { if (this.length == 0) { return new Array(0); } let first: Array = this[0] as Array; let rows: u64 = this.length; let columns: u64 = first.length; let result = new Array(columns); let column: u64 = 0; while (column < columns) { let rowResult = new Array(rows); let row: u64 = 0; while (row < rows) { let source: Array = this[row] as Array; rowResult[row] = source[column]; row = row + 1; } result[column] = rowResult; column = column + 1; } return result; } } function main(): number { let matrix = new Array(2); let first = new Array(2); first[0] = 1.0; first[1] = 2.0; let second = new Array(2); second[0] = 3.0; second[1] = 4.0; matrix[0] = first; matrix[1] = second; let result = matrix.transpose(); let column = result[1] as Array; return column[0] as number; }",
        )
        .expect("compile Array transpose");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array transpose");
        assert!(matches!(result, Value::Float(value, _) if value == 2.0));
    }

    #[test]
    fn executes_array_chunk_source_method() {
        let module = zun_compiler::compile(
            "class Array { chunk(size: u64): Array { if (size == 0) { return new Array(0); } let groups: u64 = this.length / size; if (this.length % size != 0) { groups = groups + 1; } let result = new Array(groups); let group: u64 = 0; let start: u64 = 0; while (group < groups) { let remaining: u64 = this.length - start; let count: u64 = size; if (remaining < count) { count = remaining; } let part = new Array(count); let index: u64 = 0; while (index < count) { part[index] = this[start + index]; index = index + 1; } result[group] = part; start = start + count; group = group + 1; } return result; } } function main(): number { let values = new Array(5); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 4.0; values[4] = 5.0; let groups = values.chunk(2); let last = groups[2] as Array; return last[0] as number; }",
        )
        .expect("compile Array chunk");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array chunk");
        assert!(matches!(result, Value::Float(value, _) if value == 5.0));
    }

    #[test]
    fn executes_array_windowed_source_method() {
        let module = zun_compiler::compile(
            "class Array { windowed(size: u64): Array { if (size == 0) { return new Array(0); } if (size > this.length) { return new Array(0); } let count: u64 = this.length - size + 1; let result = new Array(count); let start: u64 = 0; while (start < count) { let window = new Array(size); let index: u64 = 0; while (index < size) { window[index] = this[start + index]; index = index + 1; } result[start] = window; start = start + 1; } return result; } } function main(): number { let values = new Array(4); values[0] = 1.0; values[1] = 2.0; values[2] = 3.0; values[3] = 4.0; let windows = values.windowed(3); let first = windows[0] as Array; return first[0] as number; }",
        )
        .expect("compile Array windowed");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array windowed");
        assert!(matches!(result, Value::Float(value, _) if value == 1.0));
    }

    #[test]
    fn executes_array_product_source_method() {
        let module = zun_compiler::compile(
            "class Array { product(): number { let total: number = 1.0; let index: u64 = 0; while (index < this.length) { total = total * (this[index] as number); index = index + 1; } return total; } }\n\
             function main(): number { let values = new Array(3); values[0] = 2.0; values[1] = 3.0; values[2] = 4.0; return values.product(); }",
        )
        .expect("compile Array product");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array product");
        assert!(matches!(result, Value::Float(value, _) if value == 24.0));
    }

    #[test]
    fn executes_break_inside_loop() {
        let module = zun_compiler::compile(
            "function main(): u64 { let index: u64 = 0; while (index < 10) { if (index == 3) { break; } index = index + 1; } return index; }",
        )
        .expect("compile break");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run break");
        assert!(matches!(result, Value::Int(value, _) if value == 3));
    }

    #[test]
    fn executes_continue_inside_loop() {
        let module = zun_compiler::compile(
            "function main(): u64 { let index: u64 = 0; let total: u64 = 0; while (index < 5) { index = index + 1; if (index == 3) { continue; } total = total + index; } return total; }",
        )
        .expect("compile continue");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run continue");
        assert!(matches!(result, Value::Int(value, _) if value == 12));
    }

    #[test]
    fn executes_source_array_length() {
        let module = zun_compiler::compile(
            "function main(): u64 { let values = new Array(2); return values.length; }",
        )
        .expect("compile array length");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array length");
        assert!(matches!(result, Value::Int(2, zun_il::NumKind::U64)));
    }

    #[test]
    fn executes_source_array_pop() {
        let module = zun_compiler::compile(
            "function main(): any { let values = new Array(1); values[0] = 41.0; return values.pop(); }",
        )
        .expect("compile array pop");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array pop");
        assert!(matches!(result, Value::Float(41.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_source_array_push() {
        let module = zun_compiler::compile(
            "function main(): u64 { let values = new Array(0); return values.push(41.0); }",
        )
        .expect("compile array push");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array push");
        assert!(matches!(result, Value::Int(1, zun_il::NumKind::U64)));
    }

    #[test]
    fn executes_explicit_numeric_unbox_from_array_value() {
        let module = zun_compiler::compile(
            "function main(): number { let values = new Array(1); values[0] = 41.0; return values[0] as number; }",
        )
        .expect("compile array numeric cast");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array numeric cast");
        assert!(matches!(result, Value::Float(41.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_string_primitive_methods_through_static_class_binding() {
        let module = zun_compiler::compile(
            "import { length, contains } from \"zun:string\";\n\
             class String { extern length(): u64; extern contains(needle: string): boolean; }\n\
             function main(): boolean { let value: string = \"zun\"; if (value.length() == 3) { return value.contains(\"u\"); } return false; }",
        )
        .expect("compile String primitive methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String primitive methods");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_includes_method() {
        let module = zun_compiler::compile(
            "import { includes } from \"zun:string\";\n\
             class String { extern includes(needle: string): boolean; }\n\
             function main(): boolean { let value: string = \"zun runtime\"; return value.includes(\"runtime\"); }",
        )
        .expect("compile String includes");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String includes");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_index_of_with_utf16_indices() {
        let module = zun_compiler::compile(
            "import { indexOf } from \"zun:string\";\n\
             class String { extern indexOf(needle: string): int; }\n\
             function main(): int { let value: string = \"😀zun\"; return value.indexOf(\"zun\"); }",
        )
        .expect("compile String indexOf");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String indexOf");
        assert!(matches!(result, Value::Int(2, zun_il::NumKind::I32)));
    }

    #[test]
    fn executes_string_last_index_of() {
        let module = zun_compiler::compile(
            "import { lastIndexOf } from \"zun:string\";\n\
             class String { extern lastIndexOf(needle: string): int; }\n\
             function main(): int { let value: string = \"zunzun\"; return value.lastIndexOf(\"zun\"); }",
        )
        .expect("compile String lastIndexOf");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String lastIndexOf");
        assert!(matches!(result, Value::Int(3, zun_il::NumKind::I32)));
    }

    #[test]
    fn executes_string_replace_primitive_method() {
        let module = zun_compiler::compile(
            "import { replace } from \"zun:string\";\n\
             class String { extern replace(needle: string, replacement: string): string; }\n\
             function main(): string { let value: string = \"zun vm\"; return value.replace(\"vm\", \"runtime\"); }",
        )
        .expect("compile String replace");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String replace");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun runtime"));
    }

    #[test]
    fn executes_string_substring_with_utf16_indices() {
        let module = zun_compiler::compile(
            "import { substring } from \"zun:string\";\n\
             class String { extern substring(start: u64, end: u64): string; }\n\
             function main(): string { let value: string = \"😀zun\"; return value.substring(2, 5); }",
        )
        .expect("compile String substring");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String substring");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun"));
    }

    #[test]
    fn executes_string_pad_start_primitive_method() {
        let module = zun_compiler::compile(
            "import { padStart } from \"zun:string\";\n\
             class String { extern padStart(targetLength: u64, fill: string): string; }\n\
             function main(): string { let value: string = \"zun\"; return value.padStart(5, \"0\"); }",
        )
        .expect("compile String padStart");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String padStart");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "00zun"));
    }

    #[test]
    fn executes_string_pad_end_primitive_method() {
        let module = zun_compiler::compile(
            "import { padEnd } from \"zun:string\";\n\
             class String { extern padEnd(targetLength: u64, fill: string): string; }\n\
             function main(): string { let value: string = \"zun\"; return value.padEnd(5, \"0\"); }",
        )
        .expect("compile String padEnd");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String padEnd");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun00"));
    }

    #[test]
    fn executes_string_case_conversion_methods() {
        let module = zun_compiler::compile(
            "import { toLowerCase, toUpperCase } from \"zun:string\";\n\
             class String { extern toLowerCase(): string; extern toUpperCase(): string; }\n\
             function main(): string { let value: string = \"ZuN\"; return value.toLowerCase() + value.toUpperCase(); }",
        )
        .expect("compile String case methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String case methods");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "zunZUN"));
    }

    #[test]
    fn executes_string_trim_boundary_methods() {
        let module = zun_compiler::compile(
            "import { trimStart, trimEnd } from \"zun:string\";\n\
             class String { extern trimStart(): string; extern trimEnd(): string; }\n\
             function main(): string { let value: string = \"  zun  \"; return value.trimStart().trimEnd(); }",
        )
        .expect("compile String trim boundary methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String trim boundary methods");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun"));
    }

    #[test]
    fn executes_string_is_blank_source_method() {
        let module = zun_compiler::compile(
            "import { trim, length } from \"zun:string\";\n\
             class String { extern trim(): string; extern length(): u64; isEmpty(): boolean { return this.length() == 0; } isBlank(): boolean { return this.trim().isEmpty(); } }\n\
             function main(): boolean { let value: string = \"   \"; return value.isBlank(); }",
        )
        .expect("compile String isBlank");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isBlank");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_not_empty_source_method() {
        let module = zun_compiler::compile(
            "import { length } from \"zun:string\";\n\
             class String { extern length(): u64; isNotEmpty(): boolean { return this.length() != 0; } }\n\
             function main(): boolean { let value: string = \"zun\"; return value.isNotEmpty(); }",
        )
        .expect("compile String isNotEmpty");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isNotEmpty");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_reverse_native_method() {
        let module = zun_compiler::compile(
            "import { reverse } from \"zun:string\";\n\
             class String { extern reverse(): string; }\n\
             function main(): string { let value: string = \"zun\"; return value.reverse(); }",
        )
        .expect("compile String reverse");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String reverse");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "nuz"));
    }

    #[test]
    fn executes_string_is_ascii_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAscii(): boolean { let index: u64 = 0; while (index < this.length()) { if (this.charCodeAt(index) > 127) { return false; } index = index + 1; } return true; } }\n\
             function main(): boolean { let value: string = \"zun\"; return value.isAscii(); }",
        )
        .expect("compile String isAscii");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isAscii");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_count_native_method() {
        let module = zun_compiler::compile(
            "import { count } from \"zun:string\";\n\
             class String { extern count(needle: string): int; }\n\
             function main(): int { let value: string = \"aaaa\"; return value.count(\"aa\"); }",
        )
        .expect("compile String count");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String count");
        assert!(matches!(result, Value::Int(value, _) if value == 2));
    }

    #[test]
    fn executes_string_is_numeric_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isNumeric(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } }\n\
             function main(): boolean { let value: string = \"12345\"; return value.isNumeric(); }",
        )
        .expect("compile String isNumeric");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isNumeric");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_digit_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"2048\"; return value.isDigit(); }",
        )
        .expect("compile String isDigit");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isDigit");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_vowel_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isVowel(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let vowel: boolean = false; if (code == 65) { vowel = true; } if (code == 69) { vowel = true; } if (code == 73) { vowel = true; } if (code == 79) { vowel = true; } if (code == 85) { vowel = true; } if (code == 97) { vowel = true; } if (code == 101) { vowel = true; } if (code == 105) { vowel = true; } if (code == 111) { vowel = true; } if (code == 117) { vowel = true; } if (!vowel) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"AeIOu\"; return value.isVowel(); }",
        )
        .expect("compile String isVowel");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isVowel");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_consonant_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isAlpha(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } index = index + 1; } return true; } isConsonant(): boolean { if (!this.isAlpha()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 65) { return false; } if (code == 69) { return false; } if (code == 73) { return false; } if (code == 79) { return false; } if (code == 85) { return false; } if (code == 97) { return false; } if (code == 101) { return false; } if (code == 105) { return false; } if (code == 111) { return false; } if (code == 117) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Rhythm\"; return value.isConsonant(); }",
        )
        .expect("compile String isConsonant");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isConsonant");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_quoted_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isQuoted(): boolean { if (this.length() < 2) { return false; } let first: u32 = this.charCodeAt(0); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } } function main(): boolean { let value: string = \"\\\"zun\\\"\"; return value.isQuoted(); }",
        )
        .expect("compile String isQuoted");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isQuoted");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_unquote_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt, substring } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; extern substring(start: u64, end: u64): string; isQuoted(): boolean { if (this.length() < 2) { return false; } let first: u32 = this.charCodeAt(0); let last: u32 = this.charCodeAt(this.length() - 1); if (first == 34) { return last == 34; } if (first == 39) { return last == 39; } return false; } unquote(): string { if (!this.isQuoted()) { return this; } return this.substring(1, this.length() - 1); } } function main(): string { let value: string = \"\\\"zun\\\"\"; return value.unquote(); }",
        )
        .expect("compile String unquote");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String unquote");
        assert!(matches!(result, Value::Str(value) if value == "zun".into()));
    }

    #[test]
    fn executes_string_is_wrapped_by_source_method() {
        let module = zun_compiler::compile(
            "import { startsWith, endsWith } from \"zun:string\"; class String { extern startsWith(prefix: string): boolean; extern endsWith(suffix: string): boolean; isWrappedBy(prefix: string, suffix: string): boolean { if (!this.startsWith(prefix)) { return false; } return this.endsWith(suffix); } } function main(): boolean { let value: string = \"<zun>\"; return value.isWrappedBy(\"<\", \">\"); }",
        )
        .expect("compile String isWrappedBy");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isWrappedBy");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_quote_source_method() {
        let module = zun_compiler::compile(
            "class String { quote(): string { return \"\\\"\" + this + \"\\\"\"; } } function main(): string { let value: string = \"zun\"; return value.quote(); }",
        )
        .expect("compile String quote");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String quote");
        assert!(matches!(result, Value::Str(value) if value == "\"zun\"".into()));
    }

    #[test]
    fn executes_string_compare_native_method() {
        let module = zun_compiler::compile(
            "import { compare } from \"zun:string\";\n\
             class String { extern compare(other: string): int; }\n\
             function main(): int { let value: string = \"abc\"; return value.compare(\"abd\"); }",
        )
        .expect("compile String compare");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String compare");
        assert!(matches!(result, Value::Int(value, _) if value == -1));
    }

    #[test]
    fn executes_string_is_whitespace_source_method() {
        let module = zun_compiler::compile(
            "import { length, trim } from \"zun:string\";\n\
             class String { extern length(): u64; extern trim(): string; isEmpty(): boolean { return this.length() == 0; } isWhitespace(): boolean { if (this.isEmpty()) { return false; } return this.trim().isEmpty(); } }\n\
             function main(): boolean { let value: string = \"   \"; return value.isWhitespace(); }",
        )
        .expect("compile String isWhitespace");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isWhitespace");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_alpha_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isAlpha(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } index = index + 1; } return true; } }\n\
             function main(): boolean { let value: string = \"ZunRuntime\"; return value.isAlpha(); }",
        )
        .expect("compile String isAlpha");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isAlpha");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_alphanumeric_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isAlphanumeric(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { if (code < 65) { return false; } if (code > 90) { if (code < 97) { return false; } if (code > 122) { return false; } } } index = index + 1; } return true; } }\n\
             function main(): boolean { let value: string = \"Zun2026\"; return value.isAlphanumeric(); }",
        )
        .expect("compile String isAlphanumeric");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isAlphanumeric");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_case_predicates_source_methods() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\";\n\
             class String { extern length(): u64; extern charCodeAt(index: u64): u32; isLowerCase(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code >= 65) { if (code <= 90) { return false; } if (code >= 97) { if (code <= 122) { hasLetter = true; } } } index = index + 1; } return hasLetter; } isUpperCase(): boolean { if (this.length() == 0) { return false; } let index: u64 = 0; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code >= 97) { if (code <= 122) { return false; } } if (code >= 65) { if (code <= 90) { hasLetter = true; } } index = index + 1; } return hasLetter; } }\n\
             function main(): boolean { let lower: string = \"hello\"; let upper: string = \"WORLD\"; if (!lower.isLowerCase()) { return false; } return upper.isUpperCase(); }",
        )
        .expect("compile String case predicates");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String case predicates");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_to_int_native_method() {
        let module = zun_compiler::compile(
            "import { toInt } from \"zun:string\";\n\
             class String { extern toInt(): int; }\n\
             function main(): int { let value: string = \" -42 \"; return value.toInt(); }",
        )
        .expect("compile String toInt");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String toInt");
        assert!(matches!(result, Value::Int(value, _) if value == -42));
    }

    #[test]
    fn executes_string_to_float_native_method() {
        let module = zun_compiler::compile(
            "import { toFloat } from \"zun:string\";\n\
             class String { extern toFloat(): number; }\n\
             function main(): number { let value: string = \" 3.5 \"; return value.toFloat(); }",
        )
        .expect("compile String toFloat");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String toFloat");
        assert!(matches!(result, Value::Float(value, _) if value == 3.5));
    }

    #[test]
    fn executes_string_to_boolean_native_method() {
        let module = zun_compiler::compile(
            "import { toBoolean } from \"zun:string\";\n\
             class String { extern toBoolean(): boolean; }\n\
             function main(): boolean { let value: string = \" true \"; return value.toBoolean(); }",
        )
        .expect("compile String toBoolean");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String toBoolean");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_number_primitive_methods_through_static_class_binding() {
        let module = zun_compiler::compile(
            "import { toFixed } from \"zun:number\";\n\
             class Number { extern toFixed(digits: int): string; }\n\
             function main(): string { let value: number = 12.5; return value.toFixed(1); }",
        )
        .expect("compile Number primitive methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number primitive methods");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "12.5"));
    }

    #[test]
    fn executes_number_sign_source_method() {
        let module = zun_compiler::compile(
            "class Number { sign(): number { if (this < 0.0) { return -1.0; } if (this > 0.0) { return 1.0; } return 0.0; } }\n\
             function main(): number { let value: number = -42.0; return value.sign(); }",
        )
        .expect("compile Number sign");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number sign");
        assert!(matches!(result, Value::Float(value, _) if value == -1.0));
    }

    #[test]
    fn executes_number_sign_predicates_source_methods() {
        let module = zun_compiler::compile(
            "class Number { isPositive(): boolean { return this > 0.0; } isNonZero(): boolean { return this != 0.0; } } function main(): boolean { let value: number = 3.0; if (!value.isPositive()) { return false; } return value.isNonZero(); }",
        )
        .expect("compile Number sign predicates");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number sign predicates");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_number_is_between_source_method() {
        let module = zun_compiler::compile(
            "class Number { isBetween(low: number, high: number): boolean { if (this < low) { return false; } return this <= high; } } function main(): boolean { let value: number = 5.0; return value.isBetween(1.0, 10.0); }",
        )
        .expect("compile Number isBetween");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number isBetween");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_number_is_in_unit_interval_source_method() {
        let module = zun_compiler::compile(
            "class Number { isBetween(low: number, high: number): boolean { if (this < low) { return false; } return this <= high; } isInUnitInterval(): boolean { return this.isBetween(0.0, 1.0); } } function main(): boolean { let value: number = 0.75; return value.isInUnitInterval(); }",
        )
        .expect("compile Number isInUnitInterval");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number isInUnitInterval");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_number_is_between_exclusive_source_method() {
        let module = zun_compiler::compile(
            "class Number { isBetweenExclusive(low: number, high: number): boolean { if (this <= low) { return false; } return this < high; } } function main(): boolean { let value: number = 5.0; return value.isBetweenExclusive(0.0, 10.0); }",
        )
        .expect("compile Number isBetweenExclusive");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number isBetweenExclusive");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_number_distance_to_source_method() {
        let module = zun_compiler::compile(
            "class Number { abs(): number { if (this < 0.0) { return -this; } return this; } distanceTo(other: number): number { return (this - other).abs(); } } function main(): number { let value: number = 3.0; return value.distanceTo(10.5); }",
        )
        .expect("compile Number distanceTo");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number distanceTo");
        assert!(matches!(result, Value::Float(7.5, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_number_inverse_lerp_source_method() {
        let module = zun_compiler::compile(
            "class Number { inverseLerp(low: number, high: number): number { if (low == high) { return 0.0; } return (this - low) / (high - low); } } function main(): number { let value: number = 25.0; return value.inverseLerp(0.0, 100.0); }",
        )
        .expect("compile Number inverseLerp");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number inverseLerp");
        assert!(matches!(result, Value::Float(0.25, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_number_smooth_step_source_method() {
        let module = zun_compiler::compile(
            "class Number { clamp(low: number, high: number): number { if (this < low) { return low; } if (this > high) { return high; } return this; } inverseLerp(low: number, high: number): number { if (low == high) { return 0.0; } return (this - low) / (high - low); } smoothStep(edge0: number, edge1: number): number { let amount: number = this.inverseLerp(edge0, edge1).clamp(0.0, 1.0); return amount * amount * (3.0 - 2.0 * amount); } } function main(): number { let value: number = 50.0; return value.smoothStep(0.0, 100.0); }",
        )
        .expect("compile Number smoothStep");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number smoothStep");
        assert!(matches!(result, Value::Float(0.5, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_number_is_approximately_source_method() {
        let module = zun_compiler::compile(
            "class Number { isApproximately(other: number, epsilon: number): boolean { let difference: number = this - other; if (difference < 0.0) { difference = -difference; } return difference <= epsilon; } } function main(): boolean { let value: number = 1.001; return value.isApproximately(1.0, 0.01); }",
        )
        .expect("compile Number isApproximately");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number isApproximately");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_math_power_intrinsic() {
        let module = zun_compiler::compile(
            "import { powNumber } from \"zun:math\"; function main(): number { return powNumber(2.0, 3.0); }",
        )
        .expect("compile math power");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run math power");
        assert!(matches!(result, Value::Float(value, _) if value == 8.0));
    }

    #[test]
    fn executes_math_trigonometry_intrinsics() {
        let module = zun_compiler::compile(
            "import { sinNumber, cosNumber } from \"zun:math\"; function main(): number { return sinNumber(0.0) + cosNumber(0.0); }",
        )
        .expect("compile math trigonometry");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run math trigonometry");
        assert!(matches!(result, Value::Float(value, _) if value == 1.0));
    }

    #[test]
    fn executes_math_exponential_and_logarithm_intrinsics() {
        let module = zun_compiler::compile(
            "import { expNumber, logNumber } from \"zun:math\"; function main(): number { return expNumber(0.0) + logNumber(1.0); }",
        )
        .expect("compile math exponential and logarithm");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run math exponential and logarithm");
        assert!(matches!(result, Value::Float(value, _) if value == 1.0));
    }

    #[test]
    fn executes_math_tangent_and_atan2_intrinsics() {
        let module = zun_compiler::compile(
            "import { tanNumber, atan2Number } from \"zun:math\"; function main(): number { return tanNumber(0.0) + atan2Number(0.0, 1.0); }",
        )
        .expect("compile math tangent and atan2");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run math tangent and atan2");
        assert!(matches!(result, Value::Float(value, _) if value == 0.0));
    }

    #[test]
    fn executes_math_hypot_intrinsic() {
        let module = zun_compiler::compile(
            "import { hypotNumber } from \"zun:math\"; function main(): number { return hypotNumber(3.0, 4.0); }",
        )
        .expect("compile math hypot");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run math hypot");
        assert!(matches!(result, Value::Float(value, _) if value == 5.0));
    }

    #[test]
    fn executes_number_finiteness_source_methods() {
        let module = zun_compiler::compile(
            "class Number { isNaN(): boolean { return this != this; } isFinite(): boolean { return this == this; } }\n\
             function main(): boolean { let value: number = 42.0; if (!value.isFinite()) { return false; } return !value.isNaN(); }",
        )
        .expect("compile Number finiteness methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number finiteness methods");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_number_is_integer_source_method() {
        let module = zun_compiler::compile(
            "class Number { isInteger(): boolean { return (this as int) as number == this; } }\n\
             function main(): boolean { let value: number = 42.0; return value.isInteger(); }",
        )
        .expect("compile Number isInteger");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number isInteger");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_number_parity_source_methods() {
        let module = zun_compiler::compile(
            "class Number { isInteger(): boolean { return (this as int) as number == this; } isEven(): boolean { if (!this.isInteger()) { return false; } return (this as int) % 2 == 0; } isOdd(): boolean { if (!this.isInteger()) { return false; } return (this as int) % 2 != 0; } }\n\
             function main(): boolean { let value: number = 42.0; if (!value.isEven()) { return false; } return !value.isOdd(); }",
        )
        .expect("compile Number parity methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number parity methods");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_number_to_int_source_method() {
        let module = zun_compiler::compile(
            "class Number { toInt(): int { return this as int; } }\n\
             function main(): int { let value: number = 12.75; return value.toInt(); }",
        )
        .expect("compile Number toInt");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number toInt");
        assert!(matches!(result, Value::Int(value, _) if value == 12));
    }

    #[test]
    fn executes_number_to_boolean_source_method() {
        let module = zun_compiler::compile(
            "class Number { toBoolean(): boolean { return this != 0.0; } }\n\
             function main(): boolean { let value: number = 1.0; return value.toBoolean(); }",
        )
        .expect("compile Number toBoolean");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number toBoolean");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_number_to_string_primitive_method() {
        let module = zun_compiler::compile(
            "import { toString } from \"zun:number\";\n\
             class Number { extern toString(): string; }\n\
             function main(): string { let value: number = 12.5; return value.toString(); }",
        )
        .expect("compile Number toString");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number toString");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "12.5"));
    }

    #[test]
    fn executes_number_source_methods() {
        let module = zun_compiler::compile(
            "class Number { abs(): number { if (this < 0.0) { return -this; } return this; } isZero(): boolean { return this == 0.0; } isNegative(): boolean { return this < 0.0; } }\n\
             function main(): number { let value: number = -7.5; if (value.isNegative()) { return value.abs(); } return 0.0; }",
        )
        .expect("compile Number source methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number source methods");
        assert!(matches!(result, Value::Float(7.5, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_number_clamp_source_method() {
        let module = zun_compiler::compile(
            "class Number { clamp(low: number, high: number): number { if (this < low) { return low; } if (this > high) { return high; } return this; } }\n\
             function main(): number { let value: number = 12.0; return value.clamp(0.0, 10.0); }",
        )
        .expect("compile Number clamp");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number clamp");
        assert!(matches!(result, Value::Float(10.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_number_min_and_max_source_methods() {
        let module = zun_compiler::compile(
            "class Number { min(other: number): number { if (this < other) { return this; } return other; } max(other: number): number { if (this > other) { return this; } return other; } }\n\
             function main(): number { let value: number = 7.0; return value.min(10.0) + value.max(3.0); }",
        )
        .expect("compile Number min max");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number min max");
        assert!(matches!(result, Value::Float(14.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_number_lerp_source_method() {
        let module = zun_compiler::compile(
            "class Number { lerp(target: number, amount: number): number { return this + (target - this) * amount; } } function main(): number { let value: number = 10.0; return value.lerp(20.0, 0.25); }",
        )
        .expect("compile Number lerp");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number lerp");
        assert!(matches!(result, Value::Float(12.5, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_number_remap_source_method() {
        let module = zun_compiler::compile(
            "class Number { remap(fromLow: number, fromHigh: number, toLow: number, toHigh: number): number { if (fromLow == fromHigh) { return toLow; } let amount: number = (this - fromLow) / (fromHigh - fromLow); return toLow + (toHigh - toLow) * amount; } } function main(): number { let value: number = 5.0; return value.remap(0.0, 10.0, 100.0, 200.0); }",
        )
        .expect("compile Number remap");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Number remap");
        assert!(matches!(result, Value::Float(150.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_string_source_methods_with_primitive_receiver() {
        let module = zun_compiler::compile(
            "class String { concat(other: string): string { return this + other; } withSeparator(separator: string, other: string): string { return this + separator + other; } }\n\
             function main(): string { let value: string = \"zun\"; return value.withSeparator(\":\", \"vm\"); }",
        )
        .expect("compile String source methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String source methods");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun:vm"));
    }

    #[test]
    fn executes_string_is_empty_source_method() {
        let module = zun_compiler::compile(
            "import { length } from \"zun:string\";\n\
             class String { extern length(): u64; isEmpty(): boolean { return this.length() == 0; } }\n\
             function main(): boolean { let value: string = \"\"; return value.isEmpty(); }",
        )
        .expect("compile String isEmpty");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isEmpty");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_palindrome_source_method() {
        let module = zun_compiler::compile(
            "import { reverse } from \"zun:string\"; class String { extern reverse(): string; isPalindrome(): boolean { return this == this.reverse(); } } function main(): boolean { let value: string = \"level\"; return value.isPalindrome(); }",
        )
        .expect("compile String isPalindrome");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isPalindrome");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_hexadecimal_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexadecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { if (code < 65) { return false; } if (code > 70) { if (code < 97) { return false; } if (code > 102) { return false; } } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"a1F0\"; return value.isHexadecimal(); }",
        )
        .expect("compile String isHexadecimal");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isHexadecimal");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_hex_digit_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isHexDigit(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 70) { valid = true; } } if (code >= 97) { if (code <= 102) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"aF09\"; return value.isHexDigit(); }",
        )
        .expect("compile String isHexDigit");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isHexDigit");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_binary_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isBinary(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 49) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"101001\"; return value.isBinary(); }",
        )
        .expect("compile String isBinary");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isBinary");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_octal_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isOctal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 55) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"701234\"; return value.isOctal(); }",
        )
        .expect("compile String isOctal");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isOctal");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_printable_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPrintable(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 32) { return false; } if (code > 126) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"Zun 2026!\"; return value.isPrintable(); }",
        )
        .expect("compile String isPrintable");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isPrintable");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_punctuation_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPunctuation(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let punctuation: boolean = false; if (code >= 33) { if (code <= 47) { punctuation = true; } } if (code >= 58) { if (code <= 64) { punctuation = true; } } if (code >= 91) { if (code <= 96) { punctuation = true; } } if (code >= 123) { if (code <= 126) { punctuation = true; } } if (!punctuation) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"!?.,\"; return value.isPunctuation(); }",
        )
        .expect("compile String isPunctuation");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isPunctuation");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_control_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isControl(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code > 31) { if (code != 127) { return false; } } index = index + 1; } return true; } } function main(): boolean { let value: string = \"\\n\\t\"; return value.isControl(); }",
        )
        .expect("compile String isControl");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isControl");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_signed_integer_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedInteger(): boolean { if (this.isEmpty()) { return false; } let start: u64 = 0; let first: u32 = this.charCodeAt(0); if (first == 43) { start = 1; } if (first == 45) { start = 1; } if (start == this.length()) { return false; } let index: u64 = start; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code < 48) { return false; } if (code > 57) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"-2048\"; return value.isSignedInteger(); }",
        )
        .expect("compile String isSignedInteger");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isSignedInteger");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_signed_decimal_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSignedDecimal(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let digits: u64 = 0; let dots: u64 = 0; let first: u32 = this.charCodeAt(0); if (first == 43) { index = 1; } if (first == 45) { index = 1; } while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 46) { dots = dots + 1; if (dots > 1) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } digits = digits + 1; } index = index + 1; } return digits > 0; } } function main(): boolean { let value: string = \"-12.50\"; return value.isSignedDecimal(); }",
        )
        .expect("compile String isSignedDecimal");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isSignedDecimal");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_identifier_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isIdentifier(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code == 95) { valid = true; } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (index > 0) { if (code >= 48) { if (code <= 57) { valid = true; } } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zun_core2\"; return value.isIdentifier(); }",
        )
        .expect("compile String isIdentifier");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isIdentifier");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_title_case_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isTitleCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let expectUpper: boolean = true; let hasLetter: boolean = false; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 32) { expectUpper = true; } else { if (expectUpper) { if (code < 65) { return false; } if (code > 90) { return false; } expectUpper = false; hasLetter = true; } else { if (code >= 65) { if (code <= 90) { return false; } } if (code < 97) { return false; } if (code > 122) { return false; } } } index = index + 1; } if (!hasLetter) { return false; } return !expectUpper; } } function main(): boolean { let value: string = \"Hello World\"; return value.isTitleCase(); }",
        )
        .expect("compile String isTitleCase");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isTitleCase");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_snake_case_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isSnakeCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 95) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun_runtime2\"; return value.isSnakeCase(); }",
        )
        .expect("compile String isSnakeCase");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isSnakeCase");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_kebab_case_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isKebabCase(): boolean { if (this.isEmpty()) { return false; } let index: u64 = 0; let previousSeparator: boolean = true; while (index < this.length()) { let code: u32 = this.charCodeAt(index); if (code == 45) { if (previousSeparator) { return false; } previousSeparator = true; } else { if (code >= 97) { if (code > 122) { return false; } } else { if (code < 48) { return false; } if (code > 57) { return false; } } previousSeparator = false; } index = index + 1; } return !previousSeparator; } } function main(): boolean { let value: string = \"zun-runtime2\"; return value.isKebabCase(); }",
        )
        .expect("compile String isKebabCase");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isKebabCase");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_camel_case_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isCamelCase(): boolean { if (this.isEmpty()) { return false; } let first: u32 = this.charCodeAt(0); if (first < 97) { return false; } if (first > 122) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"zunRuntime2\"; return value.isCamelCase(); }",
        )
        .expect("compile String isCamelCase");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isCamelCase");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_is_pascal_case_source_method() {
        let module = zun_compiler::compile(
            "import { length, charCodeAt } from \"zun:string\"; class String { extern length(): u64; extern charCodeAt(index: u64): u32; isEmpty(): boolean { return this.length() == 0; } isPascalCase(): boolean { if (this.isEmpty()) { return false; } let first: u32 = this.charCodeAt(0); if (first < 65) { return false; } if (first > 90) { return false; } let index: u64 = 1; while (index < this.length()) { let code: u32 = this.charCodeAt(index); let valid: boolean = false; if (code >= 48) { if (code <= 57) { valid = true; } } if (code >= 65) { if (code <= 90) { valid = true; } } if (code >= 97) { if (code <= 122) { valid = true; } } if (!valid) { return false; } index = index + 1; } return true; } } function main(): boolean { let value: string = \"ZunRuntime2\"; return value.isPascalCase(); }",
        )
        .expect("compile String isPascalCase");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String isPascalCase");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_string_to_string_source_method() {
        let module = zun_compiler::compile(
            "class String { toString(): string { return this; } }\n\
             function main(): string { let value: string = \"zun\"; return value.toString(); }",
        )
        .expect("compile String toString");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run String toString");
        assert!(matches!(result, Value::Str(value) if value.as_ref() == "zun"));
    }

    #[test]
    fn executes_array_source_methods_with_array_receiver_binding() {
        let module = zun_compiler::compile(
            "class Array {\n\
                 contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; }\n\
             }\n\
             function main(): boolean { let values = new Array(1); values[0] = 41; return values.contains(values[0]); }",
        )
        .expect("compile Array source method");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array source method");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_array_map_with_generic_function_callback() {
        let module = zun_compiler::compile(
            "class Array { map(callback: Function): Array { let result = new Array(this.length); let index: u64 = 0; while (index < this.length) { result[index] = callback(this[index]); index = index + 1; } return result; } }\n\
             function add_one(value: any): any { return (value as number) + 1.0; }\n\
             function main(): any { let values = new Array(1); values[0] = 41.0; let mapped = values.map(add_one); return mapped[0]; }",
        )
        .expect("compile Array.map callback");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array.map callback");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_filter_with_generic_function_callback() {
        let module = zun_compiler::compile(
            "class Array { filter(callback: Predicate): Array { let result = new Array(0); let index: u64 = 0; while (index < this.length) { let value: any = this[index]; if (callback(value)) { result.push(value); } index = index + 1; } return result; } }\n\
             function is_positive(value: any): boolean { return (value as number) > 0.0; }\n\
             function main(): any { let values = new Array(2); values[0] = -1.0; values[1] = 41.0; let filtered = values.filter(is_positive); return filtered[0]; }",
        )
        .expect("compile Array.filter callback");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array.filter callback");
        assert!(matches!(result, Value::Float(41.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_reduce_with_generic_reducer_callback() {
        let module = zun_compiler::compile(
            "class Array { reduce(callback: Reducer, initial: any): any { let accumulator: any = initial; let index: u64 = 0; while (index < this.length) { accumulator = callback(accumulator, this[index]); index = index + 1; } return accumulator; } }\n\
             function add(accumulator: any, value: any): any { return (accumulator as number) + (value as number); }\n\
             function main(): any { let values = new Array(2); values[0] = 20.0; values[1] = 22.0; return values.reduce(add, 0.0); }",
        )
        .expect("compile Array.reduce callback");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array.reduce callback");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_index_of_and_is_empty_methods() {
        let module = zun_compiler::compile(
            "class Array {\n\
                 indexOf(value: any): int { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return index as int; } index = index + 1; } return -1; }\n\
                 isEmpty(): boolean { return this.length == 0; }\n\
             }\n\
             function main(): int { let values = new Array(1); values[0] = 41; if (values.isEmpty()) { return -2; } return values.indexOf(values[0]); }",
        )
        .expect("compile Array search methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array search methods");
        assert!(matches!(result, Value::Int(0, zun_il::NumKind::I32)));
    }

    #[test]
    fn executes_array_access_methods() {
        let module = zun_compiler::compile(
            "class Array { at(index: u64): any { return this[index]; } first(): any { return this[0]; } last(): any { return this[this.length - 1]; } }\n\
             function main(): number { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; return values.at(1) as number; }",
        )
        .expect("compile Array access methods");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array access methods");
        assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_reverse_source_method() {
        let module = zun_compiler::compile(
            "class Array { reverse(): Array { let left: u64 = 0; if (this.length == 0) { return this; } let right: u64 = this.length - 1; while (left < right) { let temporary: any = this[left]; this[left] = this[right]; this[right] = temporary; left = left + 1; right = right - 1; } return this; } first(): any { return this[0]; } }\n\
             function main(): number { let values = new Array(2); values[0] = 7.0; values[1] = 9.0; values.reverse(); return values.first() as number; }",
        )
        .expect("compile Array reverse");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array reverse");
        assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_fill_source_method() {
        let module = zun_compiler::compile(
            "class Array { fill(value: any): Array { let index: u64 = 0; while (index < this.length) { this[index] = value; index = index + 1; } return this; } first(): any { return this[0]; } }\n\
             function main(): number { let values = new Array(2); values[0] = 9.0; values.fill(values[0]); return values.first() as number; }",
        )
        .expect("compile Array fill");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array fill");
        assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_slice_source_method() {
        let module = zun_compiler::compile(
            "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } first(): any { return this[0]; } }\n\
             function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 7.0; values[2] = 9.0; let selected = values.slice(1, 3); return selected.first() as number; }",
        )
        .expect("compile Array slice");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array slice");
        assert!(matches!(result, Value::Float(7.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_concat_source_method() {
        let module = zun_compiler::compile(
            "class Array { concat(other: Array): Array { let result = new Array(this.length + other.length); let index: u64 = 0; while (index < this.length) { result[index] = this[index]; index = index + 1; } let otherIndex: u64 = 0; while (otherIndex < other.length) { result[index + otherIndex] = other[otherIndex]; otherIndex = otherIndex + 1; } return result; } first(): any { return this[0]; } }\n\
             function main(): number { let left = new Array(1); left[0] = 7.0; let right = new Array(1); right[0] = 9.0; let joined = left.concat(right); return joined[1] as number; }",
        )
        .expect("compile Array concat");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array concat");
        assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_copy_within_source_method() {
        let module = zun_compiler::compile(
            "class Array { slice(start: u64, end: u64): Array { let count: u64 = end - start; let result = new Array(count); let index: u64 = 0; while (index < count) { result[index] = this[start + index]; index = index + 1; } return result; } copyWithin(target: u64, start: u64, end: u64): Array { let copied = this.slice(start, end); let index: u64 = 0; while (index < copied.length) { this[target + index] = copied[index]; index = index + 1; } return this; } }\n\
             function main(): number { let values = new Array(3); values[0] = 1.0; values[1] = 7.0; values[2] = 9.0; values.copyWithin(0, 1, 3); return values[0] as number; }",
        )
        .expect("compile Array copyWithin");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array copyWithin");
        assert!(matches!(result, Value::Float(7.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_flat_source_method() {
        let module = zun_compiler::compile(
            "class Array { flat(): Array { let total: u64 = 0; let outer: u64 = 0; while (outer < this.length) { let nested: Array = this[outer] as Array; total = total + nested.length; outer = outer + 1; } let result = new Array(total); let output: u64 = 0; outer = 0; while (outer < this.length) { let nested: Array = this[outer] as Array; let inner: u64 = 0; while (inner < nested.length) { result[output] = nested[inner]; output = output + 1; inner = inner + 1; } outer = outer + 1; } return result; } }\n\
             function main(): number { let left = new Array(1); left[0] = 7.0; let right = new Array(1); right[0] = 9.0; let outer = new Array(2); outer[0] = left; outer[1] = right; let flattened = outer.flat(); return flattened[1] as number; }",
        )
        .expect("compile Array flat");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array flat");
        assert!(matches!(result, Value::Float(9.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn executes_array_includes_source_method() {
        let module = zun_compiler::compile(
            "class Array { contains(value: any): boolean { let index: u64 = 0; while (index < this.length) { if (this[index] == value) { return true; } index = index + 1; } return false; } includes(value: any): boolean { return this.contains(value); } }\n\
             function main(): boolean { let values = new Array(1); values[0] = 41; return values.includes(values[0]); }",
        )
        .expect("compile Array includes");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Array includes");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn executes_managed_regexp_methods() {
        let module = zun_compiler::compile(
            "function main(): boolean { let pattern = new RegExp(\"zun-[0-9]+\"); if (pattern.source() != \"zun-[0-9]+\") { return false; } return pattern.isMatch(\"zun-42\"); }",
        )
        .expect("compile RegExp");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run RegExp");
        assert!(matches!(result, Value::Bool(true)));
    }

    #[test]
    fn casts_any_to_a_class_with_runtime_type_check() {
        let module = zun_compiler::compile(
            "class Item {} function main(): boolean { let value: any = box_any(new Item()); let item: Item = value as Item; return true; }",
        )
        .expect("compile Any class cast");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run Any class cast");
        assert!(matches!(result, Value::Bool(true)));
    }

    struct ExternalHook;

    impl CallHook for ExternalHook {
        fn try_call(&self, _method_rid: u32, _args: &[Value]) -> Option<Value> {
            None
        }

        fn try_call_external(
            &self,
            module: &str,
            name: &str,
            args: &[Value],
        ) -> Option<Result<Value, RunError>> {
            if module == "native" && name == "answer" && args.len() == 1 {
                return Some(Ok(Value::Int(42, zun_il::NumKind::I32)));
            }
            None
        }
    }

    #[test]
    fn external_member_ref_can_be_served_by_a_host_call_hook() {
        let module = zun_compiler::compile_with_externals(
            "import { answer } from \"native\"; function main(): int { return answer(41); }",
            &[zun_sema::check::ExternalFunction {
                module: "native".into(),
                name: "answer".into(),
                params: vec![zun_sema::Ty::INT],
                return_ty: zun_sema::Ty::INT,
            }],
        )
        .expect("compile external hook call");
        let mut output = CaptureOutput::default();
        let hook = ExternalHook;
        let result = run_with(&module, &mut output, Some(&hook)).expect("run external hook call");
        assert!(matches!(result, Value::Int(42, _)));
    }

    #[test]
    fn generator_preserves_external_call_hook_across_polls() {
        let module = zun_compiler::compile_with_externals(
            "extern function answer(value: int): int from \"native\"; function* main(): int { yield answer(1); return 9; }",
            &[zun_sema::check::ExternalFunction {
                module: "native".into(),
                name: "answer".into(),
                params: vec![zun_sema::Ty::INT],
                return_ty: zun_sema::Ty::INT,
            }],
        )
        .expect("compile generator external hook call");
        let entry = module.entry_point.expect("generator entry point");
        let mut output = CaptureOutput::default();
        let hook = ExternalHook;
        let mut generator = GeneratorSession::new_with_context(
            &module,
            entry.rid(),
            Vec::new(),
            &mut output,
            Some(&hook),
            None,
        )
        .expect("create generator with host context");

        let yielded = generator
            .next_result_with_context(&mut output, Some(&hook), None)
            .expect("poll generator");
        assert!(matches!(yielded.value, Value::Int(42, zun_il::NumKind::I32)));
        assert!(!yielded.done);
    }

    #[test]
    fn tier0_loads_and_stores_static_fields() {
        let mut module = zun_module::Module::new();
        let field = module.tables.add_field_def(FieldDef {
            name: module.strings.intern("counter"), signature: zun_metadata::BlobIndex(0),
            kind: zun_il::JitKind::Num(zun_il::NumKind::I32).to_byte(), is_static: true, is_private: false,
        });
        module.tables.add_type_def(TypeDef {
            name: module.strings.intern("Globals"), namespace: zun_metadata::StringIndex(0),
            base: Token::null(TableKind::TypeDef), field_start: field.rid(), field_end: field.rid() + 1,
            method_start: 1, method_end: 2,
        });
        let body = [
            Instruction::new(Opcode::ConstI32, Operand::Imm32(41)),
            Instruction::new(Opcode::StoreStatic, Operand::Token(field)),
            Instruction::new(Opcode::LoadStatic, Operand::Token(field)),
            Instruction::simple(Opcode::Ret),
        ];
        let mut code = Vec::new();
        for instruction in body { encode(&instruction, &mut code); }
        let offset = write_body(&mut module.code, 0, 0, true,
            zun_il::JitKind::Num(zun_il::NumKind::I32).to_byte(), &[], &code);
        let method = module.tables.add_method_def(MethodDef {
            name: module.strings.intern("main"), signature: zun_metadata::BlobIndex(0),
            body_offset: offset, is_static: true,
        });
        module.entry_point = Some(method);
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run static field roundtrip");
        assert!(matches!(result, Value::Int(41, zun_il::NumKind::I32)));
    }

    #[test]
    fn source_reads_a_static_field() {
        let module = zun_compiler::compile(
            "class Math { static answer: number; } function main(): number { return Math.answer; }",
        ).expect("compile static field read");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run static field read");
        assert!(matches!(result, Value::Void));
    }

    #[test]
    fn source_writes_and_reads_a_static_field() {
        let module = zun_compiler::compile(
            "class Math { static answer: number; } function main(): number { Math.answer = 42.0; return Math.answer; }",
        ).expect("compile static field assignment");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run static field assignment");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn private_fields_lower_inside_the_declaring_class() {
        let module = zun_compiler::compile(
            "class Box { #value: number; getValue(): number { return this.#value; } setValue(v: number): number { this.#value = v; return v; } } function main(): number { let b = new Box(41.0); b.setValue(42.0); return b.getValue(); }",
        ).expect("compile private field access");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run private field access");
        assert!(matches!(result, Value::Float(42.0, zun_il::NumKind::F64)));
    }

    #[test]
    fn property_syntax_lowers_to_getter_and_setter_methods() {
        let module = zun_compiler::compile(
            "class Box { get value(): number { return 41.0; } set value(v: number): number { return v; } } function main(): number { let b = new Box(); b.value = 42.0; return b.value; }",
        ).expect("compile property accessors");
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run property accessors");
        assert!(matches!(result, Value::Float(41.0, zun_il::NumKind::F64)));
    }
}

