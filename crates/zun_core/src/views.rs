//! Byte, data, and typed-array views over the core byte buffer.

/// Bounds-checked view into a shared byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteView {
    pub(crate) offset: usize,
    pub(crate) length: usize,
}
