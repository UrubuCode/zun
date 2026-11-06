//! ECMAScript property-definition semantics shared by `Object.defineProperty`,
//! `Reflect.defineProperty`, `Reflect.set` and the strict `delete` operator,
//! including the integrity levels of arrays and typed arrays.

use super::collections::same_value;
use super::errors::RunError;
use super::integrity::{array_refusal, ArrayOp};
use super::value::Value;
use super::Interp;

/// An own property key: a string or a symbol.
#[derive(Clone, Debug)]
pub(crate) enum PropKey {
    Str(String),
    Sym(zun_core::SymbolId),
}

fn as_object(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Object(object) => Some(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
        _ => None,
    }
}

/// The object a descriptor argument reads its fields from: a plain object, or
/// a function (boxed or direct), whose expando bag holds its own properties.
pub(crate) fn descriptor_object(value: &Value) -> Option<zun_gc::Obj> {
    if let Value::Function(handle) = value {
        return Some(super::function_expando_bag(*handle));
    }
    let obj = as_object(value)?;
    match zun_gc::with_current_heap(|heap| heap.dynamic_get_function(obj, "__boxedFn")) {
        Some(handle) => Some(super::function_expando_bag(handle)),
        None => Some(obj),
    }
}

/// Decode a property-key argument (string, number or symbol).
pub(crate) fn prop_key(value: &Value) -> Option<PropKey> {
    match value {
        Value::Str(text) => Some(PropKey::Str(text.to_string())),
        Value::Int(n, _) => Some(PropKey::Str(n.to_string())),
        Value::Float(f, _) => Some(PropKey::Str(if f.fract() == 0.0 && f.abs() < 1e15 {
            (*f as i64).to_string()
        } else {
            f.to_string()
        })),
        Value::Any(any) => {
            if let Some(symbol) = any.as_symbol() {
                return Some(PropKey::Sym(symbol));
            }
            if let Some(handle) = any.as_string() {
                return zun_gc::with_current_heap(|heap| {
                    heap.try_string(handle).map(|s| PropKey::Str(s.to_string()))
                });
            }
            if let Some(n) = any.as_int() {
                return Some(PropKey::Str(n.to_string()));
            }
            if let Some(f) = any.as_float() {
                return Some(PropKey::Str(if f.fract() == 0.0 && f.abs() < 1e15 {
                    (f as i64).to_string()
                } else {
                    f.to_string()
                }));
            }
            None
        }
        _ => None,
    }
}

fn number_of(value: &Value) -> Option<f64> {
    match value {
        Value::Int(n, _) => Some(*n as f64),
        Value::Float(f, _) => Some(*f),
        Value::Any(any) => any.as_int().map(|n| n as f64).or_else(|| any.as_float()),
        _ => None,
    }
}

/// A parsed property descriptor object.
struct Desc {
    value: Option<zun_core::Any>,
    /// A function-valued `value` (kept in the function slot, not as an `Any`).
    value_fn: Option<zun_core::FunctionHandle>,
    writable: Option<bool>,
    enumerable: Option<bool>,
    configurable: Option<bool>,
    has_get: bool,
    get: Option<zun_core::FunctionHandle>,
    has_set: bool,
    set: Option<zun_core::FunctionHandle>,
}

impl Desc {
    fn parse(desc: zun_gc::Obj) -> Desc {
        zun_gc::with_current_heap_mut(|heap| {
            // A function-valued `value` lives in the descriptor's function slot.
            let mut value = heap.dynamic_get(desc, "value");
            let mut value_fn = heap.dynamic_get_function(desc, "value");
            if value_fn.is_some() {
                value = None;
            }
            // A function stored as a cell object is unwrapped to its handle.
            if let Some(cell) = value.and_then(|any| any.as_object()) {
                let cell = unsafe { zun_gc::Obj::from_addr(cell as *mut u64) };
                if let Some(function) = heap.function_of_cell(cell) {
                    value = None;
                    value_fn = Some(function);
                }
            }
            let flag = |name: &str| {
                heap.dynamic_get(desc, name)
                    .map(|any| super::collections::is_truthy(&Value::Any(any), heap))
            };
            let get = heap.dynamic_get_function(desc, "get");
            let set = heap.dynamic_get_function(desc, "set");
            Desc {
                value,
                value_fn,
                writable: flag("writable"),
                enumerable: flag("enumerable"),
                configurable: flag("configurable"),
                has_get: get.is_some() || heap.dynamic_has(desc, "get"),
                get,
                has_set: set.is_some() || heap.dynamic_has(desc, "set"),
                set,
            }
        })
    }
    fn is_accessor(&self) -> bool {
        self.has_get || self.has_set
    }
    fn is_data(&self) -> bool {
        self.value.is_some() || self.value_fn.is_some() || self.writable.is_some()
    }
}

