//! Tier 0: the ZIL interpreter.
//!
//! A straightforward stack-machine loop over a method body's ZIL. It is the
//! always-correct baseline the JIT must match (the equivalence suite in a later
//! phase runs programs through both). Calls recurse through [`Interp::call`];
//! the `print` builtin writes through a pluggable [`Output`] sink so tests can
//! capture it.

pub mod value;

use std::rc::Rc;
use value::Value;
use zun_il::{decode_one, Opcode, Operand};
use zun_metadata::TableKind;
use zun_module::{read_body, Module};

/// Sink for program output: the native modules' [`Sink`](zun_natives::Sink),
/// re-exported so callers keep using `Output`.
pub use zun_natives::Sink as Output;

/// An [`Output`] that collects lines in a vector, for tests.
#[derive(Debug, Default)]
pub struct CaptureOutput {
    /// Lines written so far.
    pub lines: Vec<String>,
}

impl Output for CaptureOutput {
    fn println(&mut self, line: &str) {
        self.lines.push(line.to_string());
    }
}

/// An [`Output`] that writes to real stdout.
#[derive(Debug, Default)]
pub struct StdoutOutput;

impl Output for StdoutOutput {
    fn println(&mut self, line: &str) {
        println!("{line}");
    }
}

/// Errors the interpreter can raise at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The module has no entry point to run.
    NoEntryPoint,
    /// A call or token did not resolve to a method body.
    BadToken,
    /// The bytecode stream was malformed (should not happen for verified code).
    Malformed(String),
    /// A runtime intrinsic (e.g. a `zun:fs` operation) failed.
    Native(String),
}

impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RunError::NoEntryPoint => write!(f, "module has no entry point"),
            RunError::BadToken => write!(f, "call did not resolve to a method"),
            RunError::Malformed(m) => write!(f, "malformed bytecode: {m}"),
            RunError::Native(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for RunError {}

/// A hook the interpreter consults before interpreting a call, so a faster
/// backend (the JIT) can execute an eligible method instead. Returning `None`
/// means "not handled — interpret it".
pub trait CallHook {
    /// Try to execute method `method_rid` with `args`.
    fn try_call(&self, method_rid: u32, args: &[Value]) -> Option<Value>;

    /// Observe a backwards branch in an interpreted method. Backedges are the
    /// stable safepoint boundary used by future OSR implementations.
    fn on_backedge(&self, _method_rid: u32, _target_pc: usize) {}
}

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

fn run_method_with(
    module: &Module,
    method_rid: u32,
    args: Vec<Value>,
    output: &mut dyn Output,
    hook: Option<&dyn CallHook>,
    loader: Option<&zun_loader::ModuleLoader>,
) -> Result<Value, RunError> {
    // Own the heap for the run and register it as the thread's current heap so
    // the interpreter and any JIT code allocate into the same one.
    let mut heap = zun_gc::Heap::new();
    let _scope = unsafe { zun_gc::HeapScope::enter(&mut heap as *mut _) };
    let types = zun_types::TypeSystem::from_module(module).map_err(|_| RunError::BadToken)?;
    Interp {
        module,
        output,
        hook,
        loader,
        allocs: 0,
        types,
    }
    .call(method_rid, args, None)
}

/// How many object allocations trigger a collection attempt.
const GC_INTERVAL: u32 = 256;

struct Interp<'a> {
    module: &'a Module,
    output: &'a mut dyn Output,
    hook: Option<&'a dyn CallHook>,
    loader: Option<&'a zun_loader::ModuleLoader>,
    /// Object allocations since the last collection (GC trigger counter).
    allocs: u32,
    types: zun_types::TypeSystem,
}

/// A link in the chain of suspended caller frames, used to find precise GC
/// roots. Each active interpreter frame contributes the object references on
/// its operand stack and in its locals; a frame is frozen (its stack/locals
/// unchanged) while a callee runs, so these borrows are valid at a safepoint.
struct RootLink<'a> {
    stack: &'a [Value],
    locals: &'a [Value],
    parent: Option<&'a RootLink<'a>>,
}

