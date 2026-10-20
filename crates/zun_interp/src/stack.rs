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

pub(crate) fn bin_int(
    stack: &mut Vec<Value>,
    f: impl Fn(i64, i64) -> i64,
) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.int_kind();
    stack.push(Value::Int(
        value::wrap_int(f(a.as_int(), b.as_int()), kind),
        kind,
    ));
    Ok(())
}

/// Integer divide or remainder, choosing signed versus unsigned by kind.
pub(crate) fn bin_int_signed_aware(
    stack: &mut Vec<Value>,
    is_div: bool,
) -> Result<(), RunError> {
    let b = pop(stack)?;
    let a = pop(stack)?;
    let kind = a.int_kind();
    let (_, signed) = kind.int_parts().expect("int kind");
    let (x, y) = (a.as_int(), b.as_int());
    if y == 0 {
        return Err(RunError::Malformed("integer division by zero".into()));
    }
    let result = if signed {
        if is_div {
            x.wrapping_div(y)
        } else {
            x.wrapping_rem(y)
        }
    } else {
        let (ux, uy) = (x as u64, y as u64);
        let unsigned_result = if is_div { ux / uy } else { ux % uy };
        unsigned_result as i64
    };
    stack.push(Value::Int(value::wrap_int(result, kind), kind));
    Ok(())
}
