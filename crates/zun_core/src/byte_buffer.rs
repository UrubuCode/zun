/// Contiguous byte storage for ArrayBuffer-like runtime objects.
///
/// A fixed-length buffer carries `max == None`; a resizable buffer (ES2024
/// `new ArrayBuffer(n, { maxByteLength })`) carries `max == Some(cap)`, where
/// `cap` is the largest byte length [`Self::resize`] will grow to. The stored
/// `bytes` length is always the buffer's current byte length.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ByteBuffer {
    bytes: Vec<u8>,
    /// `Some(max)` for a resizable buffer, `None` for a fixed-length one.
    max: Option<usize>,
}

impl ByteBuffer {
    pub fn new(length: usize) -> Self { Self { bytes: vec![0; length], max: None } }

    /// A resizable buffer of initial byte length `length` that can grow to at
    /// most `max`. Returns `None` when `length > max`, matching the TypeError
    /// the JS constructor throws for `maxByteLength < byteLength`.
    pub fn new_resizable(length: usize, max: usize) -> Option<Self> {
        (length <= max).then(|| Self { bytes: vec![0; length], max: Some(max) })
    }

    /// Whether this buffer was created resizable.
    pub fn resizable(&self) -> bool { self.max.is_some() }

    /// `ArrayBuffer.prototype.maxByteLength`: the growth cap for a resizable
    /// buffer, or the current (fixed) byte length otherwise.
    pub fn max_byte_length(&self) -> usize { self.max.unwrap_or(self.bytes.len()) }

    /// `ArrayBuffer.prototype.resize(newLength)`. Fails (returns `false`,
    /// leaving the buffer untouched) for a fixed-length buffer or when
    /// `new_length` exceeds `maxByteLength`. Growing zero-fills the new tail;
    /// shrinking preserves the surviving prefix.
    pub fn resize(&mut self, new_length: usize) -> bool {
        let Some(max) = self.max else { return false; };
        if new_length > max { return false; }
        self.bytes.resize(new_length, 0);
        true
    }

    pub fn len(&self) -> usize { self.bytes.len() }
    pub fn is_empty(&self) -> bool { self.bytes.is_empty() }
    pub fn get(&self, index: usize) -> Option<u8> { self.bytes.get(index).copied() }

    pub fn set(&mut self, index: usize, value: u8) -> bool {
        let Some(slot) = self.bytes.get_mut(index) else { return false; };
        *slot = value;
        true
    }
    pub fn as_slice(&self) -> &[u8] { &self.bytes }

    pub fn read_u16(&self, offset: usize) -> Option<u16> {
        let bytes = self.bytes.get(offset..offset.checked_add(2)?)?;
        Some(u16::from_le_bytes([bytes[0], bytes[1]]))
    }
    pub fn write_u16(&mut self, offset: usize, value: u16) -> bool {
        let Some(bytes) = self.bytes.get_mut(offset..offset.saturating_add(2)) else { return false; };
        if bytes.len() != 2 { return false; }
        bytes.copy_from_slice(&value.to_le_bytes());
        true
    }
    pub fn read_u32(&self, offset: usize) -> Option<u32> {
        let bytes = self.bytes.get(offset..offset.checked_add(4)?)?;
        Some(u32::from_le_bytes(bytes.try_into().unwrap()))
    }
    pub fn write_u32(&mut self, offset: usize, value: u32) -> bool {
        let Some(bytes) = self.bytes.get_mut(offset..offset.saturating_add(4)) else { return false; };
        if bytes.len() != 4 { return false; }
        bytes.copy_from_slice(&value.to_le_bytes());
        true
    }
    pub fn read_f64(&self, offset: usize) -> Option<f64> {
        let bytes = self.bytes.get(offset..offset.checked_add(8)?)?;
        Some(f64::from_le_bytes(bytes.try_into().unwrap()))
    }
    pub fn write_f64(&mut self, offset: usize, value: f64) -> bool {
        let Some(bytes) = self.bytes.get_mut(offset..offset.saturating_add(8)) else { return false; };
        if bytes.len() != 8 { return false; }
        bytes.copy_from_slice(&value.to_le_bytes());
        true
    }
}
