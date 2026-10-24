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

    /// The view's byte offset into its backing buffer — what JS exposes as
    /// `typedArray.byteOffset`. The element index 0 sits here.
    pub fn byte_offset(self) -> usize {
        self.view.offset()
    }

    pub fn element(self) -> TypedElement {
        self.element
    }

    /// `subarray(begin, end)`: another view of the same kind over the same
    /// bytes. Unlike `ArrayBuffer.slice`, this shares the buffer rather than
    /// copying it — a write through either view is visible through the other,
    /// which is the whole reason JS has both.
    ///
    /// `begin` and `end` count elements and are clamped into the view the way
    /// `slice` clamps, so an empty result is a view of length zero rather than
    /// a failure.
    pub fn subview(self, begin: usize, end: usize) -> Self {
        let length = self.len();
        let begin = begin.min(length);
        let end = end.min(length).max(begin);
        let size = self.element.size();
        Self {
            view: ByteView {
                offset: self.view.offset() + begin * size,
                length: (end - begin) * size,
            },
            element: self.element,
        }
    }

    /// The element range `fill(value, begin, end)` covers, clamped the same
    /// way. The filling itself is per kind, since the value is.
    pub fn fill_range(self, begin: usize, end: usize) -> std::ops::Range<usize> {
        let length = self.len();
        let begin = begin.min(length);
        begin..end.min(length).max(begin)
    }
}

impl TypedArrayView {
    /// The bytes of one element, bounds-checked against the view rather than
    /// against the whole buffer: a view is a window, and reading past its end
    /// has to fail even when the buffer behind it is longer.
    fn element_bytes<const N: usize>(self, buffer: &ByteBuffer, index: usize) -> Option<[u8; N]> {
        let offset = index.checked_mul(N)?;
        if offset.checked_add(N)? > self.view.len() {
            return None;
        }
        let base = self.view.offset() + offset;
        let mut bytes = [0u8; N];
        for (step, byte) in bytes.iter_mut().enumerate() {
            *byte = buffer.get(base + step)?;
        }
        Some(bytes)
    }

    fn set_element_bytes<const N: usize>(
        self,
        buffer: &mut ByteBuffer,
        index: usize,
        bytes: [u8; N],
    ) -> bool {
        let Some(offset) = index.checked_mul(N) else {
            return false;
        };
        if offset.checked_add(N).is_none_or(|end| end > self.view.len()) {
            return false;
        }
        let base = self.view.offset() + offset;
        bytes
            .into_iter()
            .enumerate()
            .all(|(step, byte)| buffer.set(base + step, byte))
    }
}

/// Generates the element enum and one read/write pair per kind from the
/// master table, so a kind cannot exist in the enum without its accessors.
macro_rules! define_typed_elements {
    ($({
        variant: $variant:ident, repr: $repr:ty, arg: $arg:ty, width: $width:expr,
        js: $js:literal, native: $native:literal,
        elem_ty: $elem_ty:literal, arg_ty: $arg_ty:literal,
        view_ty: $view_ty:ident, elem_native: $elem_native:ident,
        arg_native: $arg_native:ident,
        encode: $encode:path,
        read: $read:ident, write: $write:ident,
        get: $get:ident, set: $set:ident
    }),+ $(,)?) => {
        /// Which kind of element a typed-array view is a view of.
        ///
        /// Generated from the master table in `typed_elements.rs`.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum TypedElement {
            $($variant),+
        }

        impl TypedElement {
            /// One element's size in bytes.
            pub const fn size(self) -> usize {
                match self {
                    $(Self::$variant => $width),+
                }
            }

            /// The JavaScript constructor name for this kind.
            pub const fn js_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $js),+
                }
            }

            /// Every kind, in table order — for tests that must not fall
            /// behind the table.
            pub const ALL: &'static [TypedElement] = &[$(TypedElement::$variant),+];
        }

        impl TypedArrayView {
            $(
                /// Reads one element. A view of any other kind reads `None`,
                /// so a `Float32Array` cannot be read as if it held `i32`s.
                pub fn $read(self, buffer: &ByteBuffer, index: usize) -> Option<$repr> {
                    if self.element != TypedElement::$variant {
                        return None;
                    }
                    Some(<$repr>::from_le_bytes(
                        self.element_bytes::<{ $width }>(buffer, index)?,
                    ))
                }

                /// Writes one element, converting the accepted value into the
                /// stored representation. Refuses a view of any other kind.
                pub fn $write(self, buffer: &mut ByteBuffer, index: usize, value: $arg) -> bool {
                    self.element == TypedElement::$variant
                        && self.set_element_bytes::<{ $width }>(
                            buffer,
                            index,
                            $encode(value).to_le_bytes(),
                        )
                }
            )+
        }
    };
}

crate::typed_element_table!(define_typed_elements);
