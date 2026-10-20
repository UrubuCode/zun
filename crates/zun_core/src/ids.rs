#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct IntrinsicId(pub u32);

impl IntrinsicId {
    pub const fn new(raw: u32) -> Self { Self(raw) }
    pub const fn get(self) -> u32 { self.0 }
}

/// Stable positional identity for the runtime's built-in types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CoreTypeId {
    Object = 0,
    String = 1,
    Number = 2,
    Boolean = 3,
    Array = 4,
    Function = 5,
    /// Managed Promise state-machine result object.
    Promise = 6,
}

impl CoreTypeId {
    pub const fn new(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(Self::Object),
            1 => Some(Self::String),
            2 => Some(Self::Number),
            3 => Some(Self::Boolean),
            4 => Some(Self::Array),
            5 => Some(Self::Function),
            6 => Some(Self::Promise),
            _ => None,
        }
    }
    pub const fn get(self) -> u8 { self as u8 }
}
