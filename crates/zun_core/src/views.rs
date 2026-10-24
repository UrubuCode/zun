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

    // --- the remaining element widths ---
    //
    // Reading and writing `N` bytes with a choice of endianness is the same
    // operation at every width, so it is written once here and the typed
    // accessors below reinterpret the bits. Repeating the byte loop per type
    // is how the four above ended up with three slightly different shapes.
    //
    // All of these are *bit* reinterpretations, never numeric conversions:
    // `get_i8` on `0xFF` is `-1`, not `255`.

    fn get_bytes<const N: usize>(
        self,
        buffer: &ByteBuffer,
        relative: usize,
        little_endian: bool,
    ) -> Option<[u8; N]> {
        let offset = self.bytes(relative, N)?;
        let mut bytes = [0u8; N];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = buffer.get(offset + index)?;
        }
        if !little_endian {
            bytes.reverse();
        }
        Some(bytes)
    }

    fn set_bytes<const N: usize>(
        self,
        buffer: &mut ByteBuffer,
        relative: usize,
        mut bytes: [u8; N],
        little_endian: bool,
    ) -> bool {
        let Some(offset) = self.bytes(relative, N) else {
            return false;
        };
        if !little_endian {
            bytes.reverse();
        }
        bytes
            .into_iter()
            .enumerate()
            .all(|(index, byte)| buffer.set(offset + index, byte))
    }

    pub fn get_i8(self, buffer: &ByteBuffer, relative: usize) -> Option<i8> {
        self.get_u8(buffer, relative).map(|value| value as i8)
    }

    pub fn set_i8(self, buffer: &mut ByteBuffer, relative: usize, value: i8) -> bool {
        self.set_u8(buffer, relative, value as u8)
    }

    pub fn get_i16(self, buffer: &ByteBuffer, relative: usize, little_endian: bool) -> Option<i16> {
        Some(i16::from_le_bytes(self.get_bytes::<2>(
            buffer,
            relative,
            little_endian,
        )?))
    }

    pub fn set_i16(
        self,
        buffer: &mut ByteBuffer,
        relative: usize,
        value: i16,
        little_endian: bool,
    ) -> bool {
        self.set_bytes::<2>(buffer, relative, value.to_le_bytes(), little_endian)
    }

    pub fn get_i32(self, buffer: &ByteBuffer, relative: usize, little_endian: bool) -> Option<i32> {
        Some(i32::from_le_bytes(self.get_bytes::<4>(
            buffer,
            relative,
            little_endian,
        )?))
    }

    pub fn set_i32(
        self,
        buffer: &mut ByteBuffer,
        relative: usize,
        value: i32,
        little_endian: bool,
    ) -> bool {
        self.set_bytes::<4>(buffer, relative, value.to_le_bytes(), little_endian)
    }

    pub fn get_f32(self, buffer: &ByteBuffer, relative: usize, little_endian: bool) -> Option<f32> {
        Some(f32::from_le_bytes(self.get_bytes::<4>(
            buffer,
            relative,
            little_endian,
        )?))
    }

    pub fn set_f32(
        self,
        buffer: &mut ByteBuffer,
        relative: usize,
        value: f32,
        little_endian: bool,
    ) -> bool {
        self.set_bytes::<4>(buffer, relative, value.to_le_bytes(), little_endian)
    }

    pub fn get_i64(self, buffer: &ByteBuffer, relative: usize, little_endian: bool) -> Option<i64> {
        Some(i64::from_le_bytes(self.get_bytes::<8>(
            buffer,
            relative,
            little_endian,
        )?))
    }

    pub fn set_i64(
        self,
        buffer: &mut ByteBuffer,
        relative: usize,
        value: i64,
        little_endian: bool,
    ) -> bool {
        self.set_bytes::<8>(buffer, relative, value.to_le_bytes(), little_endian)
    }

    pub fn get_u64(self, buffer: &ByteBuffer, relative: usize, little_endian: bool) -> Option<u64> {
        Some(u64::from_le_bytes(self.get_bytes::<8>(
            buffer,
            relative,
            little_endian,
        )?))
    }

    pub fn set_u64(
        self,
        buffer: &mut ByteBuffer,
        relative: usize,
        value: u64,
        little_endian: bool,
    ) -> bool {
        self.set_bytes::<8>(buffer, relative, value.to_le_bytes(), little_endian)
    }
}

impl TypedArrayView {
    /// `length` counts **elements**, as `new Uint32Array(buffer, offset,
    /// length)` does in JS — so eight here is eight `u32`s, thirty-two bytes.
    ///
    /// It used to count bytes, which silently gave a quarter of the requested
    /// `u32` array: `new Uint32Array(b, 0, 8)` had `length() == 2`. Ported
    /// code read and wrote past the end of what it thought it had.
    pub fn new(
        buffer_length: usize,
        offset: usize,
        length: usize,
        element: TypedElement,
    ) -> Option<Self> {
        let byte_length = length.checked_mul(element.size())?;
        let view = ByteView::new(buffer_length, offset, byte_length)?;
        (offset % element.size() == 0).then_some(Self { view, element })
    }

    pub fn len(self) -> usize {
        self.view.len() / self.element.size()
    }

    pub fn byte_len(self) -> usize {
        self.view.len()
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

    pub fn read_u16(self, buffer: &ByteBuffer, index: usize) -> Option<u16> {
        (self.element == TypedElement::U16)
            .then(|| buffer.read_u16(self.view.offset + index.checked_mul(2)?))
            .flatten()
    }

    pub fn write_u16(self, buffer: &mut ByteBuffer, index: usize, value: u16) -> bool {
        self.element == TypedElement::U16
            && index
                .checked_mul(2)
                .is_some_and(|offset| self.view.write_u16(buffer, offset, value))
    }

    pub fn read_u32(self, buffer: &ByteBuffer, index: usize) -> Option<u32> {
        if self.element != TypedElement::U32 {
            return None;
        }
        self.view.read_u32(buffer, index.checked_mul(4)?)
    }

    pub fn write_u32(self, buffer: &mut ByteBuffer, index: usize, value: u32) -> bool {
        self.element == TypedElement::U32
            && index
                .checked_mul(4)
                .is_some_and(|offset| self.view.write_u32(buffer, offset, value))
    }

    pub fn read_f64(self, buffer: &ByteBuffer, index: usize) -> Option<f64> {
        if self.element != TypedElement::F64 {
            return None;
        }
        let offset = index.checked_mul(8)?;
        (offset.checked_add(8)? <= self.view.len())
            .then(|| buffer.read_f64(self.view.offset() + offset))
            .flatten()
    }

    pub fn write_f64(self, buffer: &mut ByteBuffer, index: usize, value: f64) -> bool {
        if self.element != TypedElement::F64 {
            return false;
        }
        let Some(offset) = index.checked_mul(8) else {
            return false;
        };
        offset.checked_add(8).is_some_and(|end| {
            end <= self.view.len() && buffer.write_f64(self.view.offset() + offset, value)
        })
    }
}
