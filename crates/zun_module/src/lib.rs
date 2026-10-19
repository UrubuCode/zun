//! The `.zm` container format.
//!
//! A `.zm` file is the unit the compiler emits and the VM loads: a fixed,
//! versioned header followed by four sections — ZIL code, the string heap, the
//! blob heap, and the metadata tables. The header carries absolute offsets so a
//! reader can slice (or later mmap) each section directly without scanning.
//!
//! Layout (all integers little-endian):
//!
//! ```text
//! offset  size  field
//!      0     4  magic "ZUNM"
//!      4     2  format major version (reader rejects a higher major)
//!      6     2  format minor version (reader accepts any minor)
//!      8     4  entry-point token (raw; 0 = no entry point)
//!     12     4  code offset        16  4  code length
//!     20     4  strings offset     24  4  strings length
//!     28     4  blobs offset       32  4  blobs length
//!     36     4  tables offset      40  4  tables length
//!     44     …  section bytes
//! ```
//!
//! The writer lays sections out contiguously in that order, so writing then
//! reading is byte-exact (deterministic builds fall out of this plus the
//! deduplicated heaps).

mod serial;

use zun_metadata::{BlobHeap, StringHeap, Tables, Token};

/// File magic, `b"ZUNM"`.
pub const MAGIC: [u8; 4] = *b"ZUNM";
/// Current format major version. Bump only on breaking layout changes.
pub const FORMAT_MAJOR: u16 = 0;
/// Current format minor version. Bump on backward-compatible additions.
pub const FORMAT_MINOR: u16 = 1;

const HEADER_LEN: usize = 44;

/// An in-memory module: everything a `.zm` file holds.
#[derive(Debug)]
pub struct Module {
    /// Entry-point method token, if the module is executable.
    pub entry_point: Option<Token>,
    /// The ZIL code section; method bodies live at offsets recorded in
    /// `MethodDef::body_offset`.
    pub code: Vec<u8>,
    /// Deduplicated string heap.
    pub strings: StringHeap,
    /// Deduplicated blob heap.
    pub blobs: BlobHeap,
    /// Definition and reference tables.
    pub tables: Tables,
}

impl Default for Module {
    fn default() -> Self {
        Self::new()
    }
}

impl Module {
    /// An empty module.
    pub fn new() -> Self {
        Self {
            entry_point: None,
            code: Vec::new(),
            strings: StringHeap::new(),
            blobs: BlobHeap::new(),
            tables: Tables::new(),
        }
    }

    /// Serialize to `.zm` bytes.
    pub fn write(&self) -> Vec<u8> {
        let tables_bytes = serial::write_tables(&self.tables);

        let code_off = HEADER_LEN as u32;
        let strings_off = code_off + self.code.len() as u32;
        let blobs_off = strings_off + self.strings.as_bytes().len() as u32;
        let tables_off = blobs_off + self.blobs.as_bytes().len() as u32;

        let mut out = Vec::with_capacity(tables_off as usize + tables_bytes.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_MAJOR.to_le_bytes());
        out.extend_from_slice(&FORMAT_MINOR.to_le_bytes());
        out.extend_from_slice(
            &self.entry_point.map_or(0, Token::to_raw).to_le_bytes(),
        );
        for (off, len) in [
            (code_off, self.code.len() as u32),
            (strings_off, self.strings.as_bytes().len() as u32),
            (blobs_off, self.blobs.as_bytes().len() as u32),
            (tables_off, tables_bytes.len() as u32),
        ] {
            out.extend_from_slice(&off.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
        }
        debug_assert_eq!(out.len(), HEADER_LEN);
        out.extend_from_slice(&self.code);
        out.extend_from_slice(self.strings.as_bytes());
        out.extend_from_slice(self.blobs.as_bytes());
        out.extend_from_slice(&tables_bytes);
        out
    }

    /// Deserialize from `.zm` bytes.
    pub fn read(bytes: &[u8]) -> Result<Self, ReadError> {
        if bytes.len() < HEADER_LEN {
            return Err(ReadError::Truncated);
        }
        if bytes[0..4] != MAGIC {
            return Err(ReadError::BadMagic);
        }
        let major = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if major > FORMAT_MAJOR {
            return Err(ReadError::UnsupportedVersion { major });
        }
        let entry_raw = u32::from_le_bytes(bytes[8..12].try_into().unwrap());

        let mut section = |i: usize| -> Result<&[u8], ReadError> {
            let base = 12 + i * 8;
            let off = u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap()) as usize;
            let len = u32::from_le_bytes(bytes[base + 4..base + 8].try_into().unwrap()) as usize;
            bytes.get(off..off + len).ok_or(ReadError::Truncated)
        };
        let code = section(0)?.to_vec();
        let strings = StringHeap::from_bytes(section(1)?.to_vec());
        let blobs = BlobHeap::from_bytes(section(2)?.to_vec());
        let tables = serial::read_tables(section(3)?)?;

        Ok(Self {
            entry_point: if entry_raw == 0 {
                None
            } else {
                Some(Token::from_raw(entry_raw))
            },
            code,
            strings,
            blobs,
            tables,
        })
    }
}

/// An error encountered while reading a `.zm` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    /// The file does not start with the `ZUNM` magic.
    BadMagic,
    /// The file's major format version is newer than this reader.
    UnsupportedVersion {
        /// The major version found in the file.
        major: u16,
    },
    /// The file is shorter than its header or section table claims.
    Truncated,
}

impl core::fmt::Display for ReadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReadError::BadMagic => write!(f, "not a .zm file (bad magic)"),
            ReadError::UnsupportedVersion { major } => {
                write!(f, "unsupported format major version {major}")
            }
            ReadError::Truncated => write!(f, "file is truncated"),
        }
    }
}

impl std::error::Error for ReadError {}
