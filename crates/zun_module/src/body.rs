//! Method body framing inside the code section.
//!
//! `MethodDef::body_offset` points at a small fixed header followed by the ZIL
//! bytes (the phase-1 equivalent of the CLR's tiny/fat body headers):
//!
//! ```text
//! offset  size  field
//!      0     2  parameter count (u16, little-endian)
//!      2     2  local slot count including parameters (u16, little-endian)
//!      4     4  code length in bytes (u32, little-endian)
//!      8     …  ZIL instruction bytes
//! ```
//!
//! Parameters occupy the leading `param_count` local slots; a caller pushes
//! exactly `param_count` arguments. Until full signatures exist (they arrive
//! with the verifier), this is how a callee's arity travels with its body.

use crate::ReadError;

const HEADER_LEN: usize = 8;

/// A method body: its frame requirements plus its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodBody<'a> {
    /// Number of leading local slots that are parameters (the method's arity).
    pub param_count: u16,
    /// Total number of local slots the frame needs (parameters included).
    pub local_count: u16,
    /// The ZIL instruction bytes.
    pub code: &'a [u8],
}

/// Append a framed method body to `code_section`, returning its offset for
/// `MethodDef::body_offset`.
pub fn write_body(code_section: &mut Vec<u8>, param_count: u16, local_count: u16, code: &[u8]) -> u32 {
    let offset = code_section.len() as u32;
    code_section.extend_from_slice(&param_count.to_le_bytes());
    code_section.extend_from_slice(&local_count.to_le_bytes());
    code_section.extend_from_slice(&(code.len() as u32).to_le_bytes());
    code_section.extend_from_slice(code);
    offset
}

/// Read the framed method body starting at `offset` in the code section.
pub fn read_body(code_section: &[u8], offset: u32) -> Result<MethodBody<'_>, ReadError> {
    let start = offset as usize;
    let header = code_section
        .get(start..start + HEADER_LEN)
        .ok_or(ReadError::Truncated)?;
    let param_count = u16::from_le_bytes(header[..2].try_into().unwrap());
    let local_count = u16::from_le_bytes(header[2..4].try_into().unwrap());
    let code_len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let code = code_section
        .get(start + HEADER_LEN..start + HEADER_LEN + code_len)
        .ok_or(ReadError::Truncated)?;
    Ok(MethodBody {
        param_count,
        local_count,
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_roundtrip() {
        let mut section = Vec::new();
        let off_a = write_body(&mut section, 2, 3, &[0x00, 0x63]);
        let off_b = write_body(&mut section, 0, 0, &[0x63]);

        let a = read_body(&section, off_a).unwrap();
        assert_eq!(a.param_count, 2);
        assert_eq!(a.local_count, 3);
        assert_eq!(a.code, &[0x00, 0x63]);

        let b = read_body(&section, off_b).unwrap();
        assert_eq!(b.param_count, 0);
        assert_eq!(b.local_count, 0);
        assert_eq!(b.code, &[0x63]);
    }

    #[test]
    fn truncated_body_is_rejected() {
        let mut section = Vec::new();
        write_body(&mut section, 0, 1, &[0x00, 0x00, 0x00]);
        section.truncate(section.len() - 1);
        assert!(matches!(read_body(&section, 0), Err(ReadError::Truncated)));
        assert!(matches!(read_body(&section, 999), Err(ReadError::Truncated)));
    }
}
