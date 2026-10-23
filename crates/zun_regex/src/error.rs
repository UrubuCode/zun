//! Error types for pattern parsing and matching.

use std::fmt;

/// A syntax error in a regex pattern or flag string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegexParseError {
    /// Human-readable description.
    pub message: String,
    /// Char index into the pattern where the error was detected.
    pub position: usize,
}

impl RegexParseError {
    pub(crate) fn new(message: impl Into<String>, position: usize) -> Self {
        RegexParseError { message: message.into(), position }
    }
}

impl fmt::Display for RegexParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid regular expression: {} (at {})", self.message, self.position)
    }
}

impl std::error::Error for RegexParseError {}

/// A runtime matching error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegexError {
    /// The matcher exhausted its step budget (catastrophic backtracking guard).
    StepLimitExceeded,
}

impl fmt::Display for RegexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegexError::StepLimitExceeded => write!(f, "regex step limit exceeded"),
        }
    }
}

impl std::error::Error for RegexError {}
