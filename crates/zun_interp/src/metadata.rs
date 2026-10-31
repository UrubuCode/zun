//! Metadata lookup helpers used by interpreter execution.

use super::{Interp, RunError};
use super::operands::operand_strref;
use super::value::Value;
use zun_il::Operand;

impl<'a> Interp<'a> {
    pub(crate) fn type_field_range(
        &self,
        token: zun_metadata::Token,
    ) -> Result<(u32, usize), RunError> {
        let ty = self
            .module
            .tables
            .type_defs
            .get(token.rid() as usize - 1)
            .ok_or(RunError::BadToken)?;
        Ok((ty.field_start, (ty.field_end - ty.field_start) as usize))
    }

    pub(crate) fn field_kind_at(&self, field_rid: u32) -> Result<u8, RunError> {
        self.module
            .tables
            .field_defs
            .get(field_rid as usize - 1)
            .map(|field| field.kind)
            .ok_or(RunError::BadToken)
    }

    pub(crate) fn static_field_key(
        &self,
        token: zun_metadata::Token,
    ) -> Result<(u32, u32), RunError> {
        if token.table() != Some(zun_metadata::TableKind::FieldDef) || token.rid() == 0 {
            return Err(RunError::BadToken);
        }
        let field = self.module.tables.field_defs.get(token.rid() as usize - 1).ok_or(RunError::BadToken)?;
        if !field.is_static {
            return Err(RunError::BadToken);
        }
        let owner = self.module.tables.type_defs.iter().enumerate()
            .find(|(_, ty)| ty.field_start <= token.rid() && token.rid() < ty.field_end)
            .map(|(index, _)| index as u32 + 1)
            .ok_or(RunError::BadToken)?;
        Ok((owner, token.rid()))
    }

    /// The declared value-type code of a static field, used to restore the exact
    /// numeric width when loading it back from the width-erasing `Any` store.
    pub(crate) fn static_field_kind(
        &self,
        token: zun_metadata::Token,
    ) -> Result<u8, RunError> {
        if token.table() != Some(zun_metadata::TableKind::FieldDef) || token.rid() == 0 {
            return Err(RunError::BadToken);
        }
        self.module
            .tables
            .field_defs
            .get(token.rid() as usize - 1)
            .map(|field| field.kind)
            .ok_or(RunError::BadToken)
    }

    /// The method RID implementing `name` on a typed instance (most-derived
    /// override), or `None` when the object is not a typed instance or has no
    /// such method.
    pub(crate) fn instance_method_rid(&self, object: zun_gc::Obj, name: &str) -> Option<u32> {
        let type_rid = object.type_rid();
        if type_rid == 0 {
            return None;
        }
        let descriptor = self.types.get(type_rid)?;
        descriptor.vtable.iter().copied().find(|&rid| {
            rid != 0
                && self
                    .module
                    .tables
                    .method_defs
                    .get(rid as usize - 1)
                    .is_some_and(|method| self.module.strings.get(method.name) == name)
        })
    }

    /// Read member `name` of a typed class instance reached through `any`: an
    /// instance field, else a `get_<name>` accessor. `Ok(None)` when the object
    /// is not a typed instance or has no such member.
    pub(crate) fn instance_member_get(
        &mut self,
        object: zun_gc::Obj,
        name: &str,
    ) -> Result<Option<Value>, RunError> {
        let type_rid = object.type_rid();
        if type_rid == 0 {
            return Ok(None);
        }
        let Some(type_def) = self.module.tables.type_defs.get(type_rid as usize - 1) else {
            return Ok(None);
        };
        let (start, end) = (type_def.field_start, type_def.field_end);
        for field_rid in start..end {
            let Some(field) = self.module.tables.field_defs.get(field_rid as usize - 1) else {
                continue;
            };
            if field.is_static || self.module.strings.get(field.name) != name {
                continue;
            }
            let Some(layout) = self.types.get_field(field_rid) else {
                continue;
            };
            let value = zun_gc::with_current_heap(|heap| {
                crate::value::load_field(layout.kind, object, layout.slot, heap)
            });
            return Ok(Some(value));
        }
        let getter = format!("get_{name}");
        if let Some(rid) = self.instance_method_rid(object, &getter) {
            return self.call(rid, vec![Value::Object(object)], None).map(Some);
        }
        Ok(None)
    }

    pub(crate) fn dynamic_name(&self, operand: &Operand) -> Result<&str, RunError> {
        let index = operand_strref(operand);
        Ok(self.module.strings.get(zun_metadata::StringIndex(index)))
    }
}

impl Interp<'_> {
    /// Assign member `name` of a typed class instance reached through `any`
    /// when it names a declared instance field (searching base classes too).
    /// Returns whether a declared field took the value.
    pub(crate) fn instance_member_set(&mut self, object: zun_gc::Obj, name: &str, value: &Value) -> bool {
        let mut current = object.type_rid();
        for _ in 0..64 {
            if current == 0 {
                return false;
            }
            let Some(type_def) = self.module.tables.type_defs.get(current as usize - 1) else {
                return false;
            };
            let (start, end, base) = (type_def.field_start, type_def.field_end, type_def.base);
            for field_rid in start..end {
                let Some(field) = self.module.tables.field_defs.get(field_rid as usize - 1) else {
                    continue;
                };
                if field.is_static || self.module.strings.get(field.name) != name {
                    continue;
                }
                let Some(layout) = self.types.get_field(field_rid) else {
                    continue;
                };
                let (slot, kind) = (layout.slot, layout.kind);
                let value = match value {
                    Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .unwrap_or(Value::Void),
                    other => other.clone(),
                };
                zun_gc::with_current_heap_mut(|heap| {
                    crate::value::store_field(kind, object, slot, &value, heap);
                });
                return true;
            }
            if base.is_null() || base.table() != Some(zun_metadata::TableKind::TypeDef) {
                return false;
            }
            current = base.rid();
        }
        false
    }
}
