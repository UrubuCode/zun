/// Contiguous byte storage for ArrayBuffer-like runtime objects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ByteBuffer {
    bytes: Vec<u8>,
}

impl ByteBuffer {
    pub fn new(length: usize) -> Self { Self { bytes: vec![0; length] } }
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
