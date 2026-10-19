//! Method body framing inside the code section.
//!
//! `MethodDef::body_offset` points at a small fixed header followed by the ZIL
//! bytes (the phase-1 equivalent of the CLR's tiny/fat body headers):
//!
//! ```text
//! offset  size  field
//!      0     2  parameter count (u16, little-endian)
//!      2     2  local slot count including parameters (u16, little-endian)
//!      4     1  flags (bit 0: returns a value)
//!      5     3  reserved (zero)
//!      8     4  code length in bytes (u32, little-endian)
//!     12     …  ZIL instruction bytes
//! ```
//!
//! Parameters occupy the leading `param_count` local slots; a caller pushes
//! exactly `param_count` arguments. Until full signatures exist, this frame is
//! how a callee's arity and return-shape travel with its body.

use crate::ReadError;

const HEADER_LEN: usize = 12;
const FLAG_RETURNS_VALUE: u8 = 0b0000_0001;

/// A method body: its frame requirements plus its code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodBody<'a> {
    /// Number of leading local slots that are parameters (the method's arity).
    pub param_count: u16,
    /// Total number of local slots the frame needs (parameters included).
    pub local_count: u16,
    /// Whether the method returns a value (its `ret` leaves one on the stack).
    pub returns_value: bool,
    /// The ZIL instruction bytes.
    pub code: &'a [u8],
}

/// Append a framed method body to `code_section`, returning its offset for
/// `MethodDef::body_offset`.
pub fn write_body(
    code_section: &mut Vec<u8>,
    param_count: u16,
    local_count: u16,
    returns_value: bool,
    code: &[u8],
) -> u32 {
    let offset = code_section.len() as u32;
    code_section.extend_from_slice(&param_count.to_le_bytes());
    code_section.extend_from_slice(&local_count.to_le_bytes());
    let flags = if returns_value { FLAG_RETURNS_VALUE } else { 0 };
    code_section.push(flags);
    code_section.extend_from_slice(&[0, 0, 0]); // reserved
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
    let returns_value = header[4] & FLAG_RETURNS_VALUE != 0;
    let code_len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
    let code = code_section
        .get(start + HEADER_LEN..start + HEADER_LEN + code_len)
        .ok_or(ReadError::Truncated)?;
    Ok(MethodBody {
        param_count,
        local_count,
        returns_value,
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_roundtrip() {
        let mut section = Vec::new();
        let off_a = write_body(&mut section, 2, 3, true, &[0x00, 0x63]);
        let off_b = write_body(&mut section, 0, 0, false, &[0x63]);

        let a = read_body(&section, off_a).unwrap();
        assert_eq!(a.param_count, 2);
        assert_eq!(a.local_count, 3);
        assert!(a.returns_value);
        assert_eq!(a.code, &[0x00, 0x63]);

        let b = read_body(&section, off_b).unwrap();
        assert_eq!(b.param_count, 0);
        assert_eq!(b.local_count, 0);
        assert!(!b.returns_value);
        assert_eq!(b.code, &[0x63]);
    }

    #[test]
    fn truncated_body_is_rejected() {
        let mut section = Vec::new();
        write_body(&mut section, 0, 1, false, &[0x00, 0x00, 0x00]);
        section.truncate(section.len() - 1);
        assert!(matches!(read_body(&section, 0), Err(ReadError::Truncated)));
        assert!(matches!(read_body(&section, 999), Err(ReadError::Truncated)));
    }
}
