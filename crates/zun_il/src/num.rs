//! Numeric kinds and their wire encoding.
//!
//! The `conv` opcode and numeric constants carry a [`NumKind`] identifying an
//! exact numeric representation. It is encoded as a small integer so it fits a
//! ZIL immediate, and is shared by the lowering (`zun_ilgen`) and the runtime
//! (`zun_interp`) so they agree on widths.

/// An exact numeric representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumKind {
    /// Signed 8-bit.
    I8,
    /// Signed 16-bit.
    I16,
    /// Signed 32-bit (`int`).
    I32,
    /// Signed 64-bit.
    I64,
    /// Unsigned 8-bit.
    U8,
    /// Unsigned 16-bit.
    U16,
    /// Unsigned 32-bit.
    U32,
    /// Unsigned 64-bit.
    U64,
    /// 32-bit float.
    F32,
    /// 64-bit float (`number`/`float`).
    F64,
}

impl NumKind {
    /// Encode as an `i32` for a ZIL immediate.
    pub fn to_imm(self) -> i32 {
        match self {
            NumKind::I8 => 0,
            NumKind::I16 => 1,
            NumKind::I32 => 2,
            NumKind::I64 => 3,
            NumKind::U8 => 4,
            NumKind::U16 => 5,
            NumKind::U32 => 6,
            NumKind::U64 => 7,
            NumKind::F32 => 8,
            NumKind::F64 => 9,
        }
    }

    /// Decode from a ZIL immediate.
    pub fn from_imm(v: i32) -> Option<NumKind> {
        Some(match v {
            0 => NumKind::I8,
            1 => NumKind::I16,
            2 => NumKind::I32,
            3 => NumKind::I64,
            4 => NumKind::U8,
            5 => NumKind::U16,
            6 => NumKind::U32,
            7 => NumKind::U64,
            8 => NumKind::F32,
            9 => NumKind::F64,
            _ => return None,
        })
    }

    /// Whether this is an integer kind.
    pub fn is_int(self) -> bool {
        !matches!(self, NumKind::F32 | NumKind::F64)
    }

    /// For an integer kind, its `(bits, signed)`.
    pub fn int_parts(self) -> Option<(u8, bool)> {
        Some(match self {
            NumKind::I8 => (8, true),
            NumKind::I16 => (16, true),
            NumKind::I32 => (32, true),
            NumKind::I64 => (64, true),
            NumKind::U8 => (8, false),
            NumKind::U16 => (16, false),
            NumKind::U32 => (32, false),
            NumKind::U64 => (64, false),
            _ => return None,
        })
    }
}

/// A JIT-level value type recorded per local/return in a method body, so the
/// typed backend knows each slot's machine type without re-deriving it. It is a
/// superset of [`NumKind`] plus `Bool` and `Other` (non-JIT-able: string, ref).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitKind {
    /// A numeric kind.
    Num(NumKind),
    /// A boolean (machine `i32` 0/1).
    Bool,
    /// An object reference (a machine pointer). Code 11 — must match
    /// `zun_interp::value::FIELD_OBJECT`.
    Object,
    /// Anything the typed JIT does not handle (string, void).
    Other,
}

impl JitKind {
    /// Encode as one byte for a method-body frame.
    pub fn to_byte(self) -> u8 {
        match self {
            JitKind::Num(n) => n.to_imm() as u8,
            JitKind::Bool => 10,
            JitKind::Object => 11,
            JitKind::Other => 255,
        }
    }

    /// Decode from a method-body frame byte.
    pub fn from_byte(b: u8) -> JitKind {
        match b {
            10 => JitKind::Bool,
            11 => JitKind::Object,
            255 => JitKind::Other,
            n => match NumKind::from_imm(n as i32) {
                Some(k) => JitKind::Num(k),
                None => JitKind::Other,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{JitKind, NumKind};

    #[test]
    fn jitkind_byte_roundtrip() {
        for k in [
            JitKind::Num(NumKind::U8),
            JitKind::Num(NumKind::F32),
            JitKind::Bool,
            JitKind::Other,
        ] {
            assert_eq!(JitKind::from_byte(k.to_byte()), k);
        }
    }

    #[test]
    fn imm_roundtrip() {
        for k in [
            NumKind::I8,
            NumKind::I16,
            NumKind::I32,
            NumKind::I64,
            NumKind::U8,
            NumKind::U16,
            NumKind::U32,
            NumKind::U64,
            NumKind::F32,
            NumKind::F64,
        ] {
            assert_eq!(NumKind::from_imm(k.to_imm()), Some(k));
        }
        assert_eq!(NumKind::from_imm(99), None);
    }
}
