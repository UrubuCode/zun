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
