//! Definition and reference tables.
//!
//! Metadata rows are grouped into typed tables. Following the .NET split,
//! **definitions** (types/methods/fields declared in this module) live in
//! separate tables from **references** (members declared elsewhere that this
//! module uses). Rows address strings and blobs by heap index and address other
//! rows by [`Token`], never by pointer, so a module stays position-independent.
//!
//! A row's RID is its 1-based position in its table; [`Tables`] hands out tokens
//! that match where it stored the row.

use crate::heap::{BlobIndex, StringIndex};
use crate::token::{TableKind, Token};

/// A type defined in this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDef {
    /// Simple name (string heap index).
    pub name: StringIndex,
    /// Namespace / qualifier (string heap index; 0 for none).
    pub namespace: StringIndex,
    /// Base type, or a null `TypeDef`/`TypeRef` token for a root type.
    pub base: Token,
    /// Half-open range of owned field RIDs in the [`FieldDef`] table: fields
    /// `[field_start, field_end)` belong to this type.
    pub field_start: u32,
    /// Exclusive end of the owned field range.
    pub field_end: u32,
    /// Half-open range of owned method RIDs in the [`MethodDef`] table.
    pub method_start: u32,
    /// Exclusive end of the owned method range.
    pub method_end: u32,
}

/// A field defined in this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldDef {
    /// Field name (string heap index).
    pub name: StringIndex,
    /// Encoded field type signature (blob heap index).
    pub signature: BlobIndex,
    /// Value-type code of the field (a `zun_il::JitKind` byte), giving its
    /// machine representation for the object layout the typed JIT will use.
    pub kind: u8,
}

/// A method defined in this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDef {
    /// Method name (string heap index).
    pub name: StringIndex,
    /// Encoded method signature (blob heap index).
    pub signature: BlobIndex,
    /// Byte offset of the method body in the module's code section, or
    /// `u32::MAX` for an abstract/external method with no body.
    pub body_offset: u32,
}

/// A reference to a type defined in another module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRef {
    /// Resolution scope: the owning `ModuleRef` token (null = current module).
    pub scope: Token,
    /// Simple name (string heap index).
    pub name: StringIndex,
    /// Namespace (string heap index; 0 for none).
    pub namespace: StringIndex,
}

/// A reference to a member (method or field) defined elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRef {
    /// The type that declares the member (a `TypeRef` or `TypeDef` token).
    pub parent: Token,
    /// Member name (string heap index).
    pub name: StringIndex,
    /// Encoded signature (blob heap index).
    pub signature: BlobIndex,
}

/// A reference to another module by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRef {
    /// Module name (string heap index).
    pub name: StringIndex,
}

/// The full set of metadata tables for one module.
#[derive(Debug, Clone, Default)]
pub struct Tables {
    /// Type definitions.
    pub type_defs: Vec<TypeDef>,
    /// Field definitions.
    pub field_defs: Vec<FieldDef>,
    /// Method definitions.
    pub method_defs: Vec<MethodDef>,
    /// Type references.
    pub type_refs: Vec<TypeRef>,
    /// Member references.
    pub member_refs: Vec<MemberRef>,
    /// Module references.
    pub module_refs: Vec<ModuleRef>,
}

impl Tables {
    /// Create empty tables.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a type definition, returning its token.
    pub fn add_type_def(&mut self, row: TypeDef) -> Token {
        push_row(&mut self.type_defs, row, TableKind::TypeDef)
    }

    /// Append a field definition, returning its token.
    pub fn add_field_def(&mut self, row: FieldDef) -> Token {
        push_row(&mut self.field_defs, row, TableKind::FieldDef)
    }

    /// Append a method definition, returning its token.
    pub fn add_method_def(&mut self, row: MethodDef) -> Token {
        push_row(&mut self.method_defs, row, TableKind::MethodDef)
    }

    /// Append a type reference, returning its token.
    pub fn add_type_ref(&mut self, row: TypeRef) -> Token {
        push_row(&mut self.type_refs, row, TableKind::TypeRef)
    }

    /// Append a member reference, returning its token.
    pub fn add_member_ref(&mut self, row: MemberRef) -> Token {
        push_row(&mut self.member_refs, row, TableKind::MemberRef)
    }

    /// Append a module reference, returning its token.
    pub fn add_module_ref(&mut self, row: ModuleRef) -> Token {
        push_row(&mut self.module_refs, row, TableKind::ModuleRef)
    }

    /// Resolve a token back to a `TypeDef`, if it points at one.
    pub fn type_def(&self, token: Token) -> Option<&TypeDef> {
        resolve(&self.type_defs, token, TableKind::TypeDef)
    }

    /// Resolve a token back to a `MethodDef`, if it points at one.
    pub fn method_def(&self, token: Token) -> Option<&MethodDef> {
        resolve(&self.method_defs, token, TableKind::MethodDef)
    }

    /// Resolve a token back to a `FieldDef`, if it points at one.
    pub fn field_def(&self, token: Token) -> Option<&FieldDef> {
        resolve(&self.field_defs, token, TableKind::FieldDef)
    }
}

/// Push `row` onto `table` and return the token for its 1-based RID.
fn push_row<T>(table: &mut Vec<T>, row: T, kind: TableKind) -> Token {
    table.push(row);
    Token::new(kind, table.len() as u32)
}

/// Look up a row by token, verifying the token names the expected table.
fn resolve<T>(table: &[T], token: Token, kind: TableKind) -> Option<&T> {
    if token.table() != Some(kind) || token.is_null() {
        return None;
    }
    table.get(token.rid() as usize - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::{BlobIndex, StringIndex};

    #[test]
    fn tokens_match_insertion_order() {
        let mut t = Tables::new();
        let m0 = t.add_method_def(MethodDef {
            name: StringIndex(1),
            signature: BlobIndex(0),
            body_offset: 0,
        });
        let m1 = t.add_method_def(MethodDef {
            name: StringIndex(2),
            signature: BlobIndex(0),
            body_offset: 10,
        });
        assert_eq!(m0.rid(), 1);
        assert_eq!(m1.rid(), 2);
        assert_eq!(t.method_def(m1).unwrap().body_offset, 10);
    }

    #[test]
    fn resolve_rejects_wrong_table() {
        let mut t = Tables::new();
        let type_tok = t.add_type_def(TypeDef {
            name: StringIndex(1),
            namespace: StringIndex(0),
            base: Token::null(TableKind::TypeDef),
            field_start: 0,
            field_end: 0,
            method_start: 0,
            method_end: 0,
        });
        // A TypeDef token must not resolve against the method table.
        assert!(t.method_def(type_tok).is_none());
        assert!(t.type_def(type_tok).is_some());
    }

    #[test]
    fn field_range_links_type_to_fields() {
        let mut t = Tables::new();
        let f0 = t.add_field_def(FieldDef {
            name: StringIndex(1),
            signature: BlobIndex(0),
            kind: 0,
        });
        let f1 = t.add_field_def(FieldDef {
            name: StringIndex(2),
            signature: BlobIndex(0),
            kind: 0,
        });
        let ty = t.add_type_def(TypeDef {
            name: StringIndex(3),
            namespace: StringIndex(0),
            base: Token::null(TableKind::TypeDef),
            field_start: f0.rid(),
            field_end: f1.rid() + 1,
            method_start: 0,
            method_end: 0,
        });
        let def = t.type_def(ty).unwrap();
        assert_eq!(def.field_end - def.field_start, 2);
    }
}