impl Interp<'_> {
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

        let mut locals: Vec<Value> = Vec::with_capacity(body.local_count as usize);
        locals.extend(args);
        locals.resize(body.local_count as usize, Value::Void);

        let mut stack: Vec<Value> = Vec::new();
        let code = body.code;
        let mut pc = 0usize;

        while pc < code.len() {
            let (instr, size) =
                decode_one(&code[pc..]).map_err(|e| RunError::Malformed(e.to_string()))?;
            let next = pc + size;
            pc = next;

            match instr.opcode {
                Opcode::Nop => {}
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
                Opcode::BoxAny => {
                    let value = pop(&mut stack)?;
                    let any = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    stack.push(Value::Any(any));
                }
                Opcode::UnboxAny => {
                    let value = pop(&mut stack)?;
                    if let Value::Any(any) = value {
                        let value = zun_gc::with_current_heap(|heap| value::Value::unbox_any(any, heap))
                            .ok_or(RunError::BadToken)?;
                        stack.push(value);
                    } else {
                        // Array slots retain the tagged runtime value directly;
                        // an explicit numeric cast must accept that representation
                        // as well as a heap-backed `Any` box.
                        stack.push(value);
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
                        heap.dynamic_get(object, name).map(Value::Any).unwrap_or(Value::Void)
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
                Opcode::NewArray => {
                    let length = pop(&mut stack)?.as_int();
                    if length < 0 {
                        return Err(RunError::Malformed("negative array length".into()));
                    }
                    let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(length as usize));
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
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(array, index as usize, any))
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
                        heap.array_pop(array).and_then(|any| value::Value::unbox_any(any, heap))
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
                    if target < pc {
                        if let Some(hook) = self.hook {
                            hook.on_backedge(method_rid, target);
                        }
                    }
                    pc = target;
                }
                Opcode::BranchTrue => {
                    if pop(&mut stack)?.as_bool() {
                        let target = branch_target(next, &instr.operand);
                        if target < pc {
                            if let Some(hook) = self.hook {
                                hook.on_backedge(method_rid, target);
                            }
                        }
                        pc = target;
                    }
                }
                Opcode::BranchFalse => {
                    if !pop(&mut stack)?.as_bool() {
                        let target = branch_target(next, &instr.operand);
                        if target < pc {
                            if let Some(hook) = self.hook {
                                hook.on_backedge(method_rid, target);
                            }
                        }
                        pc = target;
                    }
                }
                Opcode::Ret => {
                    return Ok(stack.pop().unwrap_or(Value::Void));
                }
                Opcode::Call => {
                    let token = operand_token(&instr.operand);
                    if token.table() == Some(TableKind::MemberRef) {
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
                        let result = run_method_loaded(
                            resolved.scope,
                            loader,
                            resolved.token.rid(),
                            args,
                            self.output,
                        )?;
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
                    let result = self.dispatch(callee_rid, args, Some(&link))?;
                    stack.push(result);
                }
                Opcode::CallBuiltin => {
                    let id = operand_i32(&instr.operand);
                    self.call_builtin(id, &mut stack)?;
                }
                Opcode::NewObj => {
                    // Safepoint: only the interpreter triggers GC, and only
                    // here, where every live object reference is reachable from
                    // the current frame plus the ancestor root chain (no JIT
                    // frame is executing at this point).
                    self.allocs += 1;
                    if self.allocs >= GC_INTERVAL {
                        self.allocs = 0;
                        self.collect(&stack, &locals, parent);
                    }
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
                            obj.set_field(i, raw);
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
                    let raw =
                        zun_gc::with_current_heap_mut(|h| value::encode_field(kind, &value, h));
                    obj.set_field(slot, raw);
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
                    let primitive_receiver = matches!(args.first(), Some(Value::Str(_) | Value::Float(_, _)))
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
                    let result = self.dispatch(target, args, Some(&link))?;
                    stack.push(result);
                }
                // Remaining opcodes are not produced by the front-end yet.
                Opcode::ConstNull => {
                    return Err(RunError::Malformed(format!(
                        "opcode {:?} not supported by the phase-1 interpreter",
                        instr.opcode
                    )));
                }
            }
        }
        // Falling off the end with no `ret` returns void; ilgen always appends
        // a trailing `ret`, so this is a safety net.
        Ok(Value::Void)
    }

    /// For a TypeDef token, return `(field_start_rid, field_count)`.
    fn type_field_range(&self, token: zun_metadata::Token) -> Result<(u32, usize), RunError> {
        let ty = self
            .module
            .tables
            .type_defs
            .get(token.rid() as usize - 1)
            .ok_or(RunError::BadToken)?;
        Ok((ty.field_start, (ty.field_end - ty.field_start) as usize))
    }

    /// The value-type code (`FieldDef.kind`) of the field at 1-based `field_rid`.
    fn field_kind_at(&self, field_rid: u32) -> Result<u8, RunError> {
        self.module
            .tables
            .field_defs
            .get(field_rid as usize - 1)
            .map(|f| f.kind)
            .ok_or(RunError::BadToken)
    }

    fn dynamic_name(&self, operand: &Operand) -> Result<&str, RunError> {
        let index = operand_strref(operand);
        Ok(self.module.strings.get(zun_metadata::StringIndex(index)))
    }

    /// Execute a call: route it to the JIT hook when it handles the method,
    /// otherwise interpret it.
    fn dispatch(
        &mut self,
        method_rid: u32,
        args: Vec<Value>,
        parent: Option<&RootLink>,
    ) -> Result<Value, RunError> {
        if let Some(hook) = self.hook {
            if let Some(result) = hook.try_call(method_rid, &args) {
                return Ok(result);
            }
        }
        self.call(method_rid, args, parent)
    }

    /// Run a mark-sweep collection with precise roots: the object references in
    /// the current frame's stack and locals, plus every ancestor frame in the
    /// `parent` chain. Reference fields are traced via the module's type layout.
    fn collect(&mut self, stack: &[Value], locals: &[Value], parent: Option<&RootLink>) {
        let mut roots: Vec<zun_gc::Obj> = Vec::new();
        collect_frame_roots(stack, locals, &mut roots);
        let mut link = parent;
        while let Some(l) = link {
            collect_frame_roots(l.stack, l.locals, &mut roots);
            link = l.parent;
        }
        zun_gc::append_native_roots(&mut roots);
        let types = &self.types;
        let ref_fn = |type_rid: u32| types.reference_slots(type_rid);
        // SAFETY: `roots` holds every live object reference reachable by the
        // running program at this safepoint (no JIT frame is active here), so
        // only genuinely-unreachable objects are reclaimed.
        zun_gc::with_current_heap_mut(|h| unsafe {
            h.collect(&roots, &ref_fn);
        });
    }

    fn call_builtin(&mut self, id: i32, stack: &mut Vec<Value>) -> Result<(), RunError> {
        let id = id as u32;
        // Pop the arguments, convert them to neutral native values, run the
        // native body (in zun_natives), then convert the result back.
        let args = pop_n(stack, zun_natives::param_count(id))?;
        let native_args: Vec<zun_natives::NativeArg> = args.iter().map(to_native_arg).collect();
        let result = zun_natives::dispatch(id, &native_args, self.output)
            .map_err(|e| RunError::Native(e.0))?;
        stack.push(from_native_value(result, id));
        Ok(())
    }
}

