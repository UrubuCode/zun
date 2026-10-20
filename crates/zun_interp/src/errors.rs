use super::Value;

#[derive(Debug, Clone)]
pub enum RunError {
    NoEntryPoint,
    BadToken,
    Malformed(String),
    Native(String),
    Thrown(Value),
}

impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RunError::NoEntryPoint => write!(f, "module has no entry point"),
            RunError::BadToken => write!(f, "call did not resolve to a method"),
            RunError::Malformed(m) => write!(f, "malformed bytecode: {m}"),
            RunError::Native(m) => write!(f, "{m}"),
            RunError::Thrown(value) => write!(f, "unhandled exception: {value:?}"),
        }
    }
}

impl std::error::Error for RunError {}
