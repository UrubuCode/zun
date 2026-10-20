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
//!      5     1  return value-type code (opaque to this crate)
//!      6     2  reserved (zero)
//!      8     4  code length in bytes (u32, little-endian)
//!     12     local_count  one value-type code per local slot (opaque bytes)
//!      …     …  ZIL instruction bytes
//! ```
//!
//! Parameters occupy the leading `param_count` local slots; a caller pushes
//! exactly `param_count` arguments. The per-local and return **value-type
//! codes** are opaque bytes here — the compiler and the typed JIT interpret
//! them (as `zun_il::JitKind`) so the backend knows each slot's machine type
//! without re-deriving it. This is the body's standing in for a signature.

use crate::ReadError;
use zun_metadata::Token;

const HEADER_LEN: usize = 12;
const FLAG_RETURNS_VALUE: u8 = 0b0000_0001;
const HANDLER_RECORD_LEN: usize = 24;

/// The kind of transfer performed when a protected region is unwound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerKind {
    /// Transfer to a typed catch handler.
    Catch,
    /// Transfer to a finally handler without matching a type.
    Finally,
}

/// A half-open protected IL interval and its handler target.
///
/// Offsets are relative to the method's instruction stream, not the enclosing
/// module. `catch_type` is null for finally handlers and identifies the error
/// type for catch handlers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionHandler {
    pub try_start: u32,
    pub try_end: u32,
    pub handler_start: u32,
    pub handler_end: u32,
    pub kind: HandlerKind,
    pub catch_type: Option<Token>,
}

/// A method body: its frame requirements, value-type codes, and code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodBody<'a> {
    /// Number of leading local slots that are parameters (the method's arity).
    pub param_count: u16,
    /// Total number of local slots the frame needs (parameters included).
    pub local_count: u16,
    /// Whether the method returns a value (its `ret` leaves one on the stack).
    pub returns_value: bool,
    /// Opaque value-type code of the return value.
    pub return_kind: u8,
    /// Opaque value-type code per local slot (`local_count` bytes).
    pub local_kinds: &'a [u8],
    /// Exception regions attached to this method body.
    pub handlers: Vec<ExceptionHandler>,
    /// The ZIL instruction bytes.
    pub code: &'a [u8],
}

/// Append a framed method body to `code_section`, returning its offset for
/// `MethodDef::body_offset`. `local_kinds` must have `local_count` entries.
pub fn write_body(
    code_section: &mut Vec<u8>,
    param_count: u16,
    local_count: u16,
    returns_value: bool,
    return_kind: u8,
    local_kinds: &[u8],
    code: &[u8],
) -> u32 {
    write_body_with_handlers(
        code_section,
        param_count,
        local_count,
        returns_value,
        return_kind,
        local_kinds,
        code,
        &[],
    )
}

