//! Stack helper errors and operations for Tier 0 execution.

use super::RunError;
use super::value::Value;

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