/// Borrow a `Value` as a neutral [`NativeArg`](zun_natives::NativeArg).
fn to_native_arg(v: &Value) -> zun_natives::NativeArg<'_> {
    match v {
        Value::Int(n, _) => zun_natives::NativeArg::Int(*n),
        Value::Float(n, _) => zun_natives::NativeArg::Float(*n),
        Value::Bool(b) => zun_natives::NativeArg::Bool(*b),
        Value::Str(s) => zun_natives::NativeArg::Str(s),
        Value::Object(_) => zun_natives::NativeArg::Opaque,
        Value::Any(_) => zun_natives::NativeArg::Opaque,
        Value::Void => zun_natives::NativeArg::Void,
    }
}

/// Convert a native result back to a `Value`, using the function's declared
/// return type (via `id`) to pick the integer width.
fn from_native_value(v: zun_natives::NativeValue, id: u32) -> Value {
    use zun_natives::{NativeTy, NativeValue};
    match v {
        NativeValue::Void => Value::Void,
        NativeValue::Bool(b) => Value::Bool(b),
        NativeValue::Str(s) => Value::Str(s.into()),
        NativeValue::Float(n) => {
            let kind = match zun_natives::by_id(id).map(|f| f.ret) {
                Some(NativeTy::F32) => zun_il::NumKind::F32,
                _ => zun_il::NumKind::F64,
            };
            Value::Float(value::round_float(n, kind), kind)
        }
        NativeValue::Int(n) => {
            use zun_il::NumKind as K;
            let kind = match zun_natives::by_id(id).map(|f| f.ret) {
                Some(NativeTy::I8) => K::I8,
                Some(NativeTy::I16) => K::I16,
                Some(NativeTy::I64) => K::I64,
                Some(NativeTy::U8) => K::U8,
                Some(NativeTy::U16) => K::U16,
                Some(NativeTy::U32) => K::U32,
                Some(NativeTy::U64) => K::U64,
                _ => K::I32,
            };
            Value::Int(value::wrap_int(n, kind), kind)
        }
    }
}