/// Append a method body with protected regions to `code_section`.
pub fn write_body_with_handlers(
    code_section: &mut Vec<u8>,
    param_count: u16,
    local_count: u16,
    returns_value: bool,
    return_kind: u8,
    local_kinds: &[u8],
    code: &[u8],
    handlers: &[ExceptionHandler],
) -> u32 {
    debug_assert_eq!(local_kinds.len(), local_count as usize);
    assert!(handlers.len() <= u16::MAX as usize);
    for handler in handlers {
        assert!(handler.try_start < handler.try_end);
        assert!(handler.handler_start < handler.handler_end);
        assert!(handler.try_end as usize <= code.len());
        assert!(handler.handler_end as usize <= code.len());
        assert_eq!(
            handler.kind == HandlerKind::Finally,
            handler.catch_type.is_none()
        );
    }
    let offset = code_section.len() as u32;
    code_section.extend_from_slice(&param_count.to_le_bytes());
    code_section.extend_from_slice(&local_count.to_le_bytes());
    let flags = if returns_value { FLAG_RETURNS_VALUE } else { 0 };
    code_section.push(flags);
    code_section.push(return_kind);
    code_section.extend_from_slice(&(handlers.len() as u16).to_le_bytes());
    code_section.extend_from_slice(&(code.len() as u32).to_le_bytes());
    code_section.extend_from_slice(local_kinds);
    for handler in handlers {
        code_section.extend_from_slice(&handler.try_start.to_le_bytes());
        code_section.extend_from_slice(&handler.try_end.to_le_bytes());
        code_section.extend_from_slice(&handler.handler_start.to_le_bytes());
        code_section.extend_from_slice(&handler.handler_end.to_le_bytes());
        code_section.push(match handler.kind {
            HandlerKind::Catch => 0,
            HandlerKind::Finally => 1,
        });
        code_section.extend_from_slice(&[0, 0, 0]);
        code_section.extend_from_slice(&handler.catch_type.map_or(0, Token::to_raw).to_le_bytes());
    }
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
    let return_kind = header[5];
    let handler_count = u16::from_le_bytes(header[6..8].try_into().unwrap()) as usize;
    let code_len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;

    let kinds_start = start + HEADER_LEN;
    let local_kinds = code_section
        .get(kinds_start..kinds_start + local_count as usize)
        .ok_or(ReadError::Truncated)?;
    let handlers_start = kinds_start + local_count as usize;
    let handlers_bytes = handler_count
        .checked_mul(HANDLER_RECORD_LEN)
        .ok_or(ReadError::Truncated)?;
    let handlers_slice = code_section
        .get(handlers_start..handlers_start + handlers_bytes)
        .ok_or(ReadError::Truncated)?;
    let mut handlers = Vec::with_capacity(handler_count);
    for record in handlers_slice.chunks_exact(HANDLER_RECORD_LEN) {
        let try_start = u32::from_le_bytes(record[0..4].try_into().unwrap());
        let try_end = u32::from_le_bytes(record[4..8].try_into().unwrap());
        let handler_start = u32::from_le_bytes(record[8..12].try_into().unwrap());
        let handler_end = u32::from_le_bytes(record[12..16].try_into().unwrap());
        let kind = match record[16] {
            0 => HandlerKind::Catch,
            1 => HandlerKind::Finally,
            _ => return Err(ReadError::InvalidHandler),
        };
        let raw_type = u32::from_le_bytes(record[20..24].try_into().unwrap());
        let catch_type = if raw_type == 0 {
            None
        } else {
            Some(Token::from_raw(raw_type))
        };
        if try_start >= try_end
            || handler_start >= handler_end
            || try_end as usize > code_len
            || handler_end as usize > code_len
        {
            return Err(ReadError::InvalidHandler);
        }
        if (kind == HandlerKind::Finally) != catch_type.is_none() {
            return Err(ReadError::InvalidHandler);
        }
        handlers.push(ExceptionHandler {
            try_start,
            try_end,
            handler_start,
            handler_end,
            kind,
            catch_type,
        });
    }
    let code_start = handlers_start + handlers_bytes;
    let code = code_section
        .get(code_start..code_start + code_len)
        .ok_or(ReadError::Truncated)?;
    Ok(MethodBody {
        param_count,
        local_count,
        returns_value,
        return_kind,
        local_kinds,
        handlers,
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_roundtrip() {
        let mut section = Vec::new();
        let off_a = write_body(&mut section, 2, 3, true, 9, &[2, 2, 9], &[0x00, 0x63]);
        let off_b = write_body(&mut section, 0, 0, false, 255, &[], &[0x63]);

        let a = read_body(&section, off_a).unwrap();
        assert_eq!(a.param_count, 2);
        assert_eq!(a.local_count, 3);
        assert!(a.returns_value);
        assert_eq!(a.return_kind, 9);
        assert_eq!(a.local_kinds, &[2, 2, 9]);
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
        write_body(&mut section, 0, 1, false, 2, &[2], &[0x00, 0x00, 0x00]);
        section.truncate(section.len() - 1);
        assert!(matches!(read_body(&section, 0), Err(ReadError::Truncated)));
        assert!(matches!(
            read_body(&section, 999),
            Err(ReadError::Truncated)
        ));
    }

    #[test]
    fn handler_regions_roundtrip_with_method_body() {
        let mut section = Vec::new();
        let catch_type = Token::new(zun_metadata::TableKind::TypeDef, 7);
        let offset = write_body_with_handlers(
            &mut section,
            0,
            0,
            false,
            0,
            &[],
            &[0x00, 0x63, 0x63, 0x63],
            &[ExceptionHandler {
                try_start: 0,
                try_end: 2,
                handler_start: 2,
                handler_end: 4,
                kind: HandlerKind::Catch,
                catch_type: Some(catch_type),
            }],
        );
        let body = read_body(&section, offset).unwrap();
        assert_eq!(body.handlers.len(), 1);
        assert_eq!(body.handlers[0].catch_type, Some(catch_type));
    }

    #[test]
    fn invalid_handler_kind_is_rejected() {
        let mut section = Vec::new();
        let offset = write_body_with_handlers(
            &mut section,
            0,
            0,
            false,
            0,
            &[],
            &[0x63, 0x63],
            &[ExceptionHandler {
                try_start: 0,
                try_end: 1,
                handler_start: 1,
                handler_end: 2,
                kind: HandlerKind::Finally,
                catch_type: None,
            }],
        );
        let kind_offset = offset as usize + 12 + 16;
        section[kind_offset] = 2;
        assert!(matches!(
            read_body(&section, offset),
            Err(ReadError::InvalidHandler)
        ));
    }
}
