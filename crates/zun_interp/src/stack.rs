//! Stack helper errors and operations for Tier 0 execution.

use super::RunError;
use super::value::Value;
use super::value;

pub(crate) fn underflow() -> RunError {
    RunError::Malformed("stack underflow".to_string())
}

pub(crate) fn pop(stack: &mut Vec<Value>) -> Result<Value, RunError> {
    stack.pop().ok_or_else(underflow)
}

pub(crate) fn pop_n(stack: &mut Vec<Value>, n: usize) -> Result<Vec<Value>, RunError> {
    if stack.len() < n {
        return Err(underflow());
    }
    Ok(stack.split_off(stack.len() - n))
}

pub(crate) fn bin_float(
    stack: &mut Vec<Value>,
    f: impl Fn(f64, f64) -> f64,
) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.float_kind();
    let result = value::round_float(f(a.as_number(), b.as_number()), kind);
    stack.push(Value::Float(result, kind));
    Ok(())
}
