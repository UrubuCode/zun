//! Byte, data, and typed-array views over the core byte buffer.

/// Bounds-checked view into a shared byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteView {
    pub(crate) offset: usize,
    pub(crate) length: usize,
}

/// Unaligned, endian-aware view into a shared byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataView {
    pub(crate) view: ByteView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedElement {
    U8,
    U16,
    U32,
    F64,
}

impl TypedElement {
    pub const fn size(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 => 4,
            Self::F64 => 8,
        }
    }
}
