//! Own non-enumerable data properties of managed `Error` instances.
//!
//! `message`, `cause` and `AggregateError.errors` live in hidden prefix slots
//! of the fixed `Error` layout (so `Object.keys` skips them), but ECMAScript
//! makes them own, writable, non-enumerable, configurable properties, present
//! only when installed. This module answers which of them are present so
//! `hasOwnProperty`, `in`, `getOwnPropertyNames` and
//! `getOwnPropertyDescriptor` can see them.

use super::*;

impl<'a> Interp<'a> {
    /// True when the typed instance's class chain includes the prelude `Error`.
    pub(crate) fn is_error_instance(&self, obj: zun_gc::Obj) -> bool {
        let mut current = obj.type_rid();
        while current != 0 {
            let Some(ty) = self.module.tables.type_defs.get(current as usize - 1) else {
                return false;
            };
            if self.module.strings.get(ty.name) == "Error" {
                return true;
            }
            if ty.base.is_null() || ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                return false;
            }
            current = ty.base.rid();
        }
        false
    }

    /// The value of an own enumerable `name` on an `Error` instance, present
    /// only once `name` was assigned (`undefined` included).
    pub(crate) fn error_assigned_name(&mut self, obj: zun_gc::Obj) -> Option<Value> {
        let typed = zun_gc::with_current_heap(|heap| {
            !heap.is_dynamic_object(obj) && heap.array_len(obj).is_none()
        });
        if !typed || obj.type_rid() == 0 || !self.is_error_instance(obj) {
            return None;
        }
        if matches!(
            self.instance_member_get(obj, "nameUndefined"),
            Ok(Some(Value::Bool(true)))
        ) {
            return Some(Value::Void);
        }
        if matches!(
            self.instance_member_get(obj, "hasName"),
            Ok(Some(Value::Bool(true)))
        ) {
            if let Ok(Some(Value::Str(name))) = self.instance_member_get(obj, "nameOverride") {
                return Some(Value::Str(name));
            }
        }
        None
    }

    /// `delete err.name`: drop an assigned own `name` so the class-provided
    /// default reads again. Returns whether an assigned name was removed.
    pub(crate) fn error_delete_assigned_name(&mut self, obj: zun_gc::Obj) -> bool {
        if self.error_assigned_name(obj).is_none() {
            return false;
        }
        self.instance_member_set(obj, "hasName", &Value::Bool(false));
        self.instance_member_set(obj, "nameUndefined", &Value::Bool(false));
        self.instance_member_set(obj, "nameOverride", &Value::Str("".into()));
        true
    }

    /// Own non-enumerable data properties of an `Error` instance, in
    /// specification order (`message`, `cause`, `errors`).
    pub(crate) fn error_hidden_own(&mut self, obj: zun_gc::Obj) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        let typed = zun_gc::with_current_heap(|heap| {
            !heap.is_dynamic_object(obj) && heap.array_len(obj).is_none()
        });
        if !typed || obj.type_rid() == 0 || !self.is_error_instance(obj) {
            return out;
        }
        // Own exactly when a message argument other than `undefined` was
        // passed, which the class records in `hasMessage`: `new Error("")`
        // owns an empty `message`. A non-empty message still counts, for the
        // construction paths that cannot set the flag.
        let has_message = matches!(
            self.instance_member_get(obj, "hasMessage"),
            Ok(Some(Value::Bool(true)))
        );
        if let Ok(Some(Value::Str(message))) = self.instance_member_get(obj, "message") {
            if has_message || !message.is_empty() {
                out.push(("message".to_string(), Value::Str(message)));
            }
        }
        let flagged = matches!(
            self.instance_member_get(obj, "hasCause"),
            Ok(Some(Value::Bool(true)))
        );
        if let Ok(Some(cause)) = self.instance_member_get(obj, "cause") {
            if flagged || !matches!(cause, Value::Void) {
                out.push(("cause".to_string(), cause));
            }
        }
        if let Ok(Some(errors)) = self.instance_member_get(obj, "errors") {
            out.push(("errors".to_string(), errors));
        }
        out
    }
}

impl<'a> Interp<'a> {
    /// True when the class `rid` or one of its bases is named `name`.
    pub(crate) fn class_chain_has_name(&self, rid: u32, name: &str) -> bool {
        let mut current = rid;
        for _ in 0..256 {
            if current == 0 {
                return false;
            }
            let Some(ty) = self.module.tables.type_defs.get(current as usize - 1) else {
                return false;
            };
            if self.module.strings.get(ty.name) == name {
                return true;
            }
            if ty.base.is_null() || ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                return false;
            }
            current = ty.base.rid();
        }
        false
    }
}

impl<'a> Interp<'a> {
    /// A runtime-constructed instance of a `Map`/`Set` subclass has no
    /// compile-time field initializers run, so give its native `inner`
    /// storage a fresh collection.
    pub(crate) fn init_builtin_collection_field(&mut self, obj: zun_gc::Obj) {
        let rid = obj.type_rid();
        let is_map = self.class_chain_has_name(rid, "__BuiltinMap");
        let is_set = !is_map && self.class_chain_has_name(rid, "__BuiltinSet");
        if !is_map && !is_set {
            return;
        }
        let Some(layout) = self
            .instance_field_map(rid)
            .and_then(|map| map.get("inner").copied())
        else {
            return;
        };
        zun_gc::with_current_heap_mut(|heap| {
            let collection = if is_map { heap.alloc_map() } else { heap.alloc_set() };
            crate::value::store_field(layout.kind, obj, layout.slot, &Value::Object(collection), heap);
        });
    }
}

impl Interp<'_> {
    /// The builtin a class derives from, when its chain reaches one of the
    /// prelude `__Builtin<Name>` carrier roots: `class M extends Map {}` is
    /// branded `[object Map]`, as every subclass of a builtin is.
    pub(crate) fn builtin_base_brand(&self, rid: u32) -> Option<&str> {
        let mut current = rid;
        for _ in 0..256 {
            if current == 0 {
                return None;
            }
            let ty = self.module.tables.type_defs.get(current as usize - 1)?;
            if let Some(name) = self.module.strings.get(ty.name).strip_prefix("__Builtin") {
                return Some(name);
            }
            if ty.base.is_null() || ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                return None;
            }
            current = ty.base.rid();
        }
        None
    }
}
