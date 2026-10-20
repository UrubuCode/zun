//! Stack helper errors and operations for Tier 0 execution.

use super::RunError;

pub(crate) fn underflow() -> RunError {
    RunError::Malformed("stack underflow".to_string())
}
