//! Byte, data, and typed-array views over the core byte buffer.

use super::ByteBuffer;

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

/// Typed-array view metadata over a bounded byte window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypedArrayView {
    pub(crate) view: ByteView,
    pub(crate) element: TypedElement,
}

impl ByteView {
    pub fn new(buffer_length: usize, offset: usize, length: usize) -> Option<Self> {
        offset
            .checked_add(length)
            .filter(|&end| end <= buffer_length)
            .map(|_| Self { offset, length })
    }

    pub fn offset(self) -> usize {
        self.offset
    }
    pub fn len(self) -> usize {
        self.length
    }

    pub fn read_u16(self, buffer: &ByteBuffer, relative: usize) -> Option<u16> {
        let end = relative.checked_add(2)?;
        (end <= self.length)
            .then(|| buffer.read_u16(self.offset + relative))
            .flatten()
    }

    pub fn write_u16(self, buffer: &mut ByteBuffer, relative: usize, value: u16) -> bool {
        relative.checked_add(2).is_some_and(|end| {
            end <= self.length && buffer.write_u16(self.offset + relative, value)
        })
    }

    pub fn read_u32(self, buffer: &ByteBuffer, relative: usize) -> Option<u32> {
        let end = relative.checked_add(4)?;
        if end > self.length {
            return None;
        }
        buffer.read_u32(self.offset + relative)
    }

    pub fn write_u32(self, buffer: &mut ByteBuffer, relative: usize, value: u32) -> bool {
        let Some(end) = relative.checked_add(4) else {
            return false;
        };
        end <= self.length && buffer.write_u32(self.offset + relative, value)
    }
}
