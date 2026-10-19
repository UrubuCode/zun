//! Deduplicated metadata heaps.
//!
//! Strings and binary blobs (signatures, constant data) are pooled into heaps
//! and referenced by offset. Interning is content-addressed: storing the same
//! bytes twice yields the same index. Offset `0` is reserved as the empty entry,
//! so a zero index always means "empty string" / "empty blob".
//!
//! The in-memory builders here produce the bytes that `zun_module` writes to
//! disk; the read side will later map the same layout read-only.

use std::collections::HashMap;

/// Index of a string in the [`StringHeap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StringIndex(pub u32);

/// Index of a blob in the [`BlobHeap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlobIndex(pub u32);

/// A deduplicating pool of UTF-8 strings.
///
/// Layout: a leading NUL byte (the empty entry at offset 0), then each distinct
/// string stored as its UTF-8 bytes followed by a NUL terminator.
#[derive(Debug)]
pub struct StringHeap {
    bytes: Vec<u8>,
    lookup: HashMap<Box<str>, StringIndex>,
}

impl Default for StringHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl StringHeap {
    /// Create a heap containing only the empty entry at offset 0.
    pub fn new() -> Self {
        Self {
            bytes: vec![0],
            lookup: HashMap::new(),
        }
    }

    /// Intern `s`, returning its offset. The empty string is always offset 0.
    pub fn intern(&mut self, s: &str) -> StringIndex {
        if s.is_empty() {
            return StringIndex(0);
        }
        if let Some(&idx) = self.lookup.get(s) {
            return idx;
        }
        let idx = StringIndex(self.bytes.len() as u32);
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.push(0);
        self.lookup.insert(s.into(), idx);
        idx
    }

    /// Read back the string stored at `idx`.
    pub fn get(&self, idx: StringIndex) -> &str {
        let start = idx.0 as usize;
        let rest = &self.bytes[start..];
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        std::str::from_utf8(&rest[..end]).expect("string heap holds valid UTF-8")
    }

    /// The raw heap bytes, ready to serialize.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Rebuild a heap from serialized bytes (the read side of [`as_bytes`]).
    ///
    /// The lookup map is rebuilt so that further interning still deduplicates
    /// against the existing entries.
    ///
    /// [`as_bytes`]: StringHeap::as_bytes
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        assert!(!bytes.is_empty() && bytes[0] == 0, "string heap must start with the empty entry");
        let mut lookup = HashMap::new();
        let mut pos = 1usize;
        while pos < bytes.len() {
            let rest = &bytes[pos..];
            let end = rest.iter().position(|&b| b == 0).expect("unterminated heap string");
            let s = std::str::from_utf8(&rest[..end]).expect("string heap holds valid UTF-8");
            lookup.insert(s.into(), StringIndex(pos as u32));
            pos += end + 1;
        }
        Self { bytes, lookup }
    }
}

/// A deduplicating pool of length-prefixed binary blobs.
///
/// Layout: an empty blob at offset 0 (a single `0` length byte), then each
/// distinct blob stored as a compressed length followed by its bytes.
#[derive(Debug)]
pub struct BlobHeap {
    bytes: Vec<u8>,
    lookup: HashMap<Box<[u8]>, BlobIndex>,
}

impl Default for BlobHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl BlobHeap {
    /// Create a heap containing only the empty blob at offset 0.
    pub fn new() -> Self {
        Self {
            bytes: vec![0],
            lookup: HashMap::new(),
        }
    }

    /// Intern `data`, returning its offset. The empty blob is always offset 0.
    pub fn intern(&mut self, data: &[u8]) -> BlobIndex {
        if data.is_empty() {
            return BlobIndex(0);
        }
        if let Some(&idx) = self.lookup.get(data) {
            return idx;
        }
        let idx = BlobIndex(self.bytes.len() as u32);
        write_compressed_len(&mut self.bytes, data.len() as u32);
        self.bytes.extend_from_slice(data);
        self.lookup.insert(data.into(), idx);
        idx
    }

    /// Read back the blob stored at `idx`.
    pub fn get(&self, idx: BlobIndex) -> &[u8] {
        let (len, header) = read_compressed_len(&self.bytes[idx.0 as usize..]);
        let start = idx.0 as usize + header;
        &self.bytes[start..start + len as usize]
    }

