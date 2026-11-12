//! Metadata lookup helpers used by interpreter execution.

use super::operands::operand_strref;
use super::value::Value;
use super::{Interp, RunError};
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
        let field = self
            .module
            .tables
            .field_defs
            .get(token.rid() as usize - 1)
            .ok_or(RunError::BadToken)?;
        if !field.is_static {
            return Err(RunError::BadToken);
        }
        let owner = self
            .module
            .tables
            .type_defs
            .iter()
            .enumerate()
            .find(|(_, ty)| ty.field_start <= token.rid() && token.rid() < ty.field_end)
            .map(|(index, _)| index as u32 + 1)
            .ok_or(RunError::BadToken)?;
        Ok((owner, token.rid()))
    }

    /// The declared value-type code of a static field, used to restore the exact
    /// numeric width when loading it back from the width-erasing `Any` store.
    pub(crate) fn static_field_kind(&self, token: zun_metadata::Token) -> Result<u8, RunError> {
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

    /// Arguments for a dynamic call of method `rid`: when the method ends in a
    /// rest parameter (its signature blob is the rest marker), the trailing
    /// arguments are packed into the array the callee expects; omitted fixed
    /// parameters carry the null sentinel its default prologue consumes.
    pub(crate) fn pack_rest_call_args(&self, rid: u32, mut args: Vec<Value>) -> Vec<Value> {
        let Some(method) = self
            .module
            .tables
            .method_defs
            .get((rid as usize).wrapping_sub(1))
        else {
            return args;
        };
        if method.signature.0 == 0 || self.module.blobs.get(method.signature) != b"R" {
            return args;
        }
        let Some(body) = zun_module::read_body(&self.module.code, method.body_offset).ok() else {
            return args;
        };
        let fixed = (body.param_count as usize).saturating_sub(1);
        while args.len() < fixed {
            args.push(Value::Null);
        }
        let rest = args.split_off(fixed);
        let array = zun_gc::with_current_heap_mut(|heap| {
            let array = heap.alloc_array(rest.len());
            for (index, value) in rest.iter().enumerate() {
                if let Some(any) = value.box_any(heap) {
                    let _ = heap.array_set(array, index, any);
                }
            }
            array
        });
        args.push(Value::Object(array));
        args
    }

    /// The RID of the module-level function `name` (a prelude helper such as
    /// `__zunSetOpSet`), or `None` when the module does not carry it. Resolved
    /// through a lazily built name index (first declaration wins), so repeated
    /// helper lookups do not rescan the whole method table.
    pub(crate) fn free_function_rid(&self, name: &str) -> Option<u32> {
        self.function_rid_by_name
            .get_or_init(|| {
                let mut map = std::collections::HashMap::new();
                for (index, method) in self.module.tables.method_defs.iter().enumerate() {
                    map.entry(self.module.strings.get(method.name).to_string())
                        .or_insert(index as u32 + 1);
                }
                map
            })
            .get(name)
            .copied()
    }

    /// The RID of the TypeDef named `name` (first declaration wins), via a
    /// lazily built name index, so dynamic class resolution is O(1).
    pub(crate) fn type_rid_of(&self, name: &str) -> Option<u32> {
        self.type_rid_by_name
            .get_or_init(|| {
                let mut map = std::collections::HashMap::new();
                for (index, ty) in self.module.tables.type_defs.iter().enumerate() {
                    map.entry(self.module.strings.get(ty.name).to_string())
                        .or_insert(index as u32 + 1);
                }
                map
            })
            .get(name)
            .copied()
    }

    /// The method RID implementing `name` on a typed instance (most-derived
    /// override), or `None` when the object is not a typed instance or has no
    /// such method.
    pub(crate) fn instance_method_rid(&self, object: zun_gc::Obj, name: &str) -> Option<u32> {
        self.instance_method_map(object.type_rid())?.get(name).copied()
    }

    /// Lazily built `method name -> implementation rid` index for a type's
    /// vtable (first vtable entry for a name wins, matching the previous
    /// `.find`), so repeated dynamic method lookups are O(1).
    fn instance_method_map(
        &self,
        type_rid: u32,
    ) -> Option<std::rc::Rc<std::collections::HashMap<String, u32>>> {
        if type_rid == 0 {
            return None;
        }
        if let Some(cached) = self.instance_method_index.borrow().get(&type_rid) {
            return Some(cached.clone());
        }
        let descriptor = self.types.get(type_rid)?;
        let mut map = std::collections::HashMap::new();
        for &rid in &descriptor.vtable {
            if rid == 0 {
                continue;
            }
            if let Some(method) = self.module.tables.method_defs.get(rid as usize - 1) {
                map.entry(self.module.strings.get(method.name).to_string())
                    .or_insert(rid);
            }
        }
        let rc = std::rc::Rc::new(map);
        self.instance_method_index
            .borrow_mut()
            .insert(type_rid, rc.clone());
        Some(rc)
    }

    /// Lazily built `field name -> layout` index for a type's own non-static
    /// instance fields (first occurrence wins), so dynamic field access is O(1).
    pub(crate) fn instance_field_map(
        &self,
        type_rid: u32,
    ) -> Option<std::rc::Rc<std::collections::HashMap<String, zun_types::FieldLayout>>> {
        if type_rid == 0 {
            return None;
        }
        if let Some(cached) = self.instance_field_index.borrow().get(&type_rid) {
            return Some(cached.clone());
        }
        let type_def = self.module.tables.type_defs.get(type_rid as usize - 1)?;
        let (start, end) = (type_def.field_start, type_def.field_end);
        let mut map = std::collections::HashMap::new();
        for field_rid in start..end {
            let Some(field) = self.module.tables.field_defs.get(field_rid as usize - 1) else {
                continue;
            };
            if field.is_static {
                continue;
            }
            let Some(layout) = self.types.get_field(field_rid) else {
                continue;
            };
            map.entry(self.module.strings.get(field.name).to_string())
                .or_insert(layout);
        }
        let rc = std::rc::Rc::new(map);
        self.instance_field_index
            .borrow_mut()
            .insert(type_rid, rc.clone());
        Some(rc)
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
        if let Some(layout) = self
            .instance_field_map(type_rid)
            .and_then(|map| map.get(name).copied())
        {
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
    pub(crate) fn instance_member_set(
        &mut self,
        object: zun_gc::Obj,
        name: &str,
        value: &Value,
    ) -> bool {
        let mut current = object.type_rid();
        for _ in 0..64 {
            if current == 0 {
                return false;
            }
            let Some(type_def) = self.module.tables.type_defs.get(current as usize - 1) else {
                return false;
            };
            let base = type_def.base;
            if let Some(layout) = self
                .instance_field_map(current)
                .and_then(|map| map.get(name).copied())
            {
                let (slot, kind) = (layout.slot, layout.kind);
                let value = match value {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .unwrap_or(Value::Void)
                    }
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
