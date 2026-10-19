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
                Opcode::ConstF64 => stack.push(Value::Number(operand_f64(&instr.operand))),
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
                Opcode::AddF64 => bin_num(&mut stack, |a, b| Value::Number(a + b))?,
                Opcode::SubF64 => bin_num(&mut stack, |a, b| Value::Number(a - b))?,
                Opcode::MulF64 => bin_num(&mut stack, |a, b| Value::Number(a * b))?,
                Opcode::DivF64 => bin_num(&mut stack, |a, b| Value::Number(a / b))?,
                Opcode::RemF64 => bin_num(&mut stack, |a, b| Value::Number(a % b))?,
                Opcode::Concat => {
                    let b = pop(&mut stack)?;
                    let a = pop(&mut stack)?;
                    stack.push(Value::Str(format!("{}{}", a.as_str(), b.as_str()).into()));
                }
                Opcode::CmpEq => cmp(&mut stack, true)?,
                Opcode::CmpNe => cmp(&mut stack, false)?,
                Opcode::CmpLt => cmp_num(&mut stack, |a, b| a < b)?,
                Opcode::CmpLe => cmp_num(&mut stack, |a, b| a <= b)?,
                Opcode::CmpGt => cmp_num(&mut stack, |a, b| a > b)?,
                Opcode::CmpGe => cmp_num(&mut stack, |a, b| a >= b)?,
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
                // Object/field opcodes are not reachable from phase-1 ilgen yet.
                Opcode::NewObj
                | Opcode::LoadField
                | Opcode::StoreField
                | Opcode::CallVirt
                | Opcode::ConstI32
                | Opcode::ConstI64
                | Opcode::ConstNull
                | Opcode::AddI32
                | Opcode::SubI32
                | Opcode::MulI32
                | Opcode::DivI32
                | Opcode::RemI32
                | Opcode::NegI32 => {
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

fn bin_num(
    stack: &mut Vec<Value>,
    f: impl Fn(f64, f64) -> Value,
) -> Result<(), RunError> {
    let b = pop(stack)?.as_number();
    let a = pop(stack)?.as_number();
    stack.push(f(a, b));
    Ok(())
}

fn cmp_num(stack: &mut Vec<Value>, f: impl Fn(f64, f64) -> bool) -> Result<(), RunError> {
    let b = pop(stack)?.as_number();
    let a = pop(stack)?.as_number();
    stack.push(Value::Bool(f(a, b)));
    Ok(())
}

/// Structural equality for `==`/`!=` across the primitive types. `want_equal`
/// is true for `==` and false for `!=`.
fn cmp(stack: &mut Vec<Value>, want_equal: bool) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let equal = match (&a, &b) {
        (Value::Number(x), Value::Number(y)) => x == y,
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