    /// The raw heap bytes, ready to serialize.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Rebuild a heap from serialized bytes (the read side of [`as_bytes`]).
    ///
    /// [`as_bytes`]: BlobHeap::as_bytes
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        assert!(!bytes.is_empty() && bytes[0] == 0, "blob heap must start with the empty entry");
        let mut lookup = HashMap::new();
        let mut pos = 1usize;
        while pos < bytes.len() {
            let (len, header) = read_compressed_len(&bytes[pos..]);
            let start = pos + header;
            let end = start + len as usize;
            lookup.insert(bytes[start..end].into(), BlobIndex(pos as u32));
            pos = end;
        }
        Self { bytes, lookup }
    }
}

/// Encode `value` using the ECMA-335-style compressed unsigned integer scheme:
/// 1 byte for < 0x80, 2 bytes for < 0x4000, 4 bytes otherwise.
fn write_compressed_len(out: &mut Vec<u8>, value: u32) {
    if value < 0x80 {
        out.push(value as u8);
    } else if value < 0x4000 {
        out.push(0x80 | (value >> 8) as u8);
        out.push(value as u8);
    } else {
        assert!(value < 0x2000_0000, "blob length out of range");
        out.push(0xC0 | (value >> 24) as u8);
        out.push((value >> 16) as u8);
        out.push((value >> 8) as u8);
        out.push(value as u8);
    }
}

/// Decode a compressed unsigned integer, returning `(value, bytes_consumed)`.
fn read_compressed_len(bytes: &[u8]) -> (u32, usize) {
    let b0 = bytes[0];
    if b0 & 0x80 == 0 {
        (b0 as u32, 1)
    } else if b0 & 0x40 == 0 {
        (((b0 & 0x3F) as u32) << 8 | bytes[1] as u32, 2)
    } else {
        let v = ((b0 & 0x1F) as u32) << 24
            | (bytes[1] as u32) << 16
            | (bytes[2] as u32) << 8
            | bytes[3] as u32;
        (v, 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_dedup_and_readback() {
        let mut h = StringHeap::new();
        let a = h.intern("String");
        let b = h.intern("String");
        let c = h.intern("Number");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(h.get(a), "String");
        assert_eq!(h.get(c), "Number");
    }

    #[test]
    fn empty_string_is_offset_zero() {
        let mut h = StringHeap::new();
        assert_eq!(h.intern(""), StringIndex(0));
        assert_eq!(h.get(StringIndex(0)), "");
    }

    #[test]
    fn blob_dedup_and_readback() {
        let mut h = BlobHeap::new();
        let a = h.intern(&[1, 2, 3]);
        let b = h.intern(&[1, 2, 3]);
        let c = h.intern(&[9, 9]);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(h.get(a), &[1, 2, 3]);
        assert_eq!(h.get(c), &[9, 9]);
    }

    #[test]
    fn blob_large_length_roundtrip() {
        let mut h = BlobHeap::new();
        let data = vec![7u8; 5000];
        let idx = h.intern(&data);
        assert_eq!(h.get(idx), &data[..]);
    }

    #[test]
    fn heaps_from_bytes_preserve_entries_and_dedup() {
        let mut s = StringHeap::new();
        let hello = s.intern("hello");
        let mut b = BlobHeap::new();
        let blob = b.intern(&[1, 2, 3]);

        let mut s2 = StringHeap::from_bytes(s.as_bytes().to_vec());
        let mut b2 = BlobHeap::from_bytes(b.as_bytes().to_vec());
        assert_eq!(s2.get(hello), "hello");
        assert_eq!(b2.get(blob), &[1, 2, 3]);
        // Re-interning existing content must return the original index.
        assert_eq!(s2.intern("hello"), hello);
        assert_eq!(b2.intern(&[1, 2, 3]), blob);
    }

    #[test]
    fn compressed_len_boundaries() {
        for &v in &[0u32, 1, 0x7F, 0x80, 0x3FFF, 0x4000, 0x1FFF_FFFF] {
            let mut out = Vec::new();
            write_compressed_len(&mut out, v);
            let (got, n) = read_compressed_len(&out);
            assert_eq!(got, v);
            assert_eq!(n, out.len());
        }
    }
}
