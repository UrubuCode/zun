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
//!     12     4  module-initializer token (raw; 0 = none) — the method run once
//!               before the entry point, analogous to .NET's `<Module>` `.cctor`
//!               (doc 02, area 10). A reader on an older minor treats its slot as
//!               absent, so the field is a backward-compatible addition.
//!     16     4  code offset        20  4  code length
//!     24     4  strings offset     28  4  strings length
//!     32     4  blobs offset       36  4  blobs length
//!     40     4  tables offset      44  4  tables length
//!     48     4  debug offset       52  4  debug length
//!     56     …  section bytes
//! ```
//!
//! After the debug section an optional trailer may follow (minor >= 3), written
//! only when it carries data so ordinary modules are byte-identical to before:
//!
//! ```text
//!      0     4  trailer magic "ZFLG"
//!      4     4  module flags (bit 0: requires_codegen)
//!      8     4  eval-site count N
//!     12  N*8  eval sites: (source_start u32, source_end u32)
//! ```
//!
//! Older readers slice sections by header offsets and ignore the trailer; a
//! module without one reads as requires_codegen = false.
//!
//! The writer lays sections out contiguously in that order, so writing then
//! reading is byte-exact (deterministic builds fall out of this plus the
//! deduplicated heaps).

pub mod body;
mod link;
mod serial;

pub use link::{LinkError, LinkedMethods};
pub use body::{
    read_body, write_body, write_body_with_handlers, ExceptionHandler, HandlerKind, MethodBody,
};

use zun_metadata::{BlobHeap, StringHeap, Tables, Token};

/// File magic, `b"ZUNM"`.
pub const MAGIC: [u8; 4] = *b"ZUNM";
/// Current format major version. Bump only on breaking layout changes.
pub const FORMAT_MAJOR: u16 = 1;
/// Current format minor version. Bump on backward-compatible additions.
pub const FORMAT_MINOR: u16 = 3;

/// Engine core contract version (PLAN section 4: `.zm` format + intrinsic ids +
/// core-type layouts + host ABI). Bump whenever any of those change
/// incompatibly. A `.zmlib` records the value it was built against and the
/// loader requires an exact match (0 in a package means "unspecified/legacy").
pub const CORE_CONTRACT_VERSION: u32 = 1;

const HEADER_LEN: usize = 56;
const TRAILER_MAGIC: [u8; 4] = *b"ZFLG";
const FLAG_REQUIRES_CODEGEN: u32 = 1;

/// A source mapping for one instruction boundary in one method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequencePoint {
    pub method_rid: u32,
    pub pc: u32,
    pub source_start: u32,
    pub source_end: u32,
}

/// Resolved source information for a runtime frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebugFrame<'a> {
    pub method_rid: u32,
    pub method_name: &'a str,
    pub source_span: Option<(u32, u32)>,
}

