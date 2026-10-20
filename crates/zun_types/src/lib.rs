//! Reified runtime type information derived from a loaded Zun module.
//!
//! `TypeSystem` is the shared metadata view for the interpreter, JIT and GC.
//! It owns no executable code and does not alter the `.zm` format: all slots
//! are derived from the stable TypeDef/FieldDef/MethodDef tables.

use zun_metadata::Token;
use zun_module::Module;

/// A field's resolved object-layout information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldLayout {
    /// Field definition RID.
    pub field_rid: u32,
    /// Zero-based slot in the object.
    pub slot: usize,
    /// Encoded field value kind.
    pub kind: u8,
}

/// A runtime type descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeDescriptor {
    /// Type definition RID.
    pub type_rid: u32,
    /// Base type token, or null for a root type.
    pub base: Token,
    /// Fields in their physical object-layout order.
    pub fields: Vec<FieldLayout>,
    /// Method RID -> implementation RID for virtual dispatch.
    pub vtable: Vec<u32>,
}

/// Errors found while materializing runtime type information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeError {
    /// A metadata range points outside its table.
    BadMetadata,
    /// A base type token does not identify a local TypeDef.
    BadBase,
}

impl core::fmt::Display for TypeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadMetadata => write!(f, "invalid type metadata range"),
            Self::BadBase => write!(f, "invalid local base type token"),
        }
    }
}

impl std::error::Error for TypeError {}

/// Reified types for one loaded module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSystem {
    types: Vec<TypeDescriptor>,
    fields: Vec<Option<FieldLayout>>,
}

impl TypeSystem {
    /// Build descriptors and virtual dispatch tables from module metadata.
    pub fn from_module(module: &Module) -> Result<Self, TypeError> {
        let method_count = module.tables.method_defs.len();
        let mut types = Vec::with_capacity(module.tables.type_defs.len());
        let mut field_index = vec![None; module.tables.field_defs.len() + 1];
        for (index, ty) in module.tables.type_defs.iter().enumerate() {
            let type_rid = index as u32 + 1;
            let mut type_fields = Vec::new();
            for field_rid in ty.field_start..ty.field_end {
                let field = module
                    .tables
                    .field_defs
                    .get(field_rid as usize - 1)
                    .ok_or(TypeError::BadMetadata)?;
                let layout = FieldLayout {
                    field_rid,
                    slot: (field_rid - ty.field_start) as usize,
                    kind: field.kind,
                };
                field_index[field_rid as usize] = Some(layout);
                type_fields.push(layout);
            }

            let mut vtable = vec![0; method_count + 1];
            for static_rid in 1..=method_count as u32 {
                let name = module.tables.method_defs[static_rid as usize - 1].name;
                let mut current = type_rid;
                loop {
                    let current_ty = module
                        .tables
                        .type_defs
                        .get(current as usize - 1)
                        .ok_or(TypeError::BadBase)?;
                    let implementation = (current_ty.method_start..current_ty.method_end)
                        .find(|rid| module.tables.method_defs[*rid as usize - 1].name == name);
                    if let Some(rid) = implementation {
                        vtable[static_rid as usize] = rid;
                        break;
                    }
                    if current_ty.base.is_null() {
                        break;
                    }
                    if current_ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                        return Err(TypeError::BadBase);
                    }
                    current = current_ty.base.rid();
                }
            }
            types.push(TypeDescriptor {
                type_rid,
                base: ty.base,
                fields: type_fields,
                vtable,
            });
        }
        Ok(Self { types, fields: field_index })
    }

    /// Resolve a local type RID to its descriptor.
    pub fn get(&self, type_rid: u32) -> Option<&TypeDescriptor> {
        type_rid.checked_sub(1).and_then(|rid| self.types.get(rid as usize))
    }

    /// Resolve a field RID to its physical slot and encoded value kind.
    pub fn get_field(&self, field_rid: u32) -> Option<FieldLayout> {
        self.fields.get(field_rid as usize).copied().flatten()
    }

    /// Resolve a virtual method token for a runtime type in O(1).
    pub fn resolve_virtual(&self, type_rid: u32, method_rid: u32) -> Option<u32> {
        self.get(type_rid)?.vtable.get(method_rid as usize).copied().filter(|&rid| rid != 0)
    }

    /// Number of local type descriptors.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether no local types were defined.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}