const OBJECT_PROTOTYPE_KEY: &str = "$Object.prototype";

impl Interp<'_> {
    /// The realm's shared `Object.prototype` object: created on first use and
    /// anchored on the realm global object (so it is rooted and relocated with
    /// it).
    pub(crate) fn object_prototype_singleton() -> zun_gc::Obj {
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(existing) = heap.dynamic_get(global, OBJECT_PROTOTYPE_KEY).and_then(|a| a.as_object()) {
                return unsafe { zun_gc::Obj::from_addr(existing as *mut u64) };
            }
            let root = heap.alloc_dynamic();
            let any = zun_core::Any::object(root.addr() as usize);
            heap.dynamic_define(global, OBJECT_PROTOTYPE_KEY, any, false);
            root
        })
    }

    /// The real `Class.prototype` object of the user class with TypeDef `rid`:
    /// created on first use, anchored (non-enumerable) on the realm global so
    /// it is rooted, and linked to its base class's prototype.
    pub(crate) fn class_prototype_object(&self, rid: u32) -> zun_gc::Obj {
        let key = format!("$classProto#{rid}");
        let existing = zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            heap.dynamic_get(global, &key).and_then(|a| a.as_object())
        });
        if let Some(address) = existing {
            return unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
        }
        let base_rid = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
            .and_then(|def| {
                (def.base.table() == Some(zun_metadata::TableKind::TypeDef) && !def.base.is_null())
                    .then(|| def.base.rid())
            });
        let base = base_rid.filter(|&b| b >= 1).map(|b| self.class_prototype_object(b));
        // The class's own instance accessors (`get_x` / `set_x` methods) become
        // accessor properties of the prototype object.
        let mut accessors: Vec<(String, Option<zun_core::FunctionHandle>, Option<zun_core::FunctionHandle>)> = Vec::new();
        if let Some(def) = self.module.tables.type_defs.get((rid as usize).wrapping_sub(1)) {
            for method_rid in def.method_start..def.method_end {
                let Some(method) = self.module.tables.method_defs.get((method_rid as usize).wrapping_sub(1)) else {
                    continue;
                };
                if method.is_static {
                    continue;
                }
                let name = self.module.strings.get(method.name);
                let (prop, is_get) = if let Some(prop) = name.strip_prefix("get_") {
                    (prop, true)
                } else if let Some(prop) = name.strip_prefix("set_") {
                    (prop, false)
                } else {
                    continue;
                };
                let handle = zun_core::FunctionHandle::new(method_rid, None);
                let index = match accessors.iter().position(|(n, _, _)| n == prop) {
                    Some(index) => index,
                    None => {
                        accessors.push((prop.to_owned(), None, None));
                        accessors.len() - 1
                    }
                };
                if is_get {
                    accessors[index].1 = Some(handle);
                } else {
                    accessors[index].2 = Some(handle);
                }
            }
        }
        zun_gc::with_current_heap_mut(|heap| {
            let proto = heap.alloc_dynamic();
            let any = zun_core::Any::object(proto.addr() as usize);
            let global = heap.global_object();
            heap.dynamic_define(global, key, any, false);
            if let Some(base) = base {
                heap.dynamic_set_prototype(proto, Some(zun_core::Any::object(base.addr() as usize)));
            }
            for (name, get, set) in accessors {
                heap.dynamic_define_accessor(proto, name, get, set, false, true);
            }
            proto
        })
    }

    /// Walk a typed instance's class-prototype chain for an accessor property
    /// `name`: `Some(setter)` (possibly `None` when getter-only) when found.
    pub(crate) fn proto_chain_accessor_setter(
        &self,
        object: zun_gc::Obj,
        name: &str,
    ) -> Option<Option<zun_core::FunctionHandle>> {
        let mut current = self.instance_class_prototype(object)?;
        for _ in 0..256 {
            let step = zun_gc::with_current_heap(|heap| {
                if heap.dynamic_is_accessor(current, name) {
                    return Err(Some(heap.dynamic_accessor_setter(current, name)));
                }
                if heap.dynamic_has(current, name) {
                    return Err(None);
                }
                Ok(heap
                    .dynamic_prototype(current)
                    .and_then(|any| any.as_object())
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }))
            });
            match step {
                Err(found) => return found,
                Ok(Some(next)) => current = next,
                Ok(None) => return None,
            }
        }
        None
    }

    /// TypeDef rid of a user-class descriptor token, when it names a class of
    /// this module.
    pub(crate) fn user_class_rid(&self, token: u32) -> Option<u32> {
        if token & zun_natives::USER_CLASS_FLAG == 0 {
            return None;
        }
        let rid = token & !zun_natives::USER_CLASS_FLAG;
        (rid >= 1 && (rid as usize) <= self.module.tables.type_defs.len()).then_some(rid)
    }

    /// Find static method `name` declared by `rid` or inherited along its base
    /// chain; answers the MethodDef rid.
    pub(crate) fn class_static_method(&self, rid: u32, name: &str) -> Option<(u32, bool)> {
        let mut current = Some(rid);
        for _ in 0..256 {
            let r = current?;
            let def = self.module.tables.type_defs.get((r as usize).wrapping_sub(1))?;
            for mrid in def.method_start..def.method_end {
                let Some(method) = self.module.tables.method_defs.get((mrid as usize).wrapping_sub(1)) else {
                    continue;
                };
                let method_name = self.module.strings.get(method.name);
                let (plain, hidden) = match method_name.strip_suffix("@this") { Some(p) => (p, true), None => (method_name, false) };
                if method.is_static && plain == name {
                    return Some((mrid, hidden));
                }
            }
            current = (def.base.table() == Some(zun_metadata::TableKind::TypeDef) && !def.base.is_null())
                .then(|| def.base.rid());
        }
        None
    }

    /// `Class.name` read through a class value: a static field, a static
    /// getter or a static method found on the class or its bases, else the
    /// class's own name for `name`.
    pub(crate) fn class_static_read(&mut self, rid: u32, name: &str) -> Result<Option<Value>, RunError> {
        let mut current = Some(rid);
        for _ in 0..256 {
            let Some(r) = current else { break };
            let Some(def) = self.module.tables.type_defs.get((r as usize).wrapping_sub(1)).cloned() else {
                break;
            };
            for frid in def.field_start..def.field_end {
                let Some(field) = self.module.tables.field_defs.get((frid as usize).wrapping_sub(1)) else {
                    continue;
                };
                if field.is_static && self.module.strings.get(field.name) == name {
                    let kind = field.kind;
                    let value = zun_gc::with_current_heap(|heap| {
                        if let Some(any) = heap.static_get(r, frid) {
                            let value = Value::unbox_any(any, heap).unwrap_or(Value::Void);
                            Some(crate::value::retag_static_numeric(value, kind))
                        } else {
                            heap.static_get_function(r, frid).map(Value::Function)
                        }
                    });
                    // An inherited-layout copy that was never initialized on this
                    // class falls through to the declaring base class.
                    if let Some(value) = value {
                        return Ok(Some(value));
                    }
                }
            }
            let getter = format!("get_{name}");
            let mut found_getter = None;
            let mut found_method = None;
            for mrid in def.method_start..def.method_end {
                let Some(method) = self.module.tables.method_defs.get((mrid as usize).wrapping_sub(1)) else {
                    continue;
                };
                if !method.is_static {
                    continue;
                }
                let raw_name = self.module.strings.get(method.name);
                let (method_name, hidden) = match raw_name.strip_suffix("@this") {
                    Some(plain) => (plain, true),
                    None => (raw_name, false),
                };
                if method_name == getter {
                    found_getter = Some((mrid, hidden));
                } else if method_name == name && !hidden {
                    found_method = Some(mrid);
                }
            }
            if let Some((mrid, hidden)) = found_getter {
                let handle = zun_core::FunctionHandle::new(mrid, None);
                // The receiving class value rides as the hidden first parameter.
                let call_args = if hidden {
                    vec![Value::Any(zun_core::Any::class_descriptor(
                        zun_natives::USER_CLASS_FLAG | rid,
                    ))]
                } else {
                    Vec::new()
                };
                return self.invoke_function_value(handle, call_args).map(Some);
            }
            if let Some(mrid) = found_method {
                return Ok(Some(Value::Function(zun_core::FunctionHandle::new(mrid, None))));
            }
            current = (def.base.table() == Some(zun_metadata::TableKind::TypeDef) && !def.base.is_null())
                .then(|| def.base.rid());
        }
        if name == "name" {
            let def = self.module.tables.type_defs.get((rid as usize).wrapping_sub(1));
            if let Some(def) = def {
                return Ok(Some(Value::Str(self.module.strings.get(def.name).into())));
            }
        }
        Ok(None)
    }

    /// MethodDef rid of the instance `constructor` declared by `rid` or the
    /// nearest base class.
    pub(crate) fn class_constructor(&self, rid: u32) -> Option<u32> {
        let mut current = Some(rid);
        for _ in 0..256 {
            let r = current?;
            let def = self.module.tables.type_defs.get((r as usize).wrapping_sub(1))?;
            for mrid in def.method_start..def.method_end {
                let method = self.module.tables.method_defs.get((mrid as usize).wrapping_sub(1))?;
                if !method.is_static && self.module.strings.get(method.name) == "constructor" {
                    return Some(mrid);
                }
            }
            current = (def.base.table() == Some(zun_metadata::TableKind::TypeDef) && !def.base.is_null())
                .then(|| def.base.rid());
        }
        None
    }

    /// `Class.name = value` through a class value: store into the static field
    /// found on the class or its bases. Answers whether a field took the write.
    pub(crate) fn class_static_write(&mut self, rid: u32, name: &str, value: &Value) -> bool {
        let mut current = Some(rid);
        for _ in 0..256 {
            let Some(r) = current else { break };
            let Some(def) = self.module.tables.type_defs.get((r as usize).wrapping_sub(1)).cloned() else {
                break;
            };
            for frid in def.field_start..def.field_end {
                let Some(field) = self.module.tables.field_defs.get((frid as usize).wrapping_sub(1)) else {
                    continue;
                };
                if !(field.is_static && self.module.strings.get(field.name) == name) {
                    continue;
                }
                let initialized = zun_gc::with_current_heap(|heap| {
                    heap.static_get(r, frid).is_some() || heap.static_get_function(r, frid).is_some()
                });
                if !initialized {
                    continue;
                }
                zun_gc::with_current_heap_mut(|heap| {
                    if let Value::Function(handle) = value {
                        heap.static_set_function(r, frid, *handle);
                    } else {
                        let any = value.box_any(heap).unwrap_or_else(zun_core::Any::null);
                        heap.static_set(r, frid, any);
                    }
                });
                return true;
            }
            current = (def.base.table() == Some(zun_metadata::TableKind::TypeDef) && !def.base.is_null())
                .then(|| def.base.rid());
        }
        false
    }

    /// The `Class.prototype` object a typed class instance inherits from.
    pub(crate) fn instance_class_prototype(&self, object: zun_gc::Obj) -> Option<zun_gc::Obj> {
        let typed = zun_gc::with_current_heap(|heap| {
            (!heap.is_dynamic_object(object) || heap.is_instance_bag(object))
                && heap.array_len(object).is_none()
        });
        let rid = object.type_rid();
        if !typed || rid < 1 || (rid as usize) > self.module.tables.type_defs.len() {
            return None;
        }
        Some(self.class_prototype_object(rid))
    }

    /// Map a prototype argument to the stored link: an explicit `null` is kept
    /// as a `null` link (no prototype at all); the shared `Object.prototype`
    /// is the default link (stored as none).
    pub(crate) fn normalize_prototype(
        prototype: Option<zun_core::Any>,
        arg: &Value,
    ) -> Option<zun_core::Any> {
        let is_null = matches!(arg, Value::Null | Value::Void)
            || matches!(arg, Value::Any(a) if matches!(a.tag(), zun_core::AnyTag::Null | zun_core::AnyTag::Void));
        if is_null {
            return Some(zun_core::Any::null());
        }
        let root = Self::object_prototype_singleton();
        match prototype {
            Some(link) if link.as_object() == Some(root.addr() as usize) => None,
            other => other,
        }
    }

    /// ValidateAndApplyPropertyDescriptor for a dynamic object. Returns whether
    /// the definition was accepted.
    pub(crate) fn define_own_property(
        &self,
        obj: zun_gc::Obj,
        key: &PropKey,
        desc_obj: zun_gc::Obj,
    ) -> bool {
        let desc = Desc::parse(desc_obj);
        if desc.is_accessor() && desc.is_data() {
            return false;
        }
        // A typed class instance receives expando properties in its bag.
        if zun_gc::with_current_heap(|heap| !heap.is_dynamic_object(obj) && heap.array_len(obj).is_none())
            && self.instance_class_prototype(obj).is_some()
        {
            zun_gc::with_current_heap_mut(|heap| heap.ensure_instance_bag(obj));
        }
        let name = match key {
            PropKey::Sym(symbol) => {
                // Symbol-keyed data properties carry only an enumerable flag.
                let value = desc.value.unwrap_or_else(zun_core::Any::void);
                let enumerable = desc.enumerable.unwrap_or_else(|| {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_symbol_enumerable(obj, *symbol))
                });
                if desc.is_accessor() {
                    return zun_gc::with_current_heap_mut(|heap| {
                        let ok = heap.dynamic_define_symbol(obj, *symbol, zun_core::Any::void(), enumerable);
                        if let Some(getter) = desc.get {
                            heap.dynamic_set_symbol_getter(obj, *symbol, getter);
                        }
                        ok
                    });
                }
                return zun_gc::with_current_heap_mut(|heap| {
                    let ok = heap.dynamic_define_symbol(obj, *symbol, value, enumerable);
                    if let (true, Some(function)) = (ok, desc.value_fn) {
                        heap.dynamic_set_symbol_function(obj, *symbol, function);
                    }
                    ok
                });
            }
            PropKey::Str(name) => name.as_str(),
        };
        if zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()) {
            return self.define_array_property(obj, name, &desc);
        }
        let current = zun_gc::with_current_heap(|heap| heap.dynamic_descriptor(obj, name));
        let Some(current) = current else {
            if !zun_gc::with_current_heap(|heap| heap.dynamic_is_extensible(obj)) {
                return false;
            }
            let enumerable = desc.enumerable.unwrap_or(false);
            let configurable = desc.configurable.unwrap_or(false);
            return zun_gc::with_current_heap_mut(|heap| {
                if desc.is_accessor() {
                    heap.dynamic_define_accessor(obj, name.to_owned(), desc.get, desc.set, enumerable, configurable)
                } else {
                    let ok = heap.dynamic_define_data(
                        obj,
                        name.to_owned(),
                        desc.value.unwrap_or_else(zun_core::Any::void),
                        desc.writable.unwrap_or(false),
                        enumerable,
                        configurable,
                    );
                    if let (true, Some(function)) = (ok, desc.value_fn) {
                        heap.dynamic_set_function(obj, name.to_owned(), function);
                    }
                    ok
                }
            });
        };
        let (cur_enumerable, cur_configurable) = match &current {
            zun_core::PropertyDescriptor::Data { enumerable, configurable, .. } => (*enumerable, *configurable),
            zun_core::PropertyDescriptor::Accessor { enumerable, configurable, .. } => (*enumerable, *configurable),
        };
        if !cur_configurable {
            if desc.configurable == Some(true) {
                return false;
            }
            if let Some(enumerable) = desc.enumerable {
                if enumerable != cur_enumerable {
                    return false;
                }
            }
            match &current {
                zun_core::PropertyDescriptor::Data { value, writable, .. } => {
                    if desc.is_accessor() {
                        return false;
                    }
                    if !writable {
                        if desc.writable == Some(true) {
                            return false;
                        }
                        if let Some(new_value) = desc.value {
                            let same = zun_gc::with_current_heap(|heap| {
                                match (Value::unbox_any(new_value, heap), Value::unbox_any(*value, heap)) {
                                    (Some(a), Some(b)) => same_value(&a, &b),
                                    _ => new_value == *value,
                                }
                            });
                            if !same {
                                return false;
                            }
                        }
                    }
                }
                zun_core::PropertyDescriptor::Accessor { get, set, .. } => {
                    if desc.is_data() {
                        return false;
                    }
                    if desc.has_get && desc.get != *get {
                        return false;
                    }
                    if desc.has_set && desc.set != *set {
                        return false;
                    }
                }
            }
        }
        let enumerable = desc.enumerable.unwrap_or(cur_enumerable);
        let configurable = desc.configurable.unwrap_or(cur_configurable);
        zun_gc::with_current_heap_mut(|heap| match current {
            zun_core::PropertyDescriptor::Data { value, writable, .. } => {
                if desc.is_accessor() {
                    heap.dynamic_force_define_accessor(obj, name.to_owned(), desc.get, desc.set, enumerable, configurable)
                } else {
                    heap.dynamic_force_define_data(
                        obj,
                        name.to_owned(),
                        desc.value.unwrap_or(value),
                        desc.writable.unwrap_or(writable),
                        enumerable,
                        configurable,
                    )
                }
            }
            zun_core::PropertyDescriptor::Accessor { get, set, .. } => {
                if desc.is_data() {
                    heap.dynamic_force_define_data(
                        obj,
                        name.to_owned(),
                        desc.value.unwrap_or_else(zun_core::Any::void),
                        desc.writable.unwrap_or(false),
                        enumerable,
                        configurable,
                    )
                } else {
                    heap.dynamic_force_define_accessor(
                        obj,
                        name.to_owned(),
                        if desc.has_get { desc.get } else { get },
                        if desc.has_set { desc.set } else { set },
                        enumerable,
                        configurable,
                    )
                }
            }
        })
    }

    /// `Object.defineProperty` on an array: only data descriptors that keep the
    /// element/`length` shape (value updates) are supported.
    fn define_array_property(&self, array: zun_gc::Obj, name: &str, desc: &Desc) -> bool {
        let (len, level) = zun_gc::with_current_heap(|heap| {
            (heap.array_len(array).unwrap_or(0), heap.array_integrity(array))
        });
        if desc.is_accessor() {
            // An accessor on an existing array index is stored as a marker
            // record in the element slot; iteration methods run its getter.
            let Ok(index) = name.parse::<usize>() else {
                return false;
            };
            if index >= len || array_refusal(level, ArrayOp::Write).is_some() {
                return false;
            }
            zun_gc::with_current_heap_mut(|heap| {
                let record = heap.alloc_dynamic();
                heap.dynamic_set(record, "__elemAcc", zun_core::Any::bool(true));
                heap.dynamic_define_accessor(
                    record,
                    "v",
                    if desc.has_get { desc.get } else { None },
                    if desc.has_set { desc.set } else { None },
                    desc.enumerable.unwrap_or(false),
                    desc.configurable.unwrap_or(false),
                );
                heap.array_set(array, index, zun_core::Any::object(record.addr() as usize));
            });
            return true;
        }
        if name == "length" {
            if let Some(value) = desc.value {
                let wanted = value.as_int().map(|n| n as f64).or_else(|| value.as_float());
                if let Some(wanted) = wanted {
                    let wanted = wanted as usize;
                    if wanted != len {
                        let op = if wanted < len { ArrayOp::Remove } else { ArrayOp::Write };
                        if array_refusal(level, op).is_some() {
                            return false;
                        }
                        zun_gc::with_current_heap_mut(|heap| heap.array_resize(array, wanted));
                    }
                }
            }
            if desc.writable == Some(false) {
                zun_gc::with_current_heap_mut(|heap| heap.array_raise_integrity(array, 3));
            }
            return true;
        }
        match name.parse::<usize>() {
            Ok(index) if index < len => {
                if let Some(value) = desc.value {
                    if array_refusal(level, ArrayOp::Write).is_some() {
                        return false;
                    }
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(array, index, value));
                }
                true
            }
            Ok(index) => {
                if array_refusal(level, ArrayOp::Add).is_some() {
                    return false;
                }
                zun_gc::with_current_heap_mut(|heap| {
                    heap.array_resize(array, index + 1);
                    heap.array_set(array, index, desc.value.unwrap_or_else(zun_core::Any::void));
                });
                true
            }
            Err(_) => false,
        }
    }

    /// `Reflect.set(target, key, value)`: a boolean success. Honors
    /// writability, extensibility, array integrity, and runs accessor setters.
    pub(crate) fn reflect_set(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let Some(target) = args.first().and_then(as_object) else {
            return Ok(Value::Bool(false));
        };
        let Some(key) = args.get(1).and_then(prop_key) else {
            return Ok(Value::Bool(false));
        };
        let value_arg = args.get(2).ok_or(RunError::BadToken)?;
        // Arrays: elements and `length`.
        let array_info = zun_gc::with_current_heap(|heap| {
            heap.array_len(target).map(|len| (len, heap.array_integrity(target)))
        });
        if let Some((len, level)) = array_info {
            let PropKey::Str(name) = &key else {
                return Ok(Value::Bool(false));
            };
            let any = zun_gc::with_current_heap_mut(|heap| value_arg.box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            if name == "length" {
                let Some(wanted) = number_of(value_arg) else {
                    return Ok(Value::Bool(false));
                };
                let wanted = wanted as usize;
                if wanted == len {
                    return Ok(Value::Bool(true));
                }
                let op = if wanted < len { ArrayOp::Remove } else { ArrayOp::Write };
                if array_refusal(level, op).is_some() {
                    return Ok(Value::Bool(false));
                }
                zun_gc::with_current_heap_mut(|heap| heap.array_resize(target, wanted));
                return Ok(Value::Bool(true));
            }
            let Ok(index) = name.parse::<usize>() else {
                return Ok(Value::Bool(false));
            };
            let op = if index < len { ArrayOp::Write } else { ArrayOp::Add };
            if array_refusal(level, op).is_some() {
                return Ok(Value::Bool(false));
            }
            zun_gc::with_current_heap_mut(|heap| {
                if index >= len {
                    heap.array_resize(target, index + 1);
                }
                heap.array_set(target, index, any);
            });
            return Ok(Value::Bool(true));
        }
        match key {
            PropKey::Sym(symbol) => {
                let any = zun_gc::with_current_heap_mut(|heap| value_arg.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                let ok = zun_gc::with_current_heap_mut(|heap| {
                    let exists = heap.dynamic_get_symbol(target, symbol).is_some();
                    if exists || heap.dynamic_is_extensible(target) {
                        heap.dynamic_set_symbol(target, symbol, any);
                        true
                    } else {
                        false
                    }
                });
                Ok(Value::Bool(ok))
            }
            PropKey::Str(name) => {
                let (is_accessor, setter) = zun_gc::with_current_heap(|heap| {
                    (
                        heap.dynamic_is_accessor(target, &name),
                        heap.dynamic_accessor_setter(target, &name),
                    )
                });
                if is_accessor {
                    let Some(setter) = setter else {
                        return Ok(Value::Bool(false));
                    };
                    let any = zun_gc::with_current_heap_mut(|heap| value_arg.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    let arg = zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                        .unwrap_or(Value::Any(any));
                    self.invoke_function_value(setter, self.with_this_arg(setter, target, vec![arg]))?;
                    return Ok(Value::Bool(true));
                }
                if !zun_gc::with_current_heap(|heap| heap.dynamic_can_set(target, &name)) {
                    return Ok(Value::Bool(false));
                }
                if let Value::Function(handle) = value_arg {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set_function(target, name, *handle);
                    });
                    return Ok(Value::Bool(true));
                }
                let any = zun_gc::with_current_heap_mut(|heap| value_arg.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set(target, name, any);
                });
                Ok(Value::Bool(true))
            }
        }
    }

    /// ToPropertyDescriptor: read a descriptor object's fields in spec order
    /// (`enumerable`, `configurable`, `value`, `writable`, `get`, `set`) through
    /// its getters and prototype chain, validate accessor callability and the
    /// accessor/data conflict, and return a fresh plain descriptor object.
    pub(crate) fn normalize_descriptor(&mut self, desc: zun_gc::Obj) -> Result<zun_gc::Obj, RunError> {
        let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
        let roots = [desc, out];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let (mut has_accessor, mut has_data) = (false, false);
        for field in ["enumerable", "configurable", "value", "writable", "get", "set"] {
            let present = self
                .call_managed_collection(
                    "forInHas",
                    &[Value::Object(roots[0]), Value::Str(field.into())],
                )?;
            if !matches!(present, Value::Bool(true)) {
                continue;
            }
            let value = self.dynamic_member_get(roots[0], field)?;
            match field {
                "enumerable" | "configurable" | "writable" => {
                    if field == "writable" {
                        has_data = true;
                    }
                    let flag = zun_gc::with_current_heap(|heap| super::collections::is_truthy(&value, heap));
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(roots[1], field.to_owned(), zun_core::Any::bool(flag));
                    });
                }
                "value" if matches!(value, Value::Function(_)) => {
                    has_data = true;
                    if let Value::Function(handle) = value {
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set_function(roots[1], "value", handle);
                        });
                    }
                }
                "value" => {
                    has_data = true;
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or_else(zun_core::Any::void);
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(roots[1], "value".to_owned(), boxed);
                    });
                }
                _ => {
                    has_accessor = true;
                    match value {
                        Value::Function(handle) => zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set_function(roots[1], field, handle);
                        }),
                        Value::Void => zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set(roots[1], field.to_owned(), zun_core::Any::void());
                        }),
                        Value::Any(any) if any.tag() == zun_core::AnyTag::Void => {
                            zun_gc::with_current_heap_mut(|heap| {
                                heap.dynamic_set(roots[1], field.to_owned(), zun_core::Any::void());
                            })
                        }
                        _ => {
                            let label = if field == "get" { "Getter" } else { "Setter" };
                            return Err(RunError::Native(format!(
                                "TypeError: {label} must be a function"
                            )));
                        }
                    }
                }
            }
        }
        if has_accessor && has_data {
            return Err(RunError::Native(
                "TypeError: Invalid property descriptor. Cannot both specify accessors and a value or writable attribute".into(),
            ));
        }
        Ok(roots[1])
    }

    /// `Object.defineProperty(obj, key, desc)` / `Reflect.defineProperty`.
    /// `strict` throws a `TypeError` on refusal and returns the object;
    /// otherwise a boolean.
    pub(crate) fn define_property_native(
        &mut self,
        args: &[Value],
        strict: bool,
    ) -> Result<Value, RunError> {
        let Some(target) = args.first().and_then(as_object) else {
            return Err(RunError::Native(
                "TypeError: Object.defineProperty called on non-object".into(),
            ));
        };
        let key = match args.get(1).and_then(prop_key) {
            Some(key) => key,
            // An object key is coerced with ToPropertyKey (ToPrimitive, string hint).
            None => match args.get(1).and_then(as_object) {
                Some(key_object) => match self.ordinary_to_primitive(key_object, "string")? {
                    Some(Value::Str(text)) => PropKey::Str(text.to_string()),
                    _ => PropKey::Str("[object Object]".to_owned()),
                },
                None => return Err(RunError::BadToken),
            },
        };
        let Some(desc) = args.get(2).and_then(descriptor_object) else {
            return Err(RunError::Native(
                "TypeError: Property description must be an object".into(),
            ));
        };
        let desc = self.normalize_descriptor(desc)?;
        let ok = self.define_own_property(target, &key, desc);
        if strict {
            if !ok {
                let shown = match &key {
                    PropKey::Str(name) => name.clone(),
                    PropKey::Sym(_) => "Symbol()".to_owned(),
                };
                return Err(RunError::Native(format!(
                    "TypeError: Cannot redefine property: {shown}"
                )));
            }
            Ok(Value::Object(target))
        } else {
            Ok(Value::Bool(ok))
        }
    }

    /// The strict-mode `delete obj.key` operator: a `TypeError` when the
    /// property is non-configurable, otherwise `true`.
    pub(crate) fn delete_strict(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let Some(target) = args.first().and_then(as_object) else {
            return Ok(Value::Bool(true));
        };
        let Some(PropKey::Str(name)) = args.get(1).and_then(prop_key) else {
            return Ok(Value::Bool(true));
        };
        if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(target)) {
            // `delete arr[i]` leaves a hole at an in-range index.
            if let Ok(index) = name.parse::<usize>() {
                if index < len {
                    if self.guard_array(target, crate::integrity::ArrayOp::Remove).is_err() {
                        return Err(RunError::Native(
                            "TypeError: Cannot delete property of [object Array]".into(),
                        ));
                    }
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(target, index, zun_core::Any::hole()));
                }
            }
            return Ok(Value::Bool(true));
        }
        let allowed = zun_gc::with_current_heap(|heap| heap.dynamic_can_delete(target, &name));
        if !allowed {
            return Err(RunError::Native(format!(
                "TypeError: Cannot delete property '{name}' of #<Object>"
            )));
        }
        zun_gc::with_current_heap_mut(|heap| heap.dynamic_delete(target, &name));
        Ok(Value::Bool(true))
    }

    /// Apply an integrity `level` (1 non-extensible, 2 seal, 3 freeze) to a
    /// non-dynamic object: arrays carry a level, a non-empty typed array cannot
    /// be frozen. Returns whether the object was handled.
    pub(crate) fn apply_nondynamic_integrity(
        &self,
        obj: zun_gc::Obj,
        level: u8,
    ) -> Result<bool, RunError> {
        if zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()) {
            zun_gc::with_current_heap_mut(|heap| heap.array_raise_integrity(obj, level));
            return Ok(true);
        }
        if let Some(len) = crate::typed_arrays::element_len(obj) {
            if level == 3 && len > 0 {
                return Err(RunError::Native(
                    "TypeError: Cannot freeze array buffer views with elements".into(),
                ));
            }
            return Ok(true);
        }
        Ok(false)
    }
}