#[cfg(test)]
mod method_tests {
    use super::*;
    use zun_il::{encode, Instruction, Operand};
    use zun_metadata::{MethodDef, TableKind, Token};
    use zun_module::write_body;

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
        });
        module.entry_point = Some(Token::new(TableKind::MethodDef, 1));
        let mut output = CaptureOutput::default();
        let result = run(&module, &mut output).expect("run array IL");
        assert!(matches!(result, Value::Int(41, zun_il::NumKind::I64)));
    }

    #[test]
    fn executes_source_array_constructor() {
        let module = zun_compiler::compile(
            "function main(): Array { return new Array(2); }",
        )
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
        assert!(matches!(result, Value::Float(41.0, zun_il::NumKind::F64)), "{result:?}");
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
}

/// Add the object references found in a frame's stack and locals to `roots`.
fn collect_frame_roots(stack: &[Value], locals: &[Value], roots: &mut Vec<zun_gc::Obj>) {
    for v in stack.iter().chain(locals.iter()) {
        if let Value::Object(o) = v {
            roots.push(*o);
        }
    }
}

fn underflow() -> RunError {
    RunError::Malformed("stack underflow".to_string())
}

fn pop(stack: &mut Vec<Value>) -> Result<Value, RunError> {
    stack.pop().ok_or_else(underflow)
}

fn pop_n(stack: &mut Vec<Value>, n: usize) -> Result<Vec<Value>, RunError> {
    if stack.len() < n {
        return Err(underflow());
    }
    Ok(stack.split_off(stack.len() - n))
}

fn bin_float(stack: &mut Vec<Value>, f: impl Fn(f64, f64) -> f64) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.float_kind();
    let r = value::round_float(f(a.as_number(), b.as_number()), kind);
    stack.push(Value::Float(r, kind));
    Ok(())
}

fn bin_int(stack: &mut Vec<Value>, f: impl Fn(i64, i64) -> i64) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.int_kind();
    stack.push(Value::Int(
        value::wrap_int(f(a.as_int(), b.as_int()), kind),
        kind,
    ));
    Ok(())
}

/// Integer divide (`is_div`) or remainder, choosing signed vs unsigned by the
/// operand kind so `u32` division behaves unsigned.
fn bin_int_signed_aware(stack: &mut Vec<Value>, is_div: bool) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.int_kind();
    let (_, signed) = kind.int_parts().expect("int kind");
    let (x, y) = (a.as_int(), b.as_int());
    if y == 0 {
        return Err(RunError::Malformed("integer division by zero".into()));
    }
    let r = if signed {
        if is_div {
            x.wrapping_div(y)
        } else {
            x.wrapping_rem(y)
        }
    } else {
        let (ux, uy) = (x as u64, y as u64);
        let ur = if is_div { ux / uy } else { ux % uy };
        ur as i64
    };
    stack.push(Value::Int(value::wrap_int(r, kind), kind));
    Ok(())
}

