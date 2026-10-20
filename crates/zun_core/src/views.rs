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

impl DataView {
    pub fn new(buffer_length: usize, offset: usize, length: usize) -> Option<Self> {
        Some(Self {
            view: ByteView::new(buffer_length, offset, length)?,
        })
    }

    pub fn offset(self) -> usize {
        self.view.offset()
    }

    pub fn len(self) -> usize {
        self.view.len()
    }

    pub(crate) fn bytes(self, relative: usize, width: usize) -> Option<usize> {
        relative
            .checked_add(width)
            .filter(|&end| end <= self.len())
            .map(|_| self.offset() + relative)
    }

    pub fn get_u8(self, buffer: &ByteBuffer, relative: usize) -> Option<u8> {
        buffer.get(self.bytes(relative, 1)?)
    }

    pub fn set_u8(&self, buffer: &mut ByteBuffer, relative: usize, value: u8) -> bool {
        (*self)
            .bytes(relative, 1)
            .is_some_and(|offset| buffer.set(offset, value))
    }

    pub fn get_u16(self, buffer: &ByteBuffer, relative: usize, little_endian: bool) -> Option<u16> {
        let offset = self.bytes(relative, 2)?;
        let bytes = [buffer.get(offset)?, buffer.get(offset + 1)?];
        Some(if little_endian {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    }

    pub fn set_u16(
        &self,
        buffer: &mut ByteBuffer,
        relative: usize,
        value: u16,
        little_endian: bool,
    ) -> bool {
        let Some(offset) = (*self).bytes(relative, 2) else {
            return false;
        };
        let bytes = if little_endian {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        buffer.set(offset, bytes[0]) && buffer.set(offset + 1, bytes[1])
    }

    pub fn get_u32(self, buffer: &ByteBuffer, relative: usize, little_endian: bool) -> Option<u32> {
        let offset = self.bytes(relative, 4)?;
        let bytes = [
            buffer.get(offset)?,
            buffer.get(offset + 1)?,
            buffer.get(offset + 2)?,
            buffer.get(offset + 3)?,
        ];
        Some(if little_endian {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }

    pub fn set_u32(
        &self,
        buffer: &mut ByteBuffer,
        relative: usize,
        value: u32,
        little_endian: bool,
    ) -> bool {
        let Some(offset) = (*self).bytes(relative, 4) else {
            return false;
        };
        let bytes = if little_endian {
            value.to_le_bytes()
        } else {
            value.to_be_bytes()
        };
        bytes
            .into_iter()
            .enumerate()
            .all(|(index, byte)| buffer.set(offset + index, byte))
    }

    pub fn get_f64(self, buffer: &ByteBuffer, relative: usize, little_endian: bool) -> Option<f64> {
        let offset = self.bytes(relative, 8)?;
        let mut bytes = [0; 8];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = buffer.get(offset + index)?;
        }
        Some(f64::from_bits(if little_endian {
            u64::from_le_bytes(bytes)
        } else {
            u64::from_be_bytes(bytes)
        }))
    }

    pub fn set_f64(
        &self,
        buffer: &mut ByteBuffer,
        relative: usize,
        value: f64,
        little_endian: bool,
    ) -> bool {
        let Some(offset) = (*self).bytes(relative, 8) else {
            return false;
        };
        let bytes = if little_endian {
            value.to_bits().to_le_bytes()
        } else {
            value.to_bits().to_be_bytes()
        };
        bytes
            .into_iter()
            .enumerate()
            .all(|(index, byte)| buffer.set(offset + index, byte))
    }
}

impl TypedArrayView {
    pub fn new(
        buffer_length: usize,
        offset: usize,
        length: usize,
        element: TypedElement,
    ) -> Option<Self> {
        let view = ByteView::new(buffer_length, offset, length)?;
        (offset % element.size() == 0 && length % element.size() == 0)
            .then_some(Self { view, element })
    }

    pub fn len(self) -> usize {
        self.view.len() / self.element.size()
    }

    pub fn element(self) -> TypedElement {
        self.element
    }

    pub fn read_u8(self, buffer: &ByteBuffer, index: usize) -> Option<u8> {
        (self.element == TypedElement::U8)
            .then(|| buffer.get(self.view.offset + index))
            .flatten()
    }

    pub fn write_u8(self, buffer: &mut ByteBuffer, index: usize, value: u8) -> bool {
        self.element == TypedElement::U8
            && index < self.len()
            && buffer.set(self.view.offset + index, value)
    }
}
