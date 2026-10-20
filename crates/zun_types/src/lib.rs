//! Reified runtime type information derived from a loaded Zun module.
//!
//! `TypeSystem` is the shared metadata view for the interpreter, JIT and GC.
//! It owns no executable code and does not alter the `.zm` format: all slots
//! are derived from the stable TypeDef/FieldDef/MethodDef tables.

use zun_il::JitKind;
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
    method_slots: Vec<Option<usize>>,
}

/// Immutable load-time binding for the types supplied by `zun:core`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreTypes {
    type_rids: [Option<u32>; 7],
}

impl CoreTypes {
    /// Bind the conventional core type names once while loading the core module.
    /// Runtime lookups thereafter use the positional [`zun_core::CoreTypeId`].
    pub fn from_module(module: &Module, types: &TypeSystem) -> Self {
        let mut type_rids = [None; 7];
        for (index, type_def) in module.tables.type_defs.iter().enumerate() {
            let id = match module.strings.get(type_def.name) {
                "Object" => zun_core::CoreTypeId::Object,
                "String" => zun_core::CoreTypeId::String,
                "Number" => zun_core::CoreTypeId::Number,
                "Boolean" => zun_core::CoreTypeId::Boolean,
                "Array" => zun_core::CoreTypeId::Array,
                "Function" => zun_core::CoreTypeId::Function,
                "Promise" => zun_core::CoreTypeId::Promise,
                _ => continue,
            };
            if types.get(index as u32 + 1).is_some() {
                type_rids[id.get() as usize] = Some(index as u32 + 1);
            }
        }
        Self { type_rids }
    }

