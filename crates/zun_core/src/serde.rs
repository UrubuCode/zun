//! The binary serialize/deserialize **wire-format contract** — the ser/de
//! PRIMITIVE (Erlang `term_to_binary`/`binary_to_term`-style), not a JSON API.
//!
//! This module owns only the *format*: the version byte, the per-value tag
//! set, and the compact integer/byte encodings. It is deliberately
//! dependency-free (like the rest of `zun_core`), so the compiler and both
//! runtime tiers share one definition of the bytes. The actual heap shape-walk
//! and object reconstruction ride on top of this in `zun_interp` (the Tier-0
//! floor), because they need the GC heap and the metadata type tables.
//!
//! ## Format
//! `[version byte] [one tagged value]`. Each value begins with a one-byte
//! [`Tag`]. Lengths, counts, memo indices and metadata type tokens are
//! unsigned LEB128 varints ([`write_uvarint`]/[`Reader::read_uvarint`]).
//! Fixed-width numbers are little-endian.
//!
//! ## Sharing & cycles
//! Every heap object is assigned a **memo index** the first time it is seen
//! (first sight → index `n` + its full encoding; later sights → a [`Tag::Ref`]
//! carrying `n`). Indices are assigned in pre-order (the container's own index
//! before its children), so a deserializer that allocates the shell, records
//! it at the same index, then fills the children, resolves a back-reference to
//! the object currently being built — which is how cyclic graphs rebuild
//! without infinite recursion.
//!
//! ## Safety
//! The format is data-only. An unknown tag, a truncated stream, or (a layer
//! up) a metadata type token that does not resolve in the deserializing
//! program is a clean error — never code execution, never a constructor body.

/// The format version. Bumped only on an incompatible wire change.
pub const SERDE_VERSION: u8 = 1;

/// The one-byte self-describing tag that opens every encoded value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Tag {
    /// JavaScript `null`.
    Null = 0,
    /// `undefined` (absent value).
    Undefined = 1,
    /// Boolean `false`.
    False = 2,
    /// Boolean `true`.
    True = 3,
    /// An integer of one of the lattice kinds: a kind byte
    /// ([`zun_il`-style `NumKind::to_imm`], carried as `u8`) then an i64 LE.
    Int = 4,
    /// A 64-bit float: 8 bytes LE (`f64::to_bits`).
    Float64 = 5,
    /// A 32-bit float: 4 bytes LE (`f32::to_bits`).
    Float32 = 6,
    /// A string: a uvarint byte length then that many UTF-8 bytes.
    Str = 7,
    /// A BigInt: a uvarint byte length then that many UTF-8 bytes of its
    /// canonical decimal spelling (with a leading `-` when negative).
    BigInt = 8,
    /// An array (memoized): a uvarint element count then each element value.
    Array = 9,
    /// A dynamic object (memoized): a uvarint entry count then, per entry, a
    /// string key (uvarint length + UTF-8 bytes) and its value.
    Object = 10,
    /// A typed class instance (memoized): a uvarint metadata type token then a
    /// uvarint field count then each declared instance field value, in layout
    /// order (base classes first, static/private excluded).
    Instance = 11,
    /// A `Map` (memoized): a uvarint size then, per entry, a key value and a
    /// value value.
    Map = 12,
    /// A `Set` (memoized): a uvarint size then each element value.
    Set = 13,
    /// A `Date` (memoized): a one-byte valid flag; if `1`, an i64 LE of its
    /// epoch-millisecond timestamp.
    Date = 14,
    /// An `ArrayBuffer` (memoized): a uvarint byte length then the raw bytes.
    ArrayBuffer = 15,
    /// A typed array / `Buffer` (memoized): a one-byte element-kind
    /// discriminant (`TypedElement::index`) then a uvarint byte length then
    /// the raw bytes.
    TypedArray = 16,
    /// A back-reference to an already-encoded object: a uvarint memo index.
    Ref = 17,
    /// A dynamic object (memoized) that also carries symbol-keyed entries: the
    /// [`Tag::Object`] body (count + string-keyed entries), then a uvarint
    /// symbol-entry count and, per entry, a kind byte (`0` = registered
    /// `Symbol.for` key: a length-prefixed string; `1` = well-known: a uvarint
    /// id) then the value. Unique `Symbol()` keys are not encoded (they have no
    /// cross-deserialize identity). An object with no encodable symbol key is
    /// still written as plain [`Tag::Object`], keeping that path byte-compatible.
    ObjectSym = 18,
    /// A `DataView` (memoized): the whole backing buffer (uvarint length +
    /// raw bytes), a uvarint `byteOffset`, a uvarint `byteLength`, and a
    /// one-byte length-tracking flag (`1` = tracks the buffer's length).
    DataView = 19,
}

