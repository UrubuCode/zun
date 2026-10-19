//! Tier 0: the ZIL interpreter.
//!
//! A straightforward stack-machine loop over a method body's ZIL. It is the
//! always-correct baseline the JIT must match (the equivalence suite in a later
//! phase runs programs through both). Calls recurse through [`Interp::call`];
//! the `print` builtin writes through a pluggable [`Output`] sink so tests can
//! capture it.

pub mod value;

use value::Value;
use zun_il::{decode_one, Opcode, Operand};
use zun_metadata::TableKind;
use zun_module::{read_body, Module};
use std::rc::Rc;

/// Sink for program output (the `print` builtin).
pub trait Output {
    /// Write one already-formatted line (without the trailing newline).
    fn println(&mut self, line: &str);
}

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
}

impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RunError::NoEntryPoint => write!(f, "module has no entry point"),
            RunError::BadToken => write!(f, "call did not resolve to a method"),
            RunError::Malformed(m) => write!(f, "malformed bytecode: {m}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Run a module's entry point with no arguments.
pub fn run(module: &Module, output: &mut dyn Output) -> Result<Value, RunError> {
    let entry = module.entry_point.ok_or(RunError::NoEntryPoint)?;
    let rid = entry.rid();
    Interp { module, output }.call(rid, Vec::new())
}

struct Interp<'a> {
    module: &'a Module,
    output: &'a mut dyn Output,
}

impl Interp<'_> {
    /// Call the method at `method_rid` (1-based) with `args`, returning its
    /// result ([`Value::Void`] for a void method).
    fn call(&mut self, method_rid: u32, args: Vec<Value>) -> Result<Value, RunError> {
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
            let (instr, size) = decode_one(&code[pc..])
                .map_err(|e| RunError::Malformed(e.to_string()))?;
            let next = pc + size;
            pc = next;

            match instr.opcode {
                Opcode::Nop => {}
                Opcode::ConstF64 => {
                    stack.push(Value::Float(operand_f64(&instr.operand), zun_il::NumKind::F64))
                }
                Opcode::ConstStr => {
                    let idx = operand_strref(&instr.operand);
                    let s: Rc<str> = self.module.strings.get(zun_metadata::StringIndex(idx)).into();
                    stack.push(Value::Str(s));
                }
                Opcode::ConstTrue => stack.push(Value::Bool(true)),
                Opcode::ConstFalse => stack.push(Value::Bool(false)),
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
                Opcode::ConstI32 => {
                    stack.push(Value::Int(operand_i32(&instr.operand) as i64, zun_il::NumKind::I32))
                }
                Opcode::ConstI64 => {
                    stack.push(Value::Int(operand_i64(&instr.operand), zun_il::NumKind::I64))
                }
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
                    pc = branch_target(next, &instr.operand);
                }
                Opcode::BranchTrue => {
                    if pop(&mut stack)?.as_bool() {
                        pc = branch_target(next, &instr.operand);
                    }
                }
                Opcode::BranchFalse => {
                    if !pop(&mut stack)?.as_bool() {
                        pc = branch_target(next, &instr.operand);
                    }
                }
                Opcode::Ret => {
                    return Ok(stack.pop().unwrap_or(Value::Void));
                }
                Opcode::Call => {
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
                    if callee.body_offset == u32::MAX {
                        return Err(RunError::BadToken);
                    }
                    let callee_body = read_body(&self.module.code, callee.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    let args = pop_n(&mut stack, callee_body.param_count as usize)?;
                    let result = self.call(callee_rid, args)?;
                    stack.push(result);
                }
                Opcode::CallBuiltin => {
                    let id = operand_i32(&instr.operand);
                    self.call_builtin(id, &mut stack)?;
                }
                Opcode::NewObj => {
                    let token = operand_token(&instr.operand);
                    let (field_start, field_count) = self.type_field_range(token)?;
                    let _ = field_start;
                    let args = pop_n(&mut stack, field_count)?;
                    stack.push(Value::Object(std::rc::Rc::new(value::Object {
                        type_rid: token.rid(),
                        fields: std::cell::RefCell::new(args),
                    })));
                }
                Opcode::LoadField => {
                    let slot = self.field_slot(operand_token(&instr.operand))?;
                    let obj = pop(&mut stack)?;
                    let v = obj.as_object().fields.borrow()[slot].clone();
                    stack.push(v);
                }
                Opcode::StoreField => {
                    let slot = self.field_slot(operand_token(&instr.operand))?;
                    let value = pop(&mut stack)?;
                    let obj = pop(&mut stack)?;
                    obj.as_object().fields.borrow_mut()[slot] = value.clone();
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
                    let name = self.module.strings.get(static_method.name).to_string();
                    let static_body = read_body(&self.module.code, static_method.body_offset)
                        .map_err(|e| RunError::Malformed(e.to_string()))?;
                    let args = pop_n(&mut stack, static_body.param_count as usize)?;
                    // Dispatch on the receiver's runtime class.
                    let type_rid = args[0].as_object().type_rid;
                    let target = self.resolve_virtual(type_rid, &name)?;
                    let result = self.call(target, args)?;
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

    /// For a FieldDef token, return its zero-based slot within the class that
    /// owns it (the class whose field range contains this field's RID).
    fn field_slot(&self, field_token: zun_metadata::Token) -> Result<usize, RunError> {
        let field_rid = field_token.rid();
        let owner = self
            .module
            .tables
            .type_defs
            .iter()
            .find(|t| field_rid >= t.field_start && field_rid < t.field_end)
            .ok_or(RunError::BadToken)?;
        Ok((field_rid - owner.field_start) as usize)
    }

    /// Resolve a virtual call: find the most-derived method named `name`
    /// starting at the runtime class `type_rid` and walking up to its bases.
    /// Returns the target method's RID.
    fn resolve_virtual(&self, mut type_rid: u32, name: &str) -> Result<u32, RunError> {
        loop {
            let ty = self
                .module
                .tables
                .type_defs
                .get(type_rid as usize - 1)
                .ok_or(RunError::BadToken)?;
            for rid in ty.method_start..ty.method_end {
                let m = &self.module.tables.method_defs[rid as usize - 1];
                if self.module.strings.get(m.name) == name {
                    return Ok(rid);
                }
            }
            if ty.base.is_null() {
                return Err(RunError::BadToken);
            }
            type_rid = ty.base.rid();
        }
    }

    fn call_builtin(&mut self, id: i32, stack: &mut Vec<Value>) -> Result<(), RunError> {
        match id {
            0 => {
                // print(value): pop one argument, show it, push void.
                let arg = pop(stack)?;
                self.output.println(&arg.display());
                stack.push(Value::Void);
                Ok(())
            }
            other => Err(RunError::Malformed(format!("unknown builtin {other}"))),
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
    stack.push(Value::Int(value::wrap_int(f(a.as_int(), b.as_int()), kind), kind));
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
        if is_div { x.wrapping_div(y) } else { x.wrapping_rem(y) }
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
        _ => return Err(RunError::Malformed("comparison of mismatched numeric kinds".into())),
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
