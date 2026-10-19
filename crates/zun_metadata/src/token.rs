//! Metadata tokens.
//!
//! A token is a compact, position-independent handle to a metadata row. It
//! encodes which table the row lives in and the 1-based row index (RID) within
//! that table, packed into a single `u32`:
//!
//! ```text
//!  31           24 23                         0
//! +---------------+---------------------------+
//! |   table (8)   |          rid (24)         |
//! +---------------+---------------------------+
//! ```
//!
//! RID `0` is reserved as the null token for a given table, so a token is never
//! all-zero for a valid row. ZIL instructions carry tokens (never raw offsets),
//! which is what lets object and method layout change without rewriting code.

/// Identifies which metadata table a [`Token`] points into.
///
/// The discriminant is the value stored in the high byte of a token and is part
/// of the on-disk format: never renumber an existing variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TableKind {
    /// A type defined in this module.
    TypeDef = 0x01,
    /// A reference to a type defined elsewhere.
    TypeRef = 0x02,
    /// A method defined in this module.
    MethodDef = 0x03,
    /// A field defined in this module.
    FieldDef = 0x04,
    /// A reference to a member (method or field) defined elsewhere.
    MemberRef = 0x05,
    /// A reference to another module.
    ModuleRef = 0x06,
}

impl TableKind {
    /// Recover a table kind from its on-disk discriminant.
    pub fn from_u8(value: u8) -> Option<Self> {
        Some(match value {
            0x01 => Self::TypeDef,
            0x02 => Self::TypeRef,
            0x03 => Self::MethodDef,
            0x04 => Self::FieldDef,
            0x05 => Self::MemberRef,
            0x06 => Self::ModuleRef,
            _ => return None,
        })
    }
}

/// The maximum representable RID (24 bits).
pub const MAX_RID: u32 = 0x00FF_FFFF;

/// A packed reference to a metadata row.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token(u32);

impl Token {
    /// Build a token for `kind` pointing at 1-based `rid`.
    ///
    /// Panics if `rid` is 0 (that is the null token, built with [`Token::null`])
    /// or exceeds [`MAX_RID`].
    pub fn new(kind: TableKind, rid: u32) -> Self {
        assert!(rid != 0, "rid 0 is the null token; use Token::null");
        assert!(rid <= MAX_RID, "rid out of range: {rid}");
        Self(((kind as u32) << 24) | rid)
    }

    /// The null token for `kind` (RID 0).
    pub fn null(kind: TableKind) -> Self {
        Self((kind as u32) << 24)
    }

    /// The table this token points into, or `None` if the high byte is not a
    /// known table kind.
    pub fn table(self) -> Option<TableKind> {
        TableKind::from_u8((self.0 >> 24) as u8)
    }

    /// The 1-based row index, or 0 for a null token.
    pub fn rid(self) -> u32 {
        self.0 & MAX_RID
    }

    /// Whether this is a null token (RID 0).
    pub fn is_null(self) -> bool {
        self.rid() == 0
    }

    /// The raw packed value, for serialization.
    pub fn to_raw(self) -> u32 {
        self.0
    }

    /// Rebuild a token from a raw packed value.
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

impl core::fmt::Debug for Token {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.table() {
            Some(kind) => write!(f, "{kind:?}[{}]", self.rid()),
            None => write!(f, "Token(0x{:08X})", self.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TableKind, Token, MAX_RID};

    #[test]
    fn pack_unpack() {
        let t = Token::new(TableKind::MethodDef, 42);
        assert_eq!(t.table(), Some(TableKind::MethodDef));
        assert_eq!(t.rid(), 42);
        assert!(!t.is_null());
    }

    #[test]
    fn null_token() {
        let t = Token::null(TableKind::TypeDef);
        assert!(t.is_null());
        assert_eq!(t.rid(), 0);
        assert_eq!(t.table(), Some(TableKind::TypeDef));
    }

    #[test]
    fn raw_roundtrip() {
        let t = Token::new(TableKind::TypeRef, MAX_RID);
        assert_eq!(Token::from_raw(t.to_raw()), t);
    }

    #[test]
    #[should_panic]
    fn rid_zero_panics() {
        Token::new(TableKind::FieldDef, 0);
    }
}