impl Tag {
    /// Decode a tag byte, or `None` for an unknown tag (a clean error at the
    /// call site, never undefined behavior).
    pub const fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Null,
            1 => Self::Undefined,
            2 => Self::False,
            3 => Self::True,
            4 => Self::Int,
            5 => Self::Float64,
            6 => Self::Float32,
            7 => Self::Str,
            8 => Self::BigInt,
            9 => Self::Array,
            10 => Self::Object,
            11 => Self::Instance,
            12 => Self::Map,
            13 => Self::Set,
            14 => Self::Date,
            15 => Self::ArrayBuffer,
            16 => Self::TypedArray,
            17 => Self::Ref,
            18 => Self::ObjectSym,
            19 => Self::DataView,
            _ => return None,
        })
    }

    /// The tag's wire byte.
    pub const fn to_u8(self) -> u8 {
        self as u8
    }
}

/// Append an unsigned LEB128 varint.
pub fn write_uvarint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// A forward-only reader over a serialized byte stream. Every method returns
/// `None` on a truncated/invalid stream so the caller can raise one clean
/// error rather than panic.
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Start reading at the beginning of `bytes`.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Whether every byte has been consumed.
    pub fn is_empty(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    /// Read one byte.
    pub fn read_u8(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(byte)
    }

    /// Read a tag byte and decode it.
    pub fn read_tag(&mut self) -> Option<Tag> {
        Tag::from_u8(self.read_u8()?)
    }

    /// Read an unsigned LEB128 varint (bounded to 10 bytes / 64 bits).
    pub fn read_uvarint(&mut self) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            if shift >= 64 {
                return None;
            }
            let byte = self.read_u8()?;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
        }
    }

    /// Read a uvarint length then that many raw bytes.
    pub fn read_bytes(&mut self) -> Option<&'a [u8]> {
        let len = self.read_uvarint()? as usize;
        let end = self.pos.checked_add(len)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    /// Read a length-prefixed UTF-8 string.
    pub fn read_str(&mut self) -> Option<String> {
        core::str::from_utf8(self.read_bytes()?).ok().map(str::to_owned)
    }

    /// Read a little-endian `i64`.
    pub fn read_i64(&mut self) -> Option<i64> {
        let mut buf = [0u8; 8];
        for slot in &mut buf {
            *slot = self.read_u8()?;
        }
        Some(i64::from_le_bytes(buf))
    }

    /// Read a little-endian `u32`.
    pub fn read_u32(&mut self) -> Option<u32> {
        let mut buf = [0u8; 4];
        for slot in &mut buf {
            *slot = self.read_u8()?;
        }
        Some(u32::from_le_bytes(buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uvarint_round_trips_across_boundaries() {
        for value in [0u64, 1, 127, 128, 300, 16_384, u32::MAX as u64, u64::MAX] {
            let mut out = Vec::new();
            write_uvarint(&mut out, value);
            let mut reader = Reader::new(&out);
            assert_eq!(reader.read_uvarint(), Some(value));
            assert!(reader.is_empty());
        }
    }

    #[test]
    fn tag_bytes_round_trip_and_reject_unknown() {
        for raw in 0u8..=19 {
            let tag = Tag::from_u8(raw).expect("known tag");
            assert_eq!(tag.to_u8(), raw);
        }
        assert_eq!(Tag::from_u8(200), None);
    }

    #[test]
    fn reader_reports_truncation_as_none() {
        let mut reader = Reader::new(&[0x80]); // varint continuation with no follow-up
        assert_eq!(reader.read_uvarint(), None);
        let mut reader = Reader::new(&[]);
        assert_eq!(reader.read_u8(), None);
        assert_eq!(reader.read_tag(), None);
    }
}