/// An in-memory module: everything a `.zm` file holds.
#[derive(Debug, Clone)]
pub struct Module {
    /// Entry-point method token, if the module is executable.
    pub entry_point: Option<Token>,
    /// Module-initializer method token, if the module declares one. This single,
    /// eager, no-arg, void method is run exactly once before the entry point —
    /// the Zun analogue of the CLR's `<Module>` type initializer (doc 02, area
    /// 10). Its first (and currently only) client is the declarative realm seed
    /// (PLAN §3.1): it registers the base-type globals so `globalThis.Map` holds
    /// its class descriptor at startup without the program calling `global.set`.
    pub module_initializer: Option<Token>,
    /// The ZIL code section; method bodies live at offsets recorded in
    /// `MethodDef::body_offset`.
    pub code: Vec<u8>,
    /// Deduplicated string heap.
    pub strings: StringHeap,
    /// Deduplicated blob heap.
    pub blobs: BlobHeap,
    /// Definition and reference tables.
    pub tables: Tables,
    /// Optional PC-to-source mappings used by stack capture and diagnostics.
    pub sequence_points: Vec<SequencePoint>,
    /// True when the program uses runtime code generation (new Function /
    /// Function(src) / eval). Persisted in the trailer; the deploy layer maps
    /// it to the "codegen" capability.
    pub requires_codegen: bool,
    /// Source byte spans of the code-generation sites (for diagnostics).
    pub codegen_sites: Vec<(u32, u32)>,
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
            module_initializer: None,
            code: Vec::new(),
            strings: StringHeap::new(),
            blobs: BlobHeap::new(),
            tables: Tables::new(),
            sequence_points: Vec::new(),
            requires_codegen: false,
            codegen_sites: Vec::new(),
        }
    }

    /// Capabilities this module declares on its own (currently "codegen").
    pub fn capabilities(&self) -> Vec<&'static str> {
        if self.requires_codegen {
            vec!["codegen"]
        } else {
            Vec::new()
        }
    }

    /// Resolve the source span covering a method program counter.
    ///
    /// Sequence points are emitted in lowering order. The last point at or
    /// before `pc` is the active source location until the next point.
    pub fn source_span_at(&self, method_rid: u32, pc: u32) -> Option<(u32, u32)> {
        self.sequence_points
            .iter()
            .filter(|point| point.method_rid == method_rid && point.pc <= pc)
            .max_by_key(|point| point.pc)
            .map(|point| (point.source_start, point.source_end))
    }

    /// Resolve a runtime method/PC pair into the method name and source span
    /// used by diagnostic and error-stack consumers.
    pub fn debug_frame_at(&self, method_rid: u32, pc: u32) -> Option<DebugFrame<'_>> {
        let method = self
            .tables
            .method_defs
            .get(method_rid.checked_sub(1)? as usize)?;
        Some(DebugFrame {
            method_rid,
            method_name: self.strings.get(method.name),
            source_span: self.source_span_at(method_rid, pc),
        })
    }

    /// Serialize to `.zm` bytes.
    pub fn write(&self) -> Vec<u8> {
        let tables_bytes = serial::write_tables(&self.tables);

        let code_off = HEADER_LEN as u32;
        let strings_off = code_off + self.code.len() as u32;
        let blobs_off = strings_off + self.strings.as_bytes().len() as u32;
        let tables_off = blobs_off + self.blobs.as_bytes().len() as u32;
        let debug_off = tables_off + tables_bytes.len() as u32;
        let debug_len = self.sequence_points.len() as u32 * 16;

        let mut out = Vec::with_capacity(debug_off as usize + debug_len as usize);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FORMAT_MAJOR.to_le_bytes());
        out.extend_from_slice(&FORMAT_MINOR.to_le_bytes());
        out.extend_from_slice(&self.entry_point.map_or(0, Token::to_raw).to_le_bytes());
        out.extend_from_slice(
            &self
                .module_initializer
                .map_or(0, Token::to_raw)
                .to_le_bytes(),
        );
        for (off, len) in [
            (code_off, self.code.len() as u32),
            (strings_off, self.strings.as_bytes().len() as u32),
            (blobs_off, self.blobs.as_bytes().len() as u32),
            (tables_off, tables_bytes.len() as u32),
            (debug_off, debug_len),
        ] {
            out.extend_from_slice(&off.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
        }
        debug_assert_eq!(out.len(), HEADER_LEN);
        out.extend_from_slice(&self.code);
        out.extend_from_slice(self.strings.as_bytes());
        out.extend_from_slice(self.blobs.as_bytes());
        out.extend_from_slice(&tables_bytes);
        for point in &self.sequence_points {
            out.extend_from_slice(&point.method_rid.to_le_bytes());
            out.extend_from_slice(&point.pc.to_le_bytes());
            out.extend_from_slice(&point.source_start.to_le_bytes());
            out.extend_from_slice(&point.source_end.to_le_bytes());
        }
        if self.requires_codegen {
            out.extend_from_slice(&TRAILER_MAGIC);
            out.extend_from_slice(&FLAG_REQUIRES_CODEGEN.to_le_bytes());
            out.extend_from_slice(&(self.codegen_sites.len() as u32).to_le_bytes());
            for (start, end) in &self.codegen_sites {
                out.extend_from_slice(&start.to_le_bytes());
                out.extend_from_slice(&end.to_le_bytes());
            }
        }
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
        let module_init_raw = u32::from_le_bytes(bytes[12..16].try_into().unwrap());

        let section = |i: usize| -> Result<&[u8], ReadError> {
            let base = 16 + i * 8;
            let off = u32::from_le_bytes(bytes[base..base + 4].try_into().unwrap()) as usize;
            let len = u32::from_le_bytes(bytes[base + 4..base + 8].try_into().unwrap()) as usize;
            bytes.get(off..off + len).ok_or(ReadError::Truncated)
        };
        let code = section(0)?.to_vec();
        let strings = StringHeap::from_bytes(section(1)?.to_vec());
        let blobs = BlobHeap::from_bytes(section(2)?.to_vec());
        let tables = serial::read_tables(section(3)?)?;
        let debug = section(4)?;
        if debug.len() % 16 != 0 {
            return Err(ReadError::Truncated);
        }
        let sequence_points = debug
            .chunks_exact(16)
            .map(|record| SequencePoint {
                method_rid: u32::from_le_bytes(record[0..4].try_into().unwrap()),
                pc: u32::from_le_bytes(record[4..8].try_into().unwrap()),
                source_start: u32::from_le_bytes(record[8..12].try_into().unwrap()),
                source_end: u32::from_le_bytes(record[12..16].try_into().unwrap()),
            })
            .collect();

        let mut requires_codegen = false;
        let mut codegen_sites = Vec::new();
        let debug_base = u32::from_le_bytes(bytes[48..52].try_into().unwrap()) as usize;
        let trailer_off = debug_base + debug.len();
        if let Some(t) = bytes.get(trailer_off..) {
            if t.len() >= 12 && t[0..4] == TRAILER_MAGIC {
                requires_codegen =
                    u32::from_le_bytes(t[4..8].try_into().unwrap()) & FLAG_REQUIRES_CODEGEN != 0;
                let n = u32::from_le_bytes(t[8..12].try_into().unwrap()) as usize;
                let sites = t.get(12..12 + n * 8).ok_or(ReadError::Truncated)?;
                codegen_sites = sites
                    .chunks_exact(8)
                    .map(|c| {
                        (
                            u32::from_le_bytes(c[0..4].try_into().unwrap()),
                            u32::from_le_bytes(c[4..8].try_into().unwrap()),
                        )
                    })
                    .collect();
            }
        }

        Ok(Self {
            entry_point: if entry_raw == 0 {
                None
            } else {
                Some(Token::from_raw(entry_raw))
            },
            module_initializer: if module_init_raw == 0 {
                None
            } else {
                Some(Token::from_raw(module_init_raw))
            },
            code,
            strings,
            blobs,
            tables,
            sequence_points,
            requires_codegen,
            codegen_sites,
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
    /// A method body contains an invalid exception handler region.
    InvalidHandler,
}

impl core::fmt::Display for ReadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReadError::BadMagic => write!(f, "not a .zm file (bad magic)"),
            ReadError::UnsupportedVersion { major } => {
                write!(f, "unsupported format major version {major}")
            }
            ReadError::Truncated => write!(f, "file is truncated"),
            ReadError::InvalidHandler => write!(f, "invalid exception handler region"),
        }
    }
}

impl std::error::Error for ReadError {}

#[cfg(test)]
mod codegen_flag_tests {
    use super::*;

    #[test]
    fn requires_codegen_roundtrips_with_sites() {
        let mut module = Module::new();
        module.requires_codegen = true;
        module.codegen_sites = vec![(4, 20), (30, 44)];
        let read = Module::read(&module.write()).unwrap();
        assert!(read.requires_codegen);
        assert_eq!(read.codegen_sites, vec![(4, 20), (30, 44)]);
        assert_eq!(read.capabilities(), vec!["codegen"]);
    }

    #[test]
    fn plain_module_has_no_trailer_and_reads_false() {
        let module = Module::new();
        let bytes = module.write();
        assert!(!bytes.windows(4).any(|w| w == b"ZFLG"));
        let read = Module::read(&bytes).unwrap();
        assert!(!read.requires_codegen);
        assert!(read.codegen_sites.is_empty());
    }
}