/// Convert a numeric value to `target`, following the lattice rules: int↔int
/// re-wraps, float→int truncates toward zero (saturating is a JIT concern), and
/// int→float / float→float rounds.
fn convert(v: &Value, target: zun_il::NumKind) -> Result<Value, RunError> {
    use zun_il::NumKind;
    let as_f64 = match v {
        Value::Int(n, _) => *n as f64,
        Value::Float(n, _) => *n,
        other => return Err(RunError::Malformed(format!("cannot convert {other:?}"))),
    };
    Ok(if target.is_int() {
        let truncated = match v {
            Value::Int(n, _) => *n,
            Value::Float(n, _) => *n as i64,
            _ => unreachable!(),
        };
        Value::Int(value::wrap_int(truncated, target), target)
    } else {
        match target {
            NumKind::F32 => Value::Float(as_f64 as f32 as f64, NumKind::F32),
            _ => Value::Float(as_f64, NumKind::F64),
        }
    })
}

/// A relational comparison operator.
#[derive(Clone, Copy)]
enum Ordering {
    Lt,
    Le,
    Gt,
    Ge,
}

impl Ordering {
    fn apply<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            Ordering::Lt => a < b,
            Ordering::Le => a <= b,
            Ordering::Gt => a > b,
            Ordering::Ge => a >= b,
        }
    }
}

/// Ordered comparison over two ints (signed or unsigned by kind) or two floats;
/// sema guarantees the operands share a numeric kind.
fn compare(stack: &mut Vec<Value>, op: Ordering) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let result = match (&a, &b) {
        (Value::Int(x, k), Value::Int(y, _)) => {
            let (_, signed) = k.int_parts().expect("int kind");
            if signed {
                op.apply(*x, *y)
            } else {
                op.apply(*x as u64, *y as u64)
            }
        }
        (Value::Float(x, _), Value::Float(y, _)) => op.apply(*x, *y),
        _ => {
            return Err(RunError::Malformed(
                "comparison of mismatched numeric kinds".into(),
            ))
        }
    };
    stack.push(Value::Bool(result));
    Ok(())
}

/// Structural equality for `==`/`!=` across the primitive types. `want_equal`
/// is true for `==` and false for `!=`.
fn cmp(stack: &mut Vec<Value>, want_equal: bool) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let equal = match (&a, &b) {
        (Value::Float(x, _), Value::Float(y, _)) => x == y,
        (Value::Int(x, _), Value::Int(y, _)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        // Reference identity: two object refs are equal iff the same object.
        (Value::Object(x), Value::Object(y)) => x.addr() == y.addr(),
        _ => false,
    };
    stack.push(Value::Bool(equal == want_equal));
    Ok(())
}

fn branch_target(instr_end: usize, operand: &Operand) -> usize {
    let disp = match operand {
        Operand::Branch(d) => *d,
        _ => 0,
    };
    (instr_end as i64 + disp as i64) as usize
}

fn operand_f64(o: &Operand) -> f64 {
    match o {
        Operand::Float64(v) => *v,
        _ => 0.0,
    }
}
fn operand_i64(o: &Operand) -> i64 {
    match o {
        Operand::Imm64(v) => *v,
        _ => 0,
    }
}
fn operand_i32(o: &Operand) -> i32 {
    match o {
        Operand::Imm32(v) => *v,
        _ => 0,
    }
}
fn operand_local(o: &Operand) -> usize {
    match o {
        Operand::Local(v) => *v as usize,
        _ => 0,
    }
}
fn operand_strref(o: &Operand) -> u32 {
    match o {
        Operand::StringRef(v) => *v,
        _ => 0,
    }
}
fn operand_token(o: &Operand) -> zun_metadata::Token {
    match o {
        Operand::Token(t) => *t,
        _ => zun_metadata::Token::from_raw(0),
    }
}