    /// Return the local TypeDef RID bound to a core type.
    pub fn get(&self, id: zun_core::CoreTypeId) -> Option<u32> {
        self.type_rids[id.get() as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vtable_resolves_an_override_for_a_derived_type() {
        let module = zun_compiler::compile(
            "class Base { speak(): int { return 1; } }\n\
             class Derived extends Base { speak(): int { return 2; } }\n\
             function main(): int { return 0; }",
        )
        .expect("compile");
        let types = TypeSystem::from_module(&module).expect("types");
        let base_method = module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == "speak")
            .map(|index| index as u32 + 1)
            .expect("base method");
        let derived_method = types
            .resolve_virtual(2, base_method)
            .expect("derived implementation");
        assert_ne!(base_method, derived_method);
    }

    #[test]
    fn vtable_uses_virtual_slots_instead_of_method_rids() {
        let module = zun_compiler::compile(
            "class Base { speak(): int { return 1; } }
             class Derived extends Base { speak(): int { return 2; } }
             function helper(): int { return 3; }
             function main(): int { return 0; }",
        )
        .expect("compile");
        let types = TypeSystem::from_module(&module).expect("types");
        let speak = module
            .tables
            .method_defs
            .iter()
            .position(|method| module.strings.get(method.name) == "speak")
            .map(|index| index as u32 + 1)
            .expect("speak method");
        let slots = types.virtual_slot_count();
        assert!(slots <= module.tables.method_defs.len());
        let slot = types.virtual_slot(speak).expect("speak slot");
        assert!(slot < slots);
        assert_eq!(types.get(2).expect("derived type").vtable.len(), slots);
    }

    #[test]
    fn core_types_bind_the_managed_promise_anchor() {
        let module = zun_compiler::compile(
            "class Promise { state: int; }\n\
             function main(): int { return 0; }",
        )
        .expect("compile");
        let types = TypeSystem::from_module(&module).expect("types");
        let core = CoreTypes::from_module(&module, &types);
        assert!(core.get(zun_core::CoreTypeId::Promise).is_some());
    }

    #[test]
    fn layout_exposes_reference_field_slots() {
        let module = zun_compiler::compile(
            "class Child { value: int; }\n\
             class Parent { child: Child; count: int; }\n\
             function main(): int { return 0; }",
        )
        .expect("compile");
        let types = TypeSystem::from_module(&module).expect("types");
        assert_eq!(types.reference_slots(2), vec![0]);
    }
}

impl TypeSystem {
    /// Build descriptors and virtual dispatch tables from module metadata.
    pub fn from_module(module: &Module) -> Result<Self, TypeError> {
        let method_count = module.tables.method_defs.len();
        let mut method_slots = vec![None; method_count + 1];
        let mut slot_names = Vec::new();
        for (index, method) in module.tables.method_defs.iter().enumerate() {
            let slot = slot_names
                .iter()
                .position(|&name| name == method.name)
                .unwrap_or_else(|| {
                    slot_names.push(method.name);
                    slot_names.len() - 1
                });
            method_slots[index + 1] = Some(slot);
        }
        let mut types: Vec<TypeDescriptor> = Vec::with_capacity(module.tables.type_defs.len());
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

            let mut vtable = if ty.base.is_null() {
                Vec::new()
            } else {
                if ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                    return Err(TypeError::BadBase);
                }
                types
                    .get(ty.base.rid() as usize - 1)
                    .ok_or(TypeError::BadBase)?
                    .vtable
                    .clone()
            };
            if vtable.len() < slot_names.len() {
                vtable.resize(slot_names.len(), 0);
            }
            for rid in ty.method_start..ty.method_end {
                let slot = method_slots[rid as usize].ok_or(TypeError::BadMetadata)?;
                vtable[slot] = rid;
            }
            types.push(TypeDescriptor {
                type_rid,
                base: ty.base,
                fields: type_fields,
                vtable,
            });
        }
        Ok(Self {
            types,
            fields: field_index,
            method_slots,
        })
    }

    /// Resolve a local type RID to its descriptor.
    pub fn get(&self, type_rid: u32) -> Option<&TypeDescriptor> {
        type_rid
            .checked_sub(1)
            .and_then(|rid| self.types.get(rid as usize))
    }

    /// Resolve a field RID to its physical slot and encoded value kind.
    pub fn get_field(&self, field_rid: u32) -> Option<FieldLayout> {
        self.fields.get(field_rid as usize).copied().flatten()
    }

    /// Return object slots whose values are references and must be traced.
    pub fn reference_slots(&self, type_rid: u32) -> Vec<usize> {
        self.get(type_rid)
            .map(|descriptor| {
                descriptor
                    .fields
                    .iter()
                    .filter(|field| JitKind::from_byte(field.kind) == JitKind::Object)
                    .map(|field| field.slot)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Resolve a virtual method token for a runtime type in O(1).
    pub fn resolve_virtual(&self, type_rid: u32, method_rid: u32) -> Option<u32> {
        let slot = self
            .method_slots
            .get(method_rid as usize)
            .copied()
            .flatten()?;
        self.get(type_rid)?
            .vtable
            .get(slot)
            .copied()
            .filter(|&rid| rid != 0)
    }

    /// Resolve a method RID to its compact virtual slot.
    pub fn virtual_slot(&self, method_rid: u32) -> Option<usize> {
        self.method_slots
            .get(method_rid as usize)
            .copied()
            .flatten()
    }

    /// Number of virtual slots materialized in each type vtable.
    pub fn virtual_slot_count(&self) -> usize {
        self.method_slots
            .iter()
            .filter_map(|slot| *slot)
            .max()
            .map_or(0, |slot| slot + 1)
    }

    /// Number of local type descriptors.
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Whether no local types were defined.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Whether a runtime type is the target type or derives from it.
    pub fn is_assignable(&self, runtime_type: u32, target_type: u32) -> bool {
        let mut current = Some(runtime_type);
        while let Some(type_rid) = current {
            if type_rid == target_type {
                return true;
            }
            current = self.get(type_rid).and_then(|descriptor| {
                if descriptor.base.is_null() {
                    None
                } else {
                    Some(descriptor.base.rid())
                }
            });
        }
        false
    }
}
