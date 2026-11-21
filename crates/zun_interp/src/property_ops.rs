//! ECMAScript property-definition semantics shared by `Object.defineProperty`,
//! `Reflect.defineProperty`, `Reflect.set` and the strict `delete` operator,
//! including the integrity levels of arrays and typed arrays.

use super::collections::{object, same_value, unbox_arg};
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

/// The object a descriptor argument reads its fields from: a plain object, or
/// a function (boxed or direct), whose expando bag holds its own properties.
pub(crate) fn descriptor_object(value: &Value) -> Option<zun_gc::Obj> {
    if let Value::Function(handle) = value {
        return Some(super::function_expando_bag(*handle));
    }
    let obj = object(value)?;
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

/// Getters on the shared `%TypedArray%.prototype`: `buffer`, `byteLength`,
/// `byteOffset`, `length` and `@@toStringTag`.
pub(crate) const BUILTIN_TA_BUFFER: u32 = 100;
pub(crate) const BUILTIN_TA_BYTE_LENGTH: u32 = 101;
pub(crate) const BUILTIN_TA_BYTE_OFFSET: u32 = 102;
pub(crate) const BUILTIN_TA_LENGTH: u32 = 103;
pub(crate) const BUILTIN_TA_TAG: u32 = 104;
/// `Function.prototype[Symbol.hasInstance]`: `OrdinaryHasInstance(this, value)`.
pub(crate) const BUILTIN_FN_HAS_INSTANCE: u32 = 105;
/// `get` / `set` of `Object.prototype.__proto__`: `Object.getPrototypeOf(this)` /
/// `Object.setPrototypeOf(this, proto)` (a non-object proto is ignored).
pub(crate) const BUILTIN_OBJ_PROTO_GET: u32 = 106;
pub(crate) const BUILTIN_OBJ_PROTO_SET: u32 = 107;
/// The getter of the `Symbol.prototype.description` accessor.
pub(crate) const BUILTIN_SYMBOL_DESCRIPTION: u32 = 108;
pub(crate) const BUILTIN_MAP_SIZE: u32 = 16;
pub(crate) const BUILTIN_SET_SIZE: u32 = 17;
pub(crate) const BUILTIN_ARRAY_ITER_NEXT: u32 = 18;
pub(crate) const BUILTIN_STRING_ITER_NEXT: u32 = 19;
pub(crate) const BUILTIN_ITER_SELF: u32 = 20;
pub(crate) const BUILTIN_PROXY_REVOKE: u32 = 21;
pub(crate) const BUILTIN_STRING_ITERATOR: u32 = 22;
/// `get [Symbol.species]` of the built-in constructors: returns `this`.
pub(crate) const BUILTIN_SPECIES_GETTER: u32 = 23;
/// `Map.prototype[Symbol.iterator]` (the `entries` function) and
/// `Set.prototype[Symbol.iterator]` (the `values` function).
pub(crate) const BUILTIN_MAP_ITERATOR: u32 = 24;
pub(crate) const BUILTIN_SET_ITERATOR: u32 = 25;
/// `next` of the shared Set / Map iterator prototypes.
pub(crate) const BUILTIN_SETMAP_ITER_NEXT: u32 = 26;
/// `next` / `return` / `throw` of `%GeneratorPrototype%` (27, 28, 29).
pub(crate) const BUILTIN_GEN_NEXT: u32 = 27;
pub(crate) const BUILTIN_GEN_THROW: u32 = 29;
/// `Map.prototype.keys` and `Map.prototype.values`. Unlike Set's, these are
/// *not* the `@` function: `Map.prototype.keys` and `.values` are
/// three distinct function objects together with `.entries`.
pub(crate) const BUILTIN_MAP_KEYS: u32 = 30;
pub(crate) const BUILTIN_MAP_VALUES: u32 = 31;
/// First kind of the `Map.prototype` / `Set.prototype` method values; the kind
/// selects a row of [`COLLECTION_METHODS`].
pub(crate) const BUILTIN_COLLECTION_BASE: u32 = 64;
/// `(class, js name, native, extra args, returns this)`.
pub(crate) const COLLECTION_METHODS: [(&str, &str, &str, usize, bool); 9] = [
    ("Map", "get", "mapGet", 1, false),
    ("Map", "set", "mapSet", 2, true),
    ("Map", "has", "mapHas", 1, false),
    ("Map", "delete", "mapDelete", 1, false),
    ("Map", "clear", "mapClear", 0, false),
    ("Set", "add", "setAdd", 1, true),
    ("Set", "has", "setHas", 1, false),
    ("Set", "delete", "setDelete", 1, false),
    ("Set", "clear", "setClear", 0, false),
];
/// First kind of the `DataView.prototype` accessor method values; the kind
/// selects a row of [`DATA_VIEW_METHODS`].
pub(crate) const BUILTIN_DV_BASE: u32 = 32;
/// `(js name, native, element kind, is float)`; every accessor takes an optional
/// little-endian flag except the single-byte widths.
pub(crate) const DATA_VIEW_METHODS: [(&str, &str, zun_il::NumKind, bool); 16] = [
    ("getInt8", "dataViewGetInt8", zun_il::NumKind::I8, false),
    ("getUint8", "dataViewGetUint8", zun_il::NumKind::U8, false),
    ("getInt16", "dataViewGetInt16", zun_il::NumKind::I16, false),
    ("getUint16", "dataViewGetU16", zun_il::NumKind::U16, false),
    ("getInt32", "dataViewGetInt32", zun_il::NumKind::I32, false),
    ("getUint32", "dataViewGetU32", zun_il::NumKind::U32, false),
    ("getFloat32", "dataViewGetFloat32", zun_il::NumKind::F32, true),
    ("getFloat64", "dataViewGetF64", zun_il::NumKind::F64, true),
    ("setInt8", "dataViewSetInt8", zun_il::NumKind::I8, false),
    ("setUint8", "dataViewSetUint8", zun_il::NumKind::U8, false),
    ("setInt16", "dataViewSetInt16", zun_il::NumKind::I16, false),
    ("setUint16", "dataViewSetU16", zun_il::NumKind::U16, false),
    ("setInt32", "dataViewSetInt32", zun_il::NumKind::I32, false),
    ("setUint32", "dataViewSetU32", zun_il::NumKind::U32, false),
    ("setFloat32", "dataViewSetFloat32", zun_il::NumKind::F32, true),
    ("setFloat64", "dataViewSetF64", zun_il::NumKind::F64, true),
];

const OBJECT_PROTOTYPE_KEY: &str = "$Object.prototype";
const REGEXP_PROTOTYPE_KEY: &str = "$RegExp.prototype";

impl Interp<'_> {
    /// The realm's shared `Object.prototype` object: created on first use and
    /// anchored on the realm global object (so it is rooted and relocated with
    /// it).
    pub(crate) fn object_prototype_singleton() -> zun_gc::Obj {
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(existing) = heap
                .dynamic_get(global, OBJECT_PROTOTYPE_KEY)
                .and_then(|a| a.as_object())
            {
                return unsafe { zun_gc::Obj::from_addr(existing as *mut u64) };
            }
            let root = heap.alloc_dynamic();
            let any = zun_core::Any::object(root.addr() as usize);
            heap.dynamic_define(global, OBJECT_PROTOTYPE_KEY, any, false);
            crate::intrinsic_meta::install_prototype_methods(heap, root, "Object");
            heap.dynamic_define_accessor(
                root,
                "__proto__".to_owned(),
                Some(zun_core::FunctionHandle::new_builtin_method(BUILTIN_OBJ_PROTO_GET)),
                Some(zun_core::FunctionHandle::new_builtin_method(BUILTIN_OBJ_PROTO_SET)),
                false,
                true,
            );
            root
        })
    }

    /// The intrinsic `%TypedArray%.prototype`: owns the geometry getters and the
    /// `@@toStringTag` getter shared by every concrete typed-array kind.
    pub(crate) fn typed_array_prototype_singleton() -> zun_gc::Obj {
        let root = Self::object_prototype_singleton();
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(existing) = heap
                .dynamic_get(global, "$TypedArray.prototype")
                .and_then(|a| a.as_object())
            {
                return unsafe { zun_gc::Obj::from_addr(existing as *mut u64) };
            }
            let proto = heap.alloc_dynamic();
            heap.dynamic_set_prototype(proto, Some(zun_core::Any::object(root.addr() as usize)));
            let any = zun_core::Any::object(proto.addr() as usize);
            heap.dynamic_define(global, "$TypedArray.prototype", any, false);
            crate::intrinsic_meta::install_prototype_methods(heap, proto, "TypedArray");
            for (name, kind) in [
                ("buffer", BUILTIN_TA_BUFFER),
                ("byteLength", BUILTIN_TA_BYTE_LENGTH),
                ("byteOffset", BUILTIN_TA_BYTE_OFFSET),
                ("length", BUILTIN_TA_LENGTH),
            ] {
                heap.dynamic_define_accessor(
                    proto,
                    name.to_owned(),
                    Some(zun_core::FunctionHandle::new_builtin_method(kind)),
                    None,
                    false,
                    true,
                );
            }
            heap.dynamic_set_symbol_getter(
                proto,
                zun_core::WellKnownSymbol::ToStringTag.id(),
                zun_core::FunctionHandle::new_builtin_method(BUILTIN_TA_TAG),
            );
            proto
        })
    }

    /// The intrinsic `%TypedArray%` constructor stand-in: a non-global object
    /// named `TypedArray` whose `prototype` is the shared typed-array prototype
    /// and which inherits from `Function.prototype`.
    pub(crate) fn typed_array_intrinsic_singleton() -> zun_gc::Obj {
        let proto = Self::typed_array_prototype_singleton();
        let fn_proto = Self::function_prototype_singleton();
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(existing) = heap
                .dynamic_get(global, "$TypedArray")
                .and_then(|a| a.as_object())
            {
                return unsafe { zun_gc::Obj::from_addr(existing as *mut u64) };
            }
            let ctor = heap.alloc_dynamic();
            heap.dynamic_set_prototype(ctor, Some(zun_core::Any::object(fn_proto.addr() as usize)));
            let any = zun_core::Any::object(ctor.addr() as usize);
            heap.dynamic_define(global, "$TypedArray", any, false);
            let name = zun_core::Any::string(heap.intern_string("TypedArray"));
            heap.dynamic_define_data(ctor, "name", name, false, false, true);
            heap.dynamic_define_data(
                ctor,
                "prototype",
                zun_core::Any::object(proto.addr() as usize),
                false,
                false,
                false,
            );
            ctor
        })
    }

    /// The realm's shared `RegExp.prototype`: a dynamic object linked to
    /// `Object.prototype` that owns the built-in symbol methods
    /// (`@@match`, `@@matchAll`, `@@replace`, `@@search`, `@@split`).
    pub(crate) fn regexp_prototype_singleton() -> zun_gc::Obj {
        let root = Self::object_prototype_singleton();
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(existing) = heap
                .dynamic_get(global, REGEXP_PROTOTYPE_KEY)
                .and_then(|a| a.as_object())
            {
                return unsafe { zun_gc::Obj::from_addr(existing as *mut u64) };
            }
            let proto = heap.alloc_dynamic();
            heap.dynamic_set_prototype(proto, Some(zun_core::Any::object(root.addr() as usize)));
            for (kind, symbol) in [
                zun_core::WellKnownSymbol::Match,
                zun_core::WellKnownSymbol::MatchAll,
                zun_core::WellKnownSymbol::Replace,
                zun_core::WellKnownSymbol::Search,
                zun_core::WellKnownSymbol::Split,
            ]
            .into_iter()
            .enumerate()
            {
                heap.dynamic_set_symbol_method(
                    proto,
                    symbol.id(),
                    zun_core::FunctionHandle::new_builtin_method(kind as u32),
                );
            }
            let any = zun_core::Any::object(proto.addr() as usize);
            heap.dynamic_define(global, REGEXP_PROTOTYPE_KEY, any, false);
            crate::intrinsic_meta::install_prototype_methods(heap, proto, "RegExp");
            proto
        })
    }

    /// The realm's shared `String.prototype`: a dynamic object linked to
    /// `Object.prototype` owning `[Symbol.iterator]`.
    pub(crate) fn string_prototype_singleton() -> zun_gc::Obj {
        let root = Self::object_prototype_singleton();
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(existing) = heap
                .dynamic_get(global, "$String.prototype")
                .and_then(|a| a.as_object())
            {
                return unsafe { zun_gc::Obj::from_addr(existing as *mut u64) };
            }
            let proto = heap.alloc_dynamic();
            heap.dynamic_set_prototype(proto, Some(zun_core::Any::object(root.addr() as usize)));
            // `String.prototype` is itself a String object holding `""`.
            heap.dynamic_define_data(proto, "length", zun_core::Any::float(0.0), false, false, false);
            heap.dynamic_set_symbol_method(
                proto,
                zun_core::WellKnownSymbol::Iterator.id(),
                zun_core::FunctionHandle::new_builtin_method(BUILTIN_STRING_ITERATOR),
            );
            // `String.prototype.constructor` is `String`. Without the own
            // property the read fell through to `Object.prototype`'s.
            if let Some(ctor) = heap
                .dynamic_get(global, "String")
                .filter(|any| any.as_class_descriptor().is_some())
            {
                heap.dynamic_define_data(proto, "constructor", ctor, true, false, true);
            }
            let any = zun_core::Any::object(proto.addr() as usize);
            heap.dynamic_define(global, "$String.prototype", any, false);
            crate::intrinsic_meta::install_prototype_methods(heap, proto, "String");
            proto
        })
    }

    /// The realm's shared `Function.prototype`: a plain dynamic object linked to
    /// `Object.prototype`, the prototype of every built-in constructor value.
    pub(crate) fn function_prototype_singleton() -> zun_gc::Obj {
        let root = Self::object_prototype_singleton();
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(existing) = heap
                .dynamic_get(global, "$Function.prototype")
                .and_then(|a| a.as_object())
            {
                return unsafe { zun_gc::Obj::from_addr(existing as *mut u64) };
            }
            let proto = heap.alloc_dynamic();
            heap.dynamic_set_prototype(proto, Some(zun_core::Any::object(root.addr() as usize)));
            let any = zun_core::Any::object(proto.addr() as usize);
            heap.dynamic_define(global, "$Function.prototype", any, false);
            crate::intrinsic_meta::install_prototype_methods(heap, proto, "Function");
            heap.dynamic_set_symbol_method(
                proto,
                zun_core::WellKnownSymbol::HasInstance.id(),
                zun_core::FunctionHandle::new_builtin_method(BUILTIN_FN_HAS_INSTANCE),
            );
            // `Function.prototype[Symbol.hasInstance]` is the one well-known
            // method that is non-writable, non-enumerable and non-configurable.
            heap.dynamic_set_symbol_attributes(
                proto,
                zun_core::WellKnownSymbol::HasInstance.id(),
                Some(false),
                Some(false),
            );
            proto
        })
    }

    /// Whether `target` appears on the prototype chain of `start` (excluding
    /// `start` itself). Each step is `Object.getPrototypeOf`, so a typed
    /// instance walks its class-prototype links and an array its
    /// `Array.prototype`, exactly like a reflective read of the chain.
    pub(crate) fn proto_chain_contains(
        &mut self,
        start: zun_gc::Obj,
        target: zun_gc::Obj,
    ) -> Result<bool, RunError> {
        // `getPrototypeOf` can reach a proxy trap, i.e. managed code.
        zun_gc::root!(target = target);
        let mut current = start;
        for _ in 0..1024 {
            zun_gc::root!(held = current);
            let next = self.call_managed_collection(
                "objectGetPrototypeOf",
                &[Value::Object(held.get())],
            )?;
            let next = match next {
                Value::Any(any) => any.as_object(),
                Value::Object(obj) => Some(obj.addr() as usize),
                _ => None,
            };
            let Some(address) = next else {
                return Ok(false);
            };
            if address == target.get().addr() as usize {
                return Ok(true);
            }
            current = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
        }
        Ok(false)
    }

    /// TypeDef rid of the prelude class behind the realm `Error`-family
    /// constructor descriptor `token` (`Error`, `TypeError`, ...), if it is one.
    pub(crate) fn error_family_rid(&self, token: u32) -> Option<u32> {
        if token & zun_natives::USER_CLASS_FLAG != 0 {
            return None;
        }
        zun_gc::with_current_heap_mut(|heap| {
            crate::intrinsic_meta::builtin_class_name_in(heap, token)
        })
        .filter(|name| crate::intrinsic_meta::is_error_family(name))
        .and_then(|name| self.type_rid_of(&name))
    }

    /// The class descriptor a program sees for the class with TypeDef `rid`:
    /// the realm global's descriptor for a prelude `Error`-family class (so
    /// `Object.getPrototypeOf(Sub) === Error` compares equal to the global),
    /// else the user-class descriptor.
    pub(crate) fn canonical_class_descriptor(&self, rid: u32) -> zun_core::Any {
        let user = zun_core::Any::class_descriptor(zun_natives::USER_CLASS_FLAG | rid);
        let Some(name) = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
            .map(|def| self.module.strings.get(def.name).to_owned())
            .filter(|name| crate::intrinsic_meta::is_error_family(name))
        else {
            return user;
        };
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            heap.dynamic_get(global, &name)
        })
        .filter(|any| any.as_class_descriptor().is_some())
        .unwrap_or(user)
    }

    /// The real `Class.prototype` object of the user class with TypeDef `rid`:
    /// created on first use, anchored (non-enumerable) on the realm global so
    /// it is rooted, and linked to its base class's prototype.
    pub(crate) fn class_prototype_object(&self, rid: u32) -> zun_gc::Obj {
        // A primitive-wrapper class has no prototype of its own: its instances
        // (and classes derived from it) inherit the constructor's prototype.
        if let Some(proto) = self.wrapper_class_prototype(rid) {
            return proto;
        }
        self.prototype_keys
            .borrow_mut()
            .entry(rid)
            .or_insert_with(|| format!("$classProto#{rid}"));
        let existing = {
            let keys = self.prototype_keys.borrow();
            let key = keys.get(&rid).expect("prototype key cached above");
            zun_gc::with_current_heap_mut(|heap| {
                let global = heap.global_object();
                heap.dynamic_get(global, key).and_then(|a| a.as_object())
            })
        };
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
        let base = base_rid
            .filter(|&b| b >= 1)
            .map(|b| self.class_prototype_object(b));
        // The class's own instance accessors (`get_x` / `set_x` methods) become
        // accessor properties of the prototype object.
        let mut accessors: Vec<(
            String,
            Option<zun_core::FunctionHandle>,
            Option<zun_core::FunctionHandle>,
        )> = Vec::new();
        // Key -> position in `accessors`, so pairing a getter/setter is O(1)
        // while the `Vec` still preserves first-seen prototype property order.
        let mut plain_methods: Vec<(String, zun_core::FunctionHandle)> = Vec::new();
        let mut accessor_index: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        if let Some(def) = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
        {
            for method_rid in def.method_start..def.method_end {
                let Some(method) = self
                    .module
                    .tables
                    .method_defs
                    .get((method_rid as usize).wrapping_sub(1))
                else {
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
                    let ident = name
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
                        && !name.is_empty();
                    if ident && name != "constructor" && method.body_offset != u32::MAX {
                        plain_methods.push((
                            name.to_owned(),
                            zun_core::FunctionHandle::new(method_rid, None),
                        ));
                    }
                    continue;
                };
                let handle = zun_core::FunctionHandle::new(method_rid, None);
                let index = match accessor_index.get(prop) {
                    Some(&index) => index,
                    None => {
                        let index = accessors.len();
                        accessors.push((prop.to_owned(), None, None));
                        accessor_index.insert(prop.to_owned(), index);
                        index
                    }
                };
                if is_get {
                    accessors[index].1 = Some(handle);
                } else {
                    accessors[index].2 = Some(handle);
                }
            }
        }
        let key = self
            .prototype_keys
            .borrow()
            .get(&rid)
            .expect("prototype key cached above")
            .clone();
        let is_string_class = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
            .is_some_and(|def| self.module.strings.get(def.name) == "String");
        // `Number` is a prelude class whose prototype methods take a hidden receiver;
        // its prototype exposes the specified built-in method values instead.
        let is_number_class = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
            .is_some_and(|def| self.module.strings.get(def.name) == "Number");
        // The `Error` family owns data `name` and `message` on its prototype
        // (ECMAScript 20.5.3); they replace the managed `name` accessor.
        let error_class = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
            .map(|def| self.module.strings.get(def.name).to_owned())
            .filter(|name| crate::intrinsic_meta::is_error_family(name));
        zun_gc::with_current_heap_mut(|heap| {
            let proto = heap.alloc_dynamic();
            let any = zun_core::Any::object(proto.addr() as usize);
            let global = heap.global_object();
            heap.dynamic_define(global, key, any, false);
            if let Some(base) = base {
                heap.dynamic_set_prototype(
                    proto,
                    Some(zun_core::Any::object(base.addr() as usize)),
                );
            }
            // `Class.prototype.constructor` is writable (ES2024 15.7.14 step 12
            // defines it with `{ [[Writable]]: true, [[Enumerable]]: false,
            // [[Configurable]]: true }`), so a program may replace it. Defining
            // it read-only made `C.prototype.constructor = C` throw, which is
            // the idiom `lib/events.js` and every library modelled on it uses
            // to restore the entry after reassigning a prototype.
            // A prelude `Error`-family class is reached through its realm
            // global, so `Error.prototype.constructor === Error` keeps holding.
            let ctor = error_class
                .as_deref()
                .and_then(|class| heap.dynamic_get(global, class))
                .filter(|any| any.as_class_descriptor().is_some())
                .unwrap_or_else(|| {
                    zun_core::Any::class_descriptor(zun_natives::USER_CLASS_FLAG | rid)
                });
            heap.dynamic_define_data(proto, "constructor", ctor, true, false, true);
            for (name, get, set) in accessors {
                if error_class.is_some() && matches!(name.as_str(), "name" | "errorName" | "stack") {
                    continue;
                }
                heap.dynamic_define_accessor(proto, name, get, set, false, true);
            }
            if let Some(class) = &error_class {
                crate::intrinsic_meta::define_error_prototype_fields(heap, proto, class);
            }
            // Class methods are non-enumerable (ECMAScript ClassDefinitionEvaluation).
            for (name, handle) in plain_methods {
                heap.dynamic_set_method(proto, name, handle);
            }
            if is_number_class {
                crate::intrinsic_meta::install_prototype_methods(heap, proto, "Number");
            }
            if is_string_class {
                heap.dynamic_set_symbol_method(
                    proto,
                    zun_core::WellKnownSymbol::Iterator.id(),
                    zun_core::FunctionHandle::new_builtin_method(BUILTIN_STRING_ITERATOR),
                );
            }
            proto
        })
    }

    /// The lazily-created real `Builtin.prototype` object of a builtin class
    /// descriptor `token`: a plain dynamic object anchored (non-enumerable) on
    /// the realm global so it is rooted and identity-stable.
    pub(crate) fn builtin_prototype_object(&self, token: u32) -> zun_gc::Obj {
        // `Object.prototype` IS the realm's root prototype singleton -- the
        // object the typed fold answers with and the one
        // `Object.getPrototypeOf({})` returns. Building a separate one here
        // made `globalThis.Object.prototype === Object.prototype` false (each
        // path stable, the two not identical), which breaks realm identity for
        // the most fundamental prototype there is.
        if zun_gc::with_current_heap_mut(|heap| {
            crate::intrinsic_meta::builtin_class_name_in(heap, token).as_deref() == Some("Object")
        }) {
            return Self::object_prototype_singleton();
        }
        // The `Error` family is backed by prelude classes: `Error.prototype`
        // IS the class prototype object, so instances, subclasses and the
        // global constructor all share one link in the chain.
        if let Some(rid) = self.error_family_rid(token) {
            return self.class_prototype_object(rid);
        }
        let key = format!("$builtinProto#{token}");
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(address) = heap.dynamic_get(global, &key).and_then(|a| a.as_object()) {
                return unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
            }
            let proto = heap.alloc_dynamic();
            let any = zun_core::Any::object(proto.addr() as usize);
            heap.dynamic_define(global, key, any, false);
            // `Map.prototype.size` / `Set.prototype.size`: accessor properties
            // whose getters are real built-in function values.
            let is_class = |heap: &zun_gc::Heap, name: &str| {
                heap.dynamic_get(global, name)
                    .and_then(|a| a.as_class_descriptor())
                    == Some(token)
            };
            let size_kind = if is_class(heap, "Map") {
                Some((BUILTIN_MAP_SIZE, "Map"))
            } else if is_class(heap, "Set") {
                Some((BUILTIN_SET_SIZE, "Set"))
            } else {
                None
            };
            if zun_core::TypedElement::ALL.iter().any(|k| is_class(heap, k.js_name())) {
                let shared = Self::typed_array_prototype_singleton();
                heap.dynamic_set_prototype(proto, Some(zun_core::Any::object(shared.addr() as usize)));
            }
            if let Some(class) = crate::intrinsic_meta::builtin_class_name_in(heap, token)
                .filter(|name| crate::intrinsic_meta::is_error_family(name))
            {
                crate::intrinsic_meta::define_error_prototype_fields(heap, proto, &class);
            }
            // `Builtin.prototype.constructor` is the constructor itself (writable,
            // non-enumerable, configurable).
            heap.dynamic_define_data(
                proto,
                "constructor",
                zun_core::Any::class_descriptor(token),
                true,
                false,
                true,
            );
            if let Some(class) = crate::intrinsic_meta::builtin_class_name_in(heap, token) {
                crate::intrinsic_meta::install_prototype_methods(heap, proto, &class);
            }
            // `Symbol.prototype.description`: an accessor with a getter and no
            // setter (non-enumerable, configurable).
            if is_class(heap, "Symbol") {
                heap.dynamic_define_accessor(
                    proto,
                    "description".to_owned(),
                    Some(zun_core::FunctionHandle::new_builtin_method(BUILTIN_SYMBOL_DESCRIPTION)),
                    None,
                    false,
                    true,
                );
            }
            if is_class(heap, "String") {
                heap.dynamic_set_symbol_method(
                    proto,
                    zun_core::WellKnownSymbol::Iterator.id(),
                    zun_core::FunctionHandle::new_builtin_method(BUILTIN_STRING_ITERATOR),
                );
            }
            // `%Class%.prototype[Symbol.toStringTag]` (ECMAScript: a
            // non-writable, non-enumerable, configurable data property).
            for tagged in [
                "WeakMap", "WeakSet", "WeakRef", "FinalizationRegistry", "Promise", "ArrayBuffer",
                "DataView", "Symbol", "BigInt",
            ] {
                if is_class(heap, tagged) {
                    let tag = zun_core::Any::string(heap.intern_string(tagged));
                    let id = zun_core::WellKnownSymbol::ToStringTag.id();
                    heap.dynamic_define_symbol(proto, id, tag, false);
                    heap.dynamic_mark_symbol_readonly(proto, id);
                }
            }
            if let Some((kind, _)) = size_kind {
                let class = if kind == BUILTIN_MAP_SIZE { "Map" } else { "Set" };
                for (index, entry) in COLLECTION_METHODS.iter().enumerate() {
                    if entry.0 == class {
                        heap.dynamic_set_method(
                            proto,
                            entry.1.to_owned(),
                            zun_core::FunctionHandle::new_builtin_method(
                                BUILTIN_COLLECTION_BASE + index as u32,
                            ),
                        );
                    }
                }
            }
            if let Some((kind, tag)) = size_kind {
                heap.dynamic_set_symbol_method(
                    proto,
                    zun_core::WellKnownSymbol::Iterator.id(),
                    zun_core::FunctionHandle::new_builtin_method(if kind == BUILTIN_MAP_SIZE {
                        BUILTIN_MAP_ITERATOR
                    } else {
                        BUILTIN_SET_ITERATOR
                    }),
                );
                // The string-named iterator methods are the **same function**
                // as `@@iterator`, per spec: `Set.prototype.values` and
                // `.keys` are, and so is `Map.prototype.entries`. Only the
                // symbol was installed, so those names read as `undefined` --
                // which also made the neighbouring
                // `Set.prototype.keys === Set.prototype.values` true for the
                // wrong reason, both sides being `undefined`.
                let iter_handle = zun_core::FunctionHandle::new_builtin_method(if kind
                    == BUILTIN_MAP_SIZE
                {
                    BUILTIN_MAP_ITERATOR
                } else {
                    BUILTIN_SET_ITERATOR
                });
                if kind == BUILTIN_MAP_SIZE {
                    heap.dynamic_set_method(proto, "entries".to_owned(), iter_handle);
                    // `Map.prototype.keys` and `.values` are their own
                    // functions, not the `@` one: they used to be
                    // absent, which made `keys === values` true with both
                    // sides `undefined`.
                    heap.dynamic_set_method(
                        proto,
                        "keys".to_owned(),
                        zun_core::FunctionHandle::new_builtin_method(BUILTIN_MAP_KEYS),
                    );
                    heap.dynamic_set_method(
                        proto,
                        "values".to_owned(),
                        zun_core::FunctionHandle::new_builtin_method(BUILTIN_MAP_VALUES),
                    );
                } else {
                    heap.dynamic_set_method(proto, "values".to_owned(), iter_handle);
                    heap.dynamic_set_method(proto, "keys".to_owned(), iter_handle);
                }
                heap.dynamic_define_accessor(
                    proto,
                    "size".to_owned(),
                    Some(zun_core::FunctionHandle::new_builtin_method(kind)),
                    None,
                    false,
                    true,
                );
                let tag = zun_core::Any::string(heap.intern_string(tag));
                heap.dynamic_define_symbol(
                    proto,
                    zun_core::WellKnownSymbol::ToStringTag.id(),
                    tag,
                    false,
                );
                heap.dynamic_mark_symbol_readonly(proto, zun_core::WellKnownSymbol::ToStringTag.id());
            }
            proto
        })
    }

    /// The shared iterator prototype objects: `kind` 0 is `%IteratorPrototype%`,
    /// 1 `%ArrayIteratorPrototype%`, 2 `%StringIteratorPrototype%`. Created on
    /// first use, anchored (non-enumerable) on the realm global.
    pub(crate) fn iterator_prototype_object(kind: u8) -> zun_gc::Obj {
        let base = if kind == 0 {
            None
        } else {
            Some(Self::iterator_prototype_object(0))
        };
        let root = Self::object_prototype_singleton();
        let key = format!("$iteratorProto#{kind}");
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(address) = heap.dynamic_get(global, &key).and_then(|a| a.as_object()) {
                return unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
            }
            let proto = heap.alloc_dynamic();
            let any = zun_core::Any::object(proto.addr() as usize);
            heap.dynamic_define(global, key, any, false);
            let link = base.unwrap_or(root);
            heap.dynamic_set_prototype(proto, Some(zun_core::Any::object(link.addr() as usize)));
            match kind {
                0 => {
                    heap.dynamic_set_symbol_method(
                        proto,
                        zun_core::WellKnownSymbol::Iterator.id(),
                        zun_core::FunctionHandle::new_builtin_method(BUILTIN_ITER_SELF),
                    );
                    for (index, entry) in super::collections::iterator_helpers::ITER_HELPERS
                        .iter()
                        .enumerate()
                    {
                        heap.dynamic_set_function(
                            proto,
                            entry.0.to_owned(),
                            zun_core::FunctionHandle::new_builtin_method(
                                super::collections::iterator_helpers::BUILTIN_ITER_HELPER_BASE
                                    + index as u32,
                            ),
                        );
                    }
                    let tag = zun_core::Any::string(heap.intern_string("Iterator"));
                    heap.dynamic_define_symbol(
                        proto,
                        zun_core::WellKnownSymbol::ToStringTag.id(),
                        tag,
                        false,
                    );
                }
                7 => {
                    for (offset, name) in ["next", "return", "throw"].iter().enumerate() {
                        heap.dynamic_set_function(
                            proto,
                            (*name).to_owned(),
                            zun_core::FunctionHandle::new_builtin_method(
                                BUILTIN_GEN_NEXT + offset as u32,
                            ),
                        );
                    }
                    let tag = zun_core::Any::string(heap.intern_string("Generator"));
                    heap.dynamic_define_symbol(
                        proto,
                        zun_core::WellKnownSymbol::ToStringTag.id(),
                        tag,
                        false,
                    );
                }
                5 | 6 => {
                    heap.dynamic_set_function(
                        proto,
                        "next".to_owned(),
                        zun_core::FunctionHandle::new_builtin_method(BUILTIN_SETMAP_ITER_NEXT),
                    );
                    let tag = zun_core::Any::string(
                        heap.intern_string(if kind == 5 { "Set Iterator" } else { "Map Iterator" }),
                    );
                    heap.dynamic_define_symbol(
                        proto,
                        zun_core::WellKnownSymbol::ToStringTag.id(),
                        tag,
                        false,
                    );
                }
                3 | 4 => {
                    use super::collections::iterator_helpers as ih;
                    let (next, ret) = if kind == 3 {
                        (ih::BUILTIN_ITERH_NEXT, ih::BUILTIN_ITERH_RETURN)
                    } else {
                        (ih::BUILTIN_WRAP_NEXT, ih::BUILTIN_WRAP_RETURN)
                    };
                    heap.dynamic_set_function(
                        proto,
                        "next".to_owned(),
                        zun_core::FunctionHandle::new_builtin_method(next),
                    );
                    heap.dynamic_set_function(
                        proto,
                        "return".to_owned(),
                        zun_core::FunctionHandle::new_builtin_method(ret),
                    );
                    if kind == 3 {
                        let tag = zun_core::Any::string(heap.intern_string("Iterator Helper"));
                        heap.dynamic_define_symbol(
                            proto,
                            zun_core::WellKnownSymbol::ToStringTag.id(),
                            tag,
                            false,
                        );
                    }
                }
                1 | 2 => {
                    let (next, tag) = if kind == 1 {
                        (BUILTIN_ARRAY_ITER_NEXT, "Array Iterator")
                    } else {
                        (BUILTIN_STRING_ITER_NEXT, "String Iterator")
                    };
                    heap.dynamic_set_function(
                        proto,
                        "next".to_owned(),
                        zun_core::FunctionHandle::new_builtin_method(next),
                    );
                    let tag = zun_core::Any::string(heap.intern_string(tag));
                    heap.dynamic_define_symbol(
                        proto,
                        zun_core::WellKnownSymbol::ToStringTag.id(),
                        tag,
                        false,
                    );
                }
                _ => {}
            }
            proto
        })
    }

    fn incompatible_receiver(&mut self, what: &str) -> RunError {
        let message = format!("Method {what} called on incompatible receiver");
        match self.new_error_object("TypeError", &message) {
            Some(object) => RunError::Thrown(Value::Object(object)),
            None => RunError::Native(format!("TypeError: {message}")),
        }
    }

    /// Run a built-in method value (kind >= 16) with its receiver and
    /// arguments. A receiver of the wrong brand throws a `TypeError`.
    pub(crate) fn invoke_builtin_with_this(
        &mut self,
        handle: zun_core::FunctionHandle,
        this: Value,
        _args: Vec<Value>,
    ) -> Result<Value, RunError> {
        let kind = handle.builtin_method().unwrap_or(0);
        let this_obj = match &this {
            Value::Object(obj) => Some(*obj),
            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                .and_then(|v| match v {
                    Value::Object(obj) => Some(obj),
                    _ => None,
                }),
            _ => None,
        };
        match kind {
            BUILTIN_SYMBOL_DESCRIPTION => {
                let symbol = self.this_symbol_value(&this);
                let Some(symbol) = symbol else {
                    let message = "Symbol.prototype.description requires that 'this' be a Symbol";
                    return Err(match self.new_error_object("TypeError", message) {
                        Some(object) => RunError::Thrown(Value::Object(object)),
                        None => RunError::Native(format!("TypeError: {message}")),
                    });
                };
                let description = zun_gc::with_current_heap(|heap| {
                    heap.symbol_description(symbol).map(|text| text.to_string())
                });
                Ok(description.map_or(Value::Void, |text| Value::Str(text.into())))
            }
            BUILTIN_MAP_SIZE => {
                let len = this_obj
                    .and_then(|o| zun_gc::with_current_heap(|heap| heap.map_len(o)))
                    .ok_or_else(|| self.incompatible_receiver("get Map.prototype.size"))?;
                Ok(Value::Float(len as f64, zun_il::NumKind::F64))
            }
            BUILTIN_SET_SIZE => {
                let len = this_obj
                    .and_then(|o| zun_gc::with_current_heap(|heap| heap.set_len(o)))
                    .ok_or_else(|| self.incompatible_receiver("get Set.prototype.size"))?;
                Ok(Value::Float(len as f64, zun_il::NumKind::F64))
            }
            BUILTIN_ARRAY_ITER_NEXT | BUILTIN_STRING_ITER_NEXT => {
                let want_string = kind == BUILTIN_STRING_ITER_NEXT;
                let what = if want_string {
                    "String Iterator.prototype.next"
                } else {
                    "Array Iterator.prototype.next"
                };
                let it = this_obj
                    .filter(|o| {
                        zun_gc::with_current_heap(|heap| heap.array_iterator_mode(*o))
                            .is_some_and(|mode| (mode == 3) == want_string)
                    })
                    .ok_or_else(|| self.incompatible_receiver(what))?;
                match self.builtin_iterator_result(it)? {
                    Some(result) => Ok(result),
                    None => Err(self.incompatible_receiver(what)),
                }
            }
            BUILTIN_TA_BUFFER..=BUILTIN_TA_TAG => {
                let typed = this_obj.filter(|o| {
                    zun_gc::with_current_heap(|heap| heap.typed_array_element(*o).is_some())
                });
                if kind == BUILTIN_TA_TAG {
                    return Ok(match typed {
                        Some(o) => {
                            let name = zun_gc::with_current_heap(|heap| {
                                heap.typed_array_element(o).map(|k| k.js_name())
                            })
                            .unwrap_or("");
                            Value::Str(name.into())
                        }
                        None => Value::Void,
                    });
                }
                let label = match kind {
                    BUILTIN_TA_BUFFER => "buffer",
                    BUILTIN_TA_BYTE_LENGTH => "byteLength",
                    BUILTIN_TA_BYTE_OFFSET => "byteOffset",
                    _ => "length",
                };
                let o = typed.ok_or_else(|| {
                    self.incompatible_receiver(&format!("get TypedArray.prototype.{label}"))
                })?;
                let num = |n: Option<usize>| {
                    Value::Float(n.unwrap_or(0) as f64, zun_il::NumKind::F64)
                };
                Ok(zun_gc::with_current_heap(|heap| match kind {
                    BUILTIN_TA_BUFFER => heap
                        .typed_array_buffer(o)
                        .map(Value::Object)
                        .unwrap_or(Value::Void),
                    BUILTIN_TA_BYTE_LENGTH => num(heap.typed_array_byte_len(o)),
                    BUILTIN_TA_BYTE_OFFSET => num(heap.typed_array_byte_offset(o)),
                    _ => num(heap.typed_array_len(o)),
                }))
            }
            BUILTIN_MAP_KEYS | BUILTIN_MAP_VALUES => {
                let native = if kind == BUILTIN_MAP_KEYS { "mapKeys" } else { "mapValues" };
                let receiver = this_obj.ok_or_else(|| {
                    self.incompatible_receiver(if kind == BUILTIN_MAP_KEYS {
                        "Map.prototype.keys"
                    } else {
                        "Map.prototype.values"
                    })
                })?;
                self.call_managed_collection(native, &[Value::Object(receiver)])
            }
            BUILTIN_SETMAP_ITER_NEXT => {
                let it = this_obj
                    .filter(|o| {
                        zun_gc::with_current_heap(|heap| {
                            heap.set_iterator_done(*o).is_some() || heap.map_iterator_done(*o).is_some()
                        })
                    })
                    .ok_or_else(|| self.incompatible_receiver("Set/Map Iterator.prototype.next"))?;
                match self.builtin_iterator_result(it)? {
                    Some(result) => Ok(result),
                    None => Err(self.incompatible_receiver("Set/Map Iterator.prototype.next")),
                }
            }
            BUILTIN_GEN_NEXT..=BUILTIN_GEN_THROW => {
                let generator = this_obj
                    .filter(|o| zun_gc::with_current_heap(|heap| heap.generator_state(*o).is_some()))
                    .ok_or_else(|| self.incompatible_receiver("Generator.prototype method"))?;
                self.call_generator_method(kind - BUILTIN_GEN_NEXT, generator, &_args)
            }
            BUILTIN_OBJ_PROTO_GET => {
                if matches!(unbox_arg(&this), Ok(Value::Void) | Ok(Value::Null)) {
                    return Err(self.incompatible_receiver("get Object.prototype.__proto__"));
                }
                self.call_managed_collection("objectGetPrototypeOf", &[this])
            }
            BUILTIN_OBJ_PROTO_SET => {
                let receiver = unbox_arg(&this).unwrap_or(Value::Void);
                if matches!(receiver, Value::Void | Value::Null) {
                    return Err(self.incompatible_receiver("set Object.prototype.__proto__"));
                }
                let proto = _args.first().and_then(|v| unbox_arg(v).ok()).unwrap_or(Value::Void);
                // Only an object or null is a valid prototype; anything else, and a
                // primitive receiver, is silently ignored.
                let valid = matches!(proto, Value::Object(_) | Value::Null)
                    || matches!(&proto, Value::Any(any) if any.as_object().is_some() || matches!(any.tag(), zun_core::AnyTag::Null));
                if valid && matches!(receiver, Value::Object(_)) {
                    self.call_managed_collection("objectSetPrototypeOf", &[this, proto])?;
                }
                Ok(Value::Void)
            }
            BUILTIN_ITER_SELF | BUILTIN_SPECIES_GETTER => Ok(this),
            BUILTIN_FN_HAS_INSTANCE => {
                // `OrdinaryHasInstance(C, O)`: a non-callable `this`, or a
                // non-object operand, answers false.
                let Ok(Value::Function(target)) = unbox_arg(&this) else {
                    return Ok(Value::Bool(false));
                };
                let operand = _args.first().cloned().unwrap_or(Value::Void);
                Ok(Value::Bool(self.function_instanceof(&operand, target)?))
            }
            k if (super::collections::iterator_helpers::BUILTIN_ITER_HELPER_BASE
                ..=super::collections::iterator_helpers::BUILTIN_WRAP_RETURN)
                .contains(&k) =>
            {
                self.iterator_builtin_call(k, this, _args)
            }
            BUILTIN_MAP_ITERATOR | BUILTIN_SET_ITERATOR => {
                let want_map = kind == BUILTIN_MAP_ITERATOR;
                let what = if want_map {
                    "Map.prototype.entries"
                } else {
                    "Set.prototype.values"
                };
                let source = this_obj
                    .filter(|o| {
                        zun_gc::with_current_heap(|heap| {
                            if want_map {
                                heap.map_len(*o).is_some()
                            } else {
                                heap.set_len(*o).is_some()
                            }
                        })
                    })
                    .ok_or_else(|| self.incompatible_receiver(what))?;
                Ok(Value::Object(self.resolve_iterator(source)?))
            }
            BUILTIN_STRING_ITERATOR => {
                // The receiver is `ToString`'d, not required to be a string:
                // `String.prototype[Symbol.iterator].call(123)` iterates
                // `"123"` in node. Only a nullish receiver is a TypeError
                // (RequireObjectCoercible); every other kind converts, which
                // `stringIteratorNew` now does.
                if matches!(this, Value::Null | Value::Void) {
                    return Err(self.incompatible_receiver("String.prototype[Symbol.iterator]"));
                }
                self.call_managed_collection("stringIteratorNew", &[this])
            }
            k if (BUILTIN_DV_BASE..BUILTIN_DV_BASE + DATA_VIEW_METHODS.len() as u32).contains(&k) => {
                let (js_name, native, elem, is_float) =
                    DATA_VIEW_METHODS[(k - BUILTIN_DV_BASE) as usize];
                let view = this_obj
                    .filter(|o| zun_gc::with_current_heap(|heap| heap.data_view_len(*o).is_some()))
                    .ok_or_else(|| {
                        self.incompatible_receiver(&format!("DataView.prototype.{js_name}"))
                    })?;
                let is_set = js_name.starts_with("set");
                let number = |this: &mut Self, index: usize| -> Result<f64, RunError> {
                    let raw = match _args.get(index) {
                        Some(Value::Any(any)) => {
                            zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                .unwrap_or(Value::Void)
                        }
                        Some(other) => other.clone(),
                        None => Value::Void,
                    };
                    this.to_number_value(raw)
                };
                let offset = number(self, 0)?;
                let offset = if offset.is_nan() { 0.0 } else { offset.trunc() };
                let mut call = vec![
                    Value::Object(view),
                    Value::Int(offset.max(0.0) as i64, zun_il::NumKind::U64),
                ];
                if is_set {
                    let value = number(self, 1)?;
                    call.push(if is_float {
                        Value::Float(value, elem)
                    } else {
                        let wrapped = if value.is_finite() { value.trunc() as i128 as i64 } else { 0 };
                        Value::Int(wrapped, elem)
                    });
                }
                let wants_endian = !matches!(elem, zun_il::NumKind::I8 | zun_il::NumKind::U8);
                if wants_endian {
                    let flag = _args.get(if is_set { 2 } else { 1 }).cloned().unwrap_or(Value::Void);
                    let flag = match flag {
                        Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                            .unwrap_or(Value::Void),
                        other => other,
                    };
                    call.push(Value::Bool(zun_gc::with_current_heap(|heap| {
                        crate::collections::is_truthy(&flag, heap)
                    })));
                }
                let result = self.call_managed_collection(native, &call)?;
                Ok(match result {
                    Value::Int(n, _) => Value::Float(n as f64, zun_il::NumKind::F64),
                    Value::Bool(_) => Value::Void,
                    other => other,
                })
            }
            k if k >= crate::intrinsic_meta::BUILTIN_STATIC_BASE => {
                self.call_static_method(k, this, _args)
            }
            k if k >= crate::intrinsic_meta::BUILTIN_PROTO_BASE => {
                self.call_prototype_method(k, this, _args)
            }
            k if (BUILTIN_COLLECTION_BASE..BUILTIN_COLLECTION_BASE + COLLECTION_METHODS.len() as u32)
                .contains(&k) =>
            {
                let (class, js_name, native, count, returns_this) =
                    COLLECTION_METHODS[(k - BUILTIN_COLLECTION_BASE) as usize];
                let is_map = class == "Map";
                // An instance of a `Map` / `Set` subclass is a typed object
                // holding the native collection in its `inner` field: the
                // built-in method borrowed with `.call` operates on that
                // collection, and a method that returns `this` returns the
                // instance.
                let outer = this_obj;
                let this_obj = match this_obj {
                    Some(instance) => Some(self.collection_subclass_inner(instance, is_map)?),
                    None => None,
                };
                let receiver = this_obj
                    .filter(|o| {
                        zun_gc::with_current_heap(|heap| {
                            if is_map {
                                heap.map_len(*o).is_some()
                            } else {
                                heap.set_len(*o).is_some()
                            }
                        })
                    })
                    .ok_or_else(|| {
                        self.incompatible_receiver(&format!("{class}.prototype.{js_name}"))
                    })?;
                let mut call = vec![Value::Object(receiver)];
                for index in 0..count {
                    let arg = _args.get(index).cloned().unwrap_or(Value::Void);
                    let boxed = match arg {
                        Value::Any(any) => any,
                        other => zun_gc::with_current_heap_mut(|heap| other.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void),
                    };
                    call.push(Value::Any(boxed));
                }
                let result = self.call_managed_collection(native, &call)?;
                Ok(if returns_this {
                    Value::Object(outer.unwrap_or(receiver))
                } else {
                    match result {
                        Value::Bool(b) => Value::Bool(b),
                        Value::Void => Value::Void,
                        other => other,
                    }
                })
            }
            BUILTIN_PROXY_REVOKE => {
                if let Some(address) = handle.environment() {
                    let proxy = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
                    zun_gc::with_current_heap_mut(|heap| heap.proxy_revoke(proxy));
                }
                Ok(Value::Void)
            }
            _ => Err(RunError::Native(
                "TypeError: RegExp.prototype method called on an incompatible receiver".into(),
            )),
        }
    }

    /// `name` / `length` of a built-in method value (kind >= 16).
    pub(crate) fn builtin_method_meta(kind: u32, name: &str) -> Option<Value> {
        if let Some((_, method, length, _)) = crate::intrinsic_meta::static_method_at(kind) {
            return Some(if name == "name" {
                Value::Str(method.into())
            } else {
                Value::Float(f64::from(length), zun_il::NumKind::F64)
            });
        }
        if let Some((_, method, length)) = crate::intrinsic_meta::prototype_method_at(kind) {
            return Some(if name == "name" {
                Value::Str(method.into())
            } else {
                Value::Float(f64::from(length), zun_il::NumKind::F64)
            });
        }
        let label = match kind {
            BUILTIN_MAP_SIZE | BUILTIN_SET_SIZE => "get size",
            BUILTIN_SYMBOL_DESCRIPTION => "get description",
            BUILTIN_ARRAY_ITER_NEXT | BUILTIN_STRING_ITER_NEXT | BUILTIN_SETMAP_ITER_NEXT | BUILTIN_GEN_NEXT => "next",
            28 => "return",
            BUILTIN_GEN_THROW => "throw",
            BUILTIN_ITER_SELF => "[Symbol.iterator]",
            BUILTIN_PROXY_REVOKE => "",
            BUILTIN_STRING_ITERATOR => "[Symbol.iterator]",
            BUILTIN_FN_HAS_INSTANCE => {
                return Some(if name == "name" {
                    Value::Str("[Symbol.hasInstance]".into())
                } else {
                    Value::Float(1.0, zun_il::NumKind::F64)
                });
            }
            BUILTIN_SPECIES_GETTER => "get [Symbol.species]",
            BUILTIN_TA_BUFFER => "get buffer",
            BUILTIN_TA_BYTE_LENGTH => "get byteLength",
            BUILTIN_TA_BYTE_OFFSET => "get byteOffset",
            BUILTIN_TA_LENGTH => "get length",
            BUILTIN_TA_TAG => "get [Symbol.toStringTag]",
            BUILTIN_OBJ_PROTO_GET => "get __proto__",
            BUILTIN_OBJ_PROTO_SET => {
                return Some(if name == "name" {
                    Value::Str("set __proto__".into())
                } else {
                    Value::Float(1.0, zun_il::NumKind::F64)
                });
            }
            k if (BUILTIN_COLLECTION_BASE..BUILTIN_COLLECTION_BASE + COLLECTION_METHODS.len() as u32)
                .contains(&k) =>
            {
                let entry = COLLECTION_METHODS[(k - BUILTIN_COLLECTION_BASE) as usize];
                return Some(if name == "name" {
                    Value::Str(entry.1.into())
                } else {
                    Value::Float(entry.3 as f64, zun_il::NumKind::F64)
                });
            }
            k if (super::collections::iterator_helpers::BUILTIN_ITER_HELPER_BASE
                ..=super::collections::iterator_helpers::BUILTIN_WRAP_RETURN)
                .contains(&k) =>
            {
                use super::collections::iterator_helpers as ih;
                let (label, arity) = match k {
                    ih::BUILTIN_ITER_FROM => ("from", 1),
                    ih::BUILTIN_ITERH_NEXT | ih::BUILTIN_WRAP_NEXT => ("next", 0),
                    ih::BUILTIN_ITERH_RETURN | ih::BUILTIN_WRAP_RETURN => ("return", 0),
                    other => ih::ITER_HELPERS[(other - ih::BUILTIN_ITER_HELPER_BASE) as usize],
                };
                return Some(if name == "name" {
                    Value::Str(label.into())
                } else {
                    Value::Float(arity as f64, zun_il::NumKind::F64)
                });
            }
            BUILTIN_MAP_ITERATOR => "entries",
            BUILTIN_SET_ITERATOR => "values",
            k if (BUILTIN_DV_BASE..BUILTIN_DV_BASE + DATA_VIEW_METHODS.len() as u32).contains(&k) => {
                let entry = DATA_VIEW_METHODS[(k - BUILTIN_DV_BASE) as usize];
                if name == "name" {
                    return Some(Value::Str(entry.0.into()));
                }
                let single = matches!(entry.2, zun_il::NumKind::I8 | zun_il::NumKind::U8);
                let fixed = if entry.0.starts_with("set") { 2.0 } else { 1.0 };
                let _ = single;
                return Some(Value::Float(fixed, zun_il::NumKind::F64));
            }
            _ => return None,
        };
        Some(if name == "name" {
            Value::Str(label.into())
        } else {
            Value::Float(0.0, zun_il::NumKind::F64)
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

    /// The setter of a symbol-keyed accessor reachable from `object`: its own
    /// slot first, then the class prototype chain of a typed instance, then the
    /// dynamic prototype chain.
    pub(crate) fn symbol_setter_lookup(
        &self,
        object: zun_gc::Obj,
        symbol: zun_core::SymbolId,
    ) -> Option<zun_core::FunctionHandle> {
        let mut current = Some(object);
        let mut class_proto = self.instance_class_prototype(object);
        for _ in 0..256 {
            let at = current?;
            let step = zun_gc::with_current_heap(|heap| {
                if let Some(setter) = heap.dynamic_symbol_setter(at, symbol) {
                    return Err(Some(setter));
                }
                if heap.dynamic_has_symbol_key(at, symbol) {
                    // A data property or getter-only accessor shadows the chain.
                    return Err(None);
                }
                Ok(heap
                    .dynamic_prototype(at)
                    .and_then(|any| any.as_object())
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }))
            });
            match step {
                Err(found) => return found,
                Ok(next) => current = class_proto.take().or(next),
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
            let def = self
                .module
                .tables
                .type_defs
                .get((r as usize).wrapping_sub(1))?;
            for mrid in def.method_start..def.method_end {
                let Some(method) = self
                    .module
                    .tables
                    .method_defs
                    .get((mrid as usize).wrapping_sub(1))
                else {
                    continue;
                };
                let method_name = self.module.strings.get(method.name);
                let (plain, hidden) = match method_name.strip_suffix("@this") {
                    Some(p) => (p, true),
                    None => (method_name, false),
                };
                // `name@static`: a static that shares its name with an
                // instance method of the class (see `static_twins` in sema).
                let plain = plain.strip_suffix("@static").unwrap_or(plain);
                if method.is_static && plain == name {
                    return Some((mrid, hidden));
                }
            }
            current = self
                .class_static_proto_override(r)
                .or_else(|| {
                    (def.base.table() == Some(zun_metadata::TableKind::TypeDef)
                        && !def.base.is_null())
                    .then(|| def.base.rid())
                });
        }
        None
    }

    /// `delete Class.name` / `Reflect.deleteProperty(Class, name)`: drop the
    /// class's own initialized static field slot so reads fall through to the
    /// base chain. Intrinsic `prototype` is non-configurable.
    pub(crate) fn class_delete_own(&mut self, rid: u32, name: &str) -> bool {
        if name == "prototype" {
            return false;
        }
        let Some(def) = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
            .cloned()
        else {
            return true;
        };
        for frid in def.field_start..def.field_end {
            let matches = self
                .module
                .tables
                .field_defs
                .get((frid as usize).wrapping_sub(1))
                .is_some_and(|f| f.is_static && !name.starts_with('#') && self.module.strings.get(f.name) == name);
            if matches {
                zun_gc::with_current_heap_mut(|heap| heap.static_remove(rid, frid));
            }
        }
        true
    }

    /// A static read whose own slot is unset (deleted, or never initialised on
    /// this class): the same-named static of the nearest base class that has
    /// one, as the prototype chain of constructors would answer.
    pub(crate) fn static_inherited_read(&self, owner: u32, field: u32) -> Option<Value> {
        let name = self
            .module
            .strings
            .get(self.module.tables.field_defs.get((field as usize).wrapping_sub(1))?.name)
            .to_owned();
        let kind = self
            .module
            .tables
            .field_defs
            .get((field as usize).wrapping_sub(1))?
            .kind;
        let mut current = self
            .module
            .tables
            .type_defs
            .get((owner as usize).wrapping_sub(1))
            .and_then(|def| {
                (def.base.table() == Some(zun_metadata::TableKind::TypeDef)
                    && !def.base.is_null())
                .then(|| def.base.rid())
            });
        for _ in 0..256 {
            let r = current?;
            let def = self.module.tables.type_defs.get((r as usize).wrapping_sub(1))?;
            for frid in def.field_start..def.field_end {
                let Some(f) = self.module.tables.field_defs.get((frid as usize).wrapping_sub(1))
                else {
                    continue;
                };
                if f.is_static && !name.starts_with('#') && self.module.strings.get(f.name) == name {
                    let hit = zun_gc::with_current_heap(|heap| {
                        if let Some(any) = heap.static_get(r, frid) {
                            let value = Value::unbox_any(any, heap).unwrap_or(Value::Void);
                            Some(crate::value::retag_static_numeric(value, kind))
                        } else {
                            heap.static_get_function(r, frid).map(Value::Function)
                        }
                    });
                    if hit.is_some() {
                        return hit;
                    }
                }
            }
            current = (def.base.table() == Some(zun_metadata::TableKind::TypeDef)
                && !def.base.is_null())
            .then(|| def.base.rid());
        }
        None
    }

    /// The class descriptor token carried by a value (a class value read
    /// through `any`), user-defined or built-in.
    pub(crate) fn class_token_of(value: &Value) -> Option<u32> {
        match value {
            Value::Any(any) => {
                if let Some(token) = any.as_class_descriptor() {
                    return Some(token);
                }
                match zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)) {
                    Some(Value::Any(inner)) => inner.as_class_descriptor(),
                    Some(Value::Object(record)) => Self::constructor_record_token(record),
                    _ => None,
                }
            }
            Value::Object(record) => Self::constructor_record_token(*record),
            _ => None,
        }
    }

    /// The class a constructor record names. `instance.constructor` of a typed
    /// instance is a record object (`{ name, __classDesc }`), not the class
    /// descriptor itself, so a caller holding one still resolves the class.
    fn constructor_record_token(record: zun_gc::Obj) -> Option<u32> {
        zun_gc::with_current_heap(|heap| {
            if !heap.is_dynamic_object(record) {
                return None;
            }
            heap.dynamic_get(record, "__classDesc")
                .and_then(|descriptor| descriptor.as_class_descriptor())
        })
    }

    /// The expando bag of a class value: a plain dynamic object, anchored on the
    /// realm global, that holds the properties scripts define on a constructor
    /// (`Reflect.defineProperty(Cls, ...)`) beyond its declared statics.
    pub(crate) fn class_expando_bag(token: u32) -> zun_gc::Obj {
        let key = format!("$classBag#{token}");
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            if let Some(address) = heap.dynamic_get(global, &key).and_then(|a| a.as_object()) {
                return unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
            }
            let bag = heap.alloc_dynamic();
            let any = zun_core::Any::object(bag.addr() as usize);
            heap.dynamic_define(global, key, any, false);
            bag
        })
    }

    /// The expando bag of a class value when one exists (never created here).
    pub(crate) fn class_expando_peek(token: u32) -> Option<zun_gc::Obj> {
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            let address = heap
                .dynamic_get(global, &format!("$classBag#{token}"))
                .and_then(|a| a.as_object())?;
            Some(unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
        })
    }

    /// Read a property a script defined on a class value (its expando bag)
    /// under a string or symbol `key`: a function slot or a data value. A
    /// class whose bag was never created has nothing to read, and the lookup
    /// does not create it.
    pub(crate) fn class_expando_read_own(token: u32, key: &Value) -> Option<Value> {
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            let address = heap
                .dynamic_get(global, &format!("$classBag#{token}"))
                .and_then(|a| a.as_object())?;
            let bag = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
            match key {
                Value::Str(name) => heap
                    .dynamic_get_function(bag, name)
                    .map(Value::Function)
                    .or_else(|| {
                        heap.dynamic_get(bag, name)
                            .and_then(|any| Value::unbox_any(any, heap))
                    }),
                Value::Any(any) => {
                    let symbol = any.as_symbol()?;
                    heap.dynamic_get_symbol_function(bag, symbol)
                        .map(Value::Function)
                        .or_else(|| {
                            heap.dynamic_get_symbol(bag, symbol)
                                .and_then(|any| Value::unbox_any(any, heap))
                        })
                }
                _ => None,
            }
        })
    }

    /// [`Self::class_expando_read_own`] on the class, then on each base class
    /// (a static member is inherited by the subclass constructor).
    pub(crate) fn class_expando_read(&self, token: u32, key: &Value) -> Option<Value> {
        let mut rid = self.user_class_rid(token);
        let mut current = token;
        for _ in 0..64 {
            if let Some(found) = Self::class_expando_read_own(current, key) {
                return Some(found);
            }
            let def = self.module.tables.type_defs.get((rid? as usize).wrapping_sub(1))?;
            if def.base.table() != Some(zun_metadata::TableKind::TypeDef) || def.base.is_null() {
                return None;
            }
            rid = Some(def.base.rid());
            current = def.base.rid() | zun_natives::USER_CLASS_FLAG;
        }
        None
    }

    /// `Object.hasOwn(Class, name)`: whether the class value itself (not its
    /// bases) owns the static field, static method or accessor `name`, or one
    /// of the intrinsic function properties.
    pub(crate) fn class_has_own(&self, rid: u32, name: &str) -> bool {
        if matches!(name, "name" | "length" | "prototype") {
            return true;
        }
        // A value stored for an *inherited* static field is an own property of
        // this class: the storage is keyed by (class, declaring field), which
        // is how `Sub.count = v` keeps the base's value untouched.
        if let Some((owner, frid)) = self.static_field_origin(rid, name) {
            if owner != rid
                && zun_gc::with_current_heap(|heap| {
                    heap.static_get(rid, frid).is_some()
                        || heap.static_get_function(rid, frid).is_some()
                })
            {
                return true;
            }
        }
        let Some(def) = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
        else {
            return false;
        };
        for frid in def.field_start..def.field_end {
            let Some(field) = self
                .module
                .tables
                .field_defs
                .get((frid as usize).wrapping_sub(1))
            else {
                continue;
            };
            if field.is_static
                && !name.starts_with('#') && self.module.strings.get(field.name) == name
                && zun_gc::with_current_heap(|heap| {
                    heap.static_get(rid, frid).is_some()
                        || heap.static_get_function(rid, frid).is_some()
                })
            {
                return true;
            }
        }
        let getter = format!("get_{name}");
        let setter = format!("set_{name}");
        for mrid in def.method_start..def.method_end {
            let Some(method) = self
                .module
                .tables
                .method_defs
                .get((mrid as usize).wrapping_sub(1))
            else {
                continue;
            };
            if !method.is_static {
                continue;
            }
            let raw = self.module.strings.get(method.name);
            let plain = raw.strip_suffix("@this").unwrap_or(raw);
            let plain = plain.strip_suffix("@static").unwrap_or(plain);
            if plain == name || plain == getter || plain == setter {
                return true;
            }
        }
        false
    }

    /// `Class.name` read through a class value: a static field, a static
    /// getter or a static method found on the class or its bases, else the
    /// class's own name for `name`.
    pub(crate) fn class_static_read(
        &mut self,
        rid: u32,
        name: &str,
    ) -> Result<Option<Value>, RunError> {
        let mut current = Some(rid);
        for _ in 0..256 {
            let Some(r) = current else { break };
            let Some(def) = self
                .module
                .tables
                .type_defs
                .get((r as usize).wrapping_sub(1))
                .cloned()
            else {
                break;
            };
            // The declaring class holds the FieldDef, but the *value* may sit
            // on any class between here and there: a write through a subclass
            // creates an own property on it. So the search walks the chain,
            // keyed by the declaring field.
            if let Some((_, frid)) = self.static_field_origin(r, name) {
                if let Some(kind) = self
                    .module
                    .tables
                    .field_defs
                    .get((frid as usize).wrapping_sub(1))
                    .map(|field| field.kind)
                {
                    let value = zun_gc::with_current_heap(|heap| {
                        if let Some(any) = heap.static_get(r, frid) {
                            let value = Value::unbox_any(any, heap).unwrap_or(Value::Void);
                            Some(crate::value::retag_static_numeric(value, kind))
                        } else {
                            heap.static_get_function(r, frid).map(Value::Function)
                        }
                    });
                    if let Some(value) = value {
                        return Ok(Some(value));
                    }
                }
            }
            let getter = format!("get_{name}");
            let mut found_getter = None;
            let mut found_method = None;
            for mrid in def.method_start..def.method_end {
                let Some(method) = self
                    .module
                    .tables
                    .method_defs
                    .get((mrid as usize).wrapping_sub(1))
                else {
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
                let method_name = method_name.strip_suffix("@static").unwrap_or(method_name);
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
                return Ok(Some(Value::Function(zun_core::FunctionHandle::new(
                    mrid, None,
                ))));
            }
            current = self
                .class_static_proto_override(r)
                .or_else(|| {
                    (def.base.table() == Some(zun_metadata::TableKind::TypeDef)
                        && !def.base.is_null())
                    .then(|| def.base.rid())
                });
        }
        if name == "length" {
            return Ok(Some(Value::Float(self.class_ctor_length(rid), zun_il::NumKind::F64)));
        }
        if name == "name" {
            let def = self
                .module
                .tables
                .type_defs
                .get((rid as usize).wrapping_sub(1));
            if let Some(def) = def {
                return Ok(Some(Value::Str(self.module.strings.get(def.name).into())));
            }
        }
        // A class whose base is only a run-time value inherits that value's
        // statics (`class S extends Symbol {}` reads `S.iterator`). The
        // metadata records no base for such a class, so the module initializer
        // registered the name.
        if let Some(base) = self.dynamic_base_of(rid) {
            if let Some(found) = self.static_of_runtime_base(&base, name)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    /// The source name of the run-time value class `rid` extends, if any.
    fn dynamic_base_of(&self, rid: u32) -> Option<String> {
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            let any = heap.dynamic_get(global, &format!("$dynamicBase#{rid}"))?;
            heap.try_string(any.as_string()?).cloned()
        })
        .map(|text| text.to_string())
    }

    /// Read the static `name` off the run-time value named `base`.
    fn static_of_runtime_base(
        &mut self,
        base: &str,
        name: &str,
    ) -> Result<Option<Value>, RunError> {
        let value = zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            heap.dynamic_get(global, base)
        });
        let Some(token) = value.and_then(|any| any.as_class_descriptor()) else {
            return Ok(None);
        };
        if let Some(rid) = self.user_class_rid(token) {
            return self.class_static_read(rid, name);
        }
        // `Symbol`'s well-known symbols are data statics, not methods.
        if base == "Symbol" {
            if let Some(index) = Self::well_known_symbol_index(name) {
                let id = zun_core::SymbolId::from_raw(index);
                return Ok(Some(Value::Any(zun_core::Any::symbol(id))));
            }
        }
        Ok(self
            .builtin_static_kind(token, name)
            .map(|kind| Value::Function(zun_core::FunctionHandle::new_builtin_method(kind))))
    }

    /// The `length` of the class `rid` as a constructor: the specified value for
    /// a built-in (prelude) constructor, else its constructor's declared
    /// parameter count (excluding the receiver), 0 without an explicit one.
    pub(crate) fn class_ctor_length(&self, rid: u32) -> f64 {
        let declared = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
            .and_then(|def| crate::intrinsic_meta::intrinsic_ctor_length(self.module.strings.get(def.name)));
        if let Some(count) = declared {
            return f64::from(count);
        }
        self.class_constructor(rid)
            .and_then(|mrid| {
                let method = self.module.tables.method_defs.get((mrid as usize).wrapping_sub(1))?;
                let body = zun_module::read_body(&self.module.code, method.body_offset).ok()?;
                Some((body.param_count as usize).saturating_sub(1))
            })
            .unwrap_or(0) as f64
    }

    /// MethodDef rid of the instance `constructor` declared by `rid` or the
    /// nearest base class.
    pub(crate) fn class_constructor(&self, rid: u32) -> Option<u32> {
        let mut current = Some(rid);
        for _ in 0..256 {
            let r = current?;
            let def = self
                .module
                .tables
                .type_defs
                .get((r as usize).wrapping_sub(1))?;
            for mrid in def.method_start..def.method_end {
                let method = self
                    .module
                    .tables
                    .method_defs
                    .get((mrid as usize).wrapping_sub(1))?;
                if !method.is_static && self.module.strings.get(method.name) == "constructor" {
                    return Some(mrid);
                }
            }
            current = (def.base.table() == Some(zun_metadata::TableKind::TypeDef)
                && !def.base.is_null())
            .then(|| def.base.rid());
        }
        None
    }

    /// `Class.name = value` through a class value: store into the static field
    /// found on the class or its bases. Answers whether a field took the write.
    /// The `(declaring class rid, FieldDef rid)` of the static field `name`
    /// visible from class `rid`, searching base classes.
    ///
    /// Static storage is keyed by the *pair*, so a subclass can hold its own
    /// value for a base's field: that is what JS does when a write goes
    /// through the subclass (`Sub.count++` reads the base's and creates an own
    /// property on `Sub`).
    pub(crate) fn static_field_origin(&self, rid: u32, name: &str) -> Option<(u32, u32)> {
        if name.starts_with('#') {
            return None;
        }
        let mut current = Some(rid);
        for _ in 0..256 {
            let r = current?;
            let def = self.module.tables.type_defs.get((r as usize).wrapping_sub(1))?;
            for frid in def.field_start..def.field_end {
                let field = self
                    .module
                    .tables
                    .field_defs
                    .get((frid as usize).wrapping_sub(1))?;
                if field.is_static && self.module.strings.get(field.name) == name {
                    return Some((r, frid));
                }
            }
            current = (def.base.table() == Some(zun_metadata::TableKind::TypeDef)
                && !def.base.is_null())
            .then(|| def.base.rid());
        }
        None
    }

    pub(crate) fn class_static_write(&mut self, rid: u32, name: &str, value: &Value) -> bool {
        // The write lands on the *receiving* class, not the declaring one: in
        // JS `Sub.count = v` creates an own property on `Sub` and leaves the
        // base's alone. It used to walk the chain and overwrite the declaring
        // class's slot, so every subclass shared one counter.
        let Some((_, frid)) = self.static_field_origin(rid, name) else {
            return false;
        };
        zun_gc::with_current_heap_mut(|heap| {
            if let Value::Function(handle) = value {
                heap.static_set_function(rid, frid, *handle);
            } else {
                let any = value.box_any(heap).unwrap_or_else(zun_core::Any::null);
                heap.static_set(rid, frid, any);
            }
        });
        true
    }

    /// A store to `Class.name` through a class value: a declared static field
    /// takes the value in place and a declared `static set name` runs. A static
    /// getter without a setter refuses the write (silently, as sloppy-mode JS
    /// does). `Ok(false)` means the class declares no such static member, so
    /// the caller keeps the value as an own property in the class expando bag.
    pub(crate) fn class_static_store(
        &mut self,
        rid: u32,
        name: &str,
        value: &Value,
    ) -> Result<bool, RunError> {
        if self.class_static_write(rid, name, value) {
            return Ok(true);
        }
        if let Some((mrid, hidden)) = self.class_static_method(rid, &format!("set_{name}")) {
            let handle = zun_core::FunctionHandle::new(mrid, None);
            // The receiving class value rides as the hidden first parameter.
            let mut call_args = if hidden {
                vec![Value::Any(zun_core::Any::class_descriptor(
                    zun_natives::USER_CLASS_FLAG | rid,
                ))]
            } else {
                Vec::new()
            };
            call_args.push(unbox_arg(value).unwrap_or_else(|_| value.clone()));
            self.invoke_function_value(handle, call_args)?;
            return Ok(true);
        }
        Ok(self.class_static_method(rid, &format!("get_{name}")).is_some())
    }

    /// The native collection behind `object` for a borrowed `Map` / `Set`
    /// prototype method: `object` itself, or the `inner` collection of an
    /// instance of a `Map` / `Set` subclass.
    fn collection_subclass_inner(
        &mut self,
        object: zun_gc::Obj,
        is_map: bool,
    ) -> Result<zun_gc::Obj, RunError> {
        let typed = zun_gc::with_current_heap(|heap| {
            (!heap.is_dynamic_object(object) || heap.is_instance_bag(object))
                && heap.array_len(object).is_none()
                && heap.map_len(object).is_none()
                && heap.set_len(object).is_none()
                && object.type_rid() >= 1
        });
        let base = if is_map { "__BuiltinMap" } else { "__BuiltinSet" };
        if !typed
            || !self.type_rid_describes_object(object)
            || !self.class_chain_has_name(object.type_rid(), base)
        {
            return Ok(object);
        }
        Ok(match self.instance_member_get(object, "inner")? {
            Some(Value::Object(inner)) => inner,
            _ => object,
        })
    }

    /// The `Class.prototype` object a typed class instance inherits from.
    pub(crate) fn instance_class_prototype(&self, object: zun_gc::Obj) -> Option<zun_gc::Obj> {
        let typed = zun_gc::with_current_heap(|heap| {
            (!heap.is_dynamic_object(object) || heap.is_instance_bag(object))
                && heap.array_len(object).is_none()
        });
        let rid = object.type_rid();
        if !typed
            || rid < 1
            || (rid as usize) > self.module.tables.type_defs.len()
            || !self.type_rid_describes_object(object)
        {
            return None;
        }
        // A relinked instance (`Object.setPrototypeOf(inst, p)`, or
        // `Reflect.construct(.., newTarget)`) records its prototype in its
        // expando bag; that link wins over the class prototype.
        let relinked = zun_gc::with_current_heap(|heap| {
            if !heap.is_instance_bag(object) {
                return None;
            }
            heap.dynamic_prototype(object)
                .and_then(|any| any.as_object())
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
        });
        Some(relinked.unwrap_or_else(|| self.class_prototype_object(rid)))
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
        if zun_gc::with_current_heap(|heap| {
            !heap.is_dynamic_object(obj) && heap.array_len(obj).is_none()
        }) && self.instance_class_prototype(obj).is_some()
        {
            zun_gc::with_current_heap_mut(|heap| heap.ensure_instance_bag(obj));
        } else if zun_gc::with_current_heap(|heap| {
            heap.map_len(obj).is_some()
                || heap.set_len(obj).is_some()
                // A Promise too: it is not a dynamic object and had no table,
                // so `Object.defineProperty(p, Symbol.toStringTag, ..)` was
                // refused outright.
                || heap.promise_state(obj).is_some()
        }) {
            // A Map / Set / Promise takes expando properties in an instance bag.
            zun_gc::with_current_heap_mut(|heap| heap.ensure_instance_bag(obj));
        }
        let name = match key {
            PropKey::Sym(symbol) => {
                // Symbol-keyed data properties carry only an enumerable flag.
                let value = desc.value.unwrap_or_else(zun_core::Any::void);
                let enumerable = desc.enumerable.unwrap_or_else(|| {
                    zun_gc::with_current_heap(|heap| {
                        heap.dynamic_is_symbol_enumerable(obj, *symbol)
                    })
                });
                // An array (or regexp) carries symbol-keyed expandos in a dynamic entry.
                zun_gc::with_current_heap_mut(|heap| {
                    if heap.array_len(obj).is_some() || heap.regex_source(obj).is_some() {
                        heap.ensure_dynamic_entry(obj);
                    }
                });
                let existing =
                    zun_gc::with_current_heap(|heap| heap.dynamic_has_symbol_key(obj, *symbol));
                if existing {
                    // ValidateAndApplyPropertyDescriptor: a non-configurable key
                    // refuses a change of its configurable / enumerable
                    // attributes, and (once non-writable) of its value.
                    let (configurable, current_enumerable, writable, current_fn, accessor) =
                        zun_gc::with_current_heap(|heap| {
                            (
                                heap.dynamic_is_symbol_configurable(obj, *symbol),
                                heap.dynamic_is_symbol_enumerable(obj, *symbol),
                                heap.dynamic_is_symbol_writable(obj, *symbol),
                                heap.dynamic_get_symbol_function(obj, *symbol),
                                heap.dynamic_symbol_getter(obj, *symbol).is_some(),
                            )
                        });
                    if !configurable {
                        let value_changes = desc.value_fn.is_some_and(|f| Some(f) != current_fn)
                            || desc.value.is_some_and(|v| {
                                current_fn.is_some()
                                    || zun_gc::with_current_heap(|heap| {
                                        heap.dynamic_get_symbol(obj, *symbol) != Some(v)
                                    })
                            });
                        if desc.configurable == Some(true)
                            || desc.enumerable.is_some_and(|e| e != current_enumerable)
                            || (desc.is_accessor() != accessor
                                && (desc.is_accessor() || desc.is_data()))
                            || (!writable
                                && !accessor
                                && (desc.writable == Some(true) || value_changes))
                        {
                            return false;
                        }
                    }
                }
                let flag_configurable = desc.configurable.or((!existing).then_some(false));
                if desc.is_accessor() {
                    return zun_gc::with_current_heap_mut(|heap| {
                        // A half-specified redefinition (`{ set }` over an existing
                        // accessor) keeps the half it does not mention.
                        let prior_get = if existing { heap.dynamic_symbol_getter(obj, *symbol) } else { None };
                        let prior_set = if existing { heap.dynamic_symbol_setter(obj, *symbol) } else { None };
                        let ok = heap.dynamic_define_symbol(
                            obj,
                            *symbol,
                            zun_core::Any::void(),
                            enumerable,
                        );
                        if let Some(getter) = desc.get {
                            heap.dynamic_set_symbol_getter(obj, *symbol, getter);
                        }
                        if let Some(setter) = desc.set {
                            heap.dynamic_set_symbol_setter(obj, *symbol, setter);
                        }
                        if !desc.has_get {
                            if let Some(getter) = prior_get {
                                heap.dynamic_set_symbol_getter(obj, *symbol, getter);
                            }
                        }
                        if !desc.has_set {
                            if let Some(setter) = prior_set {
                                heap.dynamic_set_symbol_setter(obj, *symbol, setter);
                            }
                        }
                        heap.dynamic_set_symbol_attributes(obj, *symbol, None, flag_configurable);
                        ok
                    });
                }
                // A descriptor naming no value on an existing key only updates
                // its attributes (the value, function slot or getter stays).
                let attributes_only =
                    existing && desc.value.is_none() && desc.value_fn.is_none();
                let flag_writable = desc.writable.or((!existing).then_some(false));
                return zun_gc::with_current_heap_mut(|heap| {
                    let ok = if attributes_only {
                        true
                    } else {
                        heap.dynamic_define_symbol(obj, *symbol, value, enumerable)
                    };
                    if ok && attributes_only {
                        // Re-apply the enumerable flag without touching the value.
                        heap.dynamic_set_symbol_enumerable(obj, *symbol, enumerable);
                    }
                    if let (true, Some(function)) = (ok, desc.value_fn) {
                        heap.dynamic_set_symbol_function(obj, *symbol, function);
                    }
                    if ok {
                        heap.dynamic_set_symbol_attributes(
                            obj,
                            *symbol,
                            flag_writable,
                            flag_configurable,
                        );
                    }
                    ok
                });
            }
            PropKey::Str(name) => name.as_str(),
        };
        if zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()) {
            return self.define_array_property(obj, name, &desc);
        }
        // A canonical numeric key of a typed array addresses an element, never
        // a property of the view's expando table: redefining an element is not
        // modelled, so the definition is refused whether or not the view has
        // gained a table.
        if crate::typed_arrays::element_of(obj).is_some()
            && crate::typed_dyn::typed_numeric_key(&Value::Str(name.into())).is_some()
        {
            return false;
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
                    heap.dynamic_define_accessor(
                        obj,
                        name.to_owned(),
                        desc.get,
                        desc.set,
                        enumerable,
                        configurable,
                    )
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
            zun_core::PropertyDescriptor::Data {
                enumerable,
                configurable,
                ..
            } => (*enumerable, *configurable),
            zun_core::PropertyDescriptor::Accessor {
                enumerable,
                configurable,
                ..
            } => (*enumerable, *configurable),
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
                zun_core::PropertyDescriptor::Data {
                    value, writable, ..
                } => {
                    if desc.is_accessor() {
                        return false;
                    }
                    if !writable {
                        if desc.writable == Some(true) {
                            return false;
                        }
                        if let Some(new_value) = desc.value {
                            let same = zun_gc::with_current_heap(|heap| {
                                match (
                                    Value::unbox_any(new_value, heap),
                                    Value::unbox_any(*value, heap),
                                ) {
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
            zun_core::PropertyDescriptor::Data {
                value, writable, ..
            } => {
                if desc.is_accessor() {
                    heap.dynamic_force_define_accessor(
                        obj,
                        name.to_owned(),
                        desc.get,
                        desc.set,
                        enumerable,
                        configurable,
                    )
                } else {
                    let ok = heap.dynamic_force_define_data(
                        obj,
                        name.to_owned(),
                        desc.value.unwrap_or(value),
                        desc.writable.unwrap_or(writable),
                        enumerable,
                        configurable,
                    );
                    // Redefining a method: the new function value replaces the
                    // old one in the function side table.
                    if let (true, Some(function)) = (ok, desc.value_fn) {
                        heap.dynamic_set_function(obj, name.to_owned(), function);
                    }
                    ok
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
            (
                heap.array_len(array).unwrap_or(0),
                heap.array_integrity(array),
            )
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
                let wanted = value
                    .as_int()
                    .map(|n| n as f64)
                    .or_else(|| value.as_float());
                if let Some(wanted) = wanted {
                    let wanted = wanted as usize;
                    if wanted != len {
                        let op = if wanted < len {
                            ArrayOp::Remove
                        } else {
                            ArrayOp::Write
                        };
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
        let Some(key) = args.get(1).and_then(prop_key) else {
            return Ok(Value::Bool(false));
        };
        let value_arg = args.get(2).ok_or(RunError::BadToken)?;
        let first = args.first().ok_or(RunError::BadToken)?;
        // A class value: a declared static takes the write in place; any other
        // key is an own property kept in the class's expando bag. A function
        // value keeps its own properties in its bag too, and its `name` and
        // `length` are read-only.
        let class_token = match first {
            Value::Any(any) => any.as_class_descriptor(),
            _ => None,
        };
        let target = if let Some(token) = class_token {
            if let (Some(rid), PropKey::Str(name)) = (self.user_class_rid(token), &key) {
                if self.class_static_store(rid, name, value_arg)? {
                    return Ok(Value::Bool(true));
                }
            }
            // A constant of a built-in constructor is read-only.
            if matches!(&key, PropKey::Str(name) if self.builtin_constant(token, name).is_some())
            {
                return Ok(Value::Bool(false));
            }
            Self::class_expando_bag(token)
        } else if let Ok(Value::Function(handle)) = unbox_arg(first) {
            // `name` and `length` are read-only, and so are the constants of a
            // coercion constructor (`Number.MAX_VALUE`).
            if matches!(&key, PropKey::Str(name) if name == "name"
                || name == "length"
                || self.coerce_ctor_constant(handle, name).is_some())
            {
                return Ok(Value::Bool(false));
            }
            crate::function_expando_bag(handle)
        } else {
            let Some(target) = object(first) else {
                return Ok(Value::Bool(false));
            };
            // The indices and `length` of a `String` wrapper are non-writable.
            if matches!(&key, PropKey::Str(name) if self.string_wrapper_owns(target, name)) {
                return Ok(Value::Bool(false));
            }
            target
        };
        // A typed class instance: a declared field or `set` accessor takes the
        // write; any other key is an expando, which needs the instance's
        // property table. A Map, Set, RegExp ... gains a table the same way, so
        // the store lands instead of reporting success and vanishing.
        let (typed_instance, has_table, is_array, is_typed_array) =
            zun_gc::with_current_heap(|heap| {
                let array = heap.array_len(target).is_some();
                (
                    (!heap.is_dynamic_object(target) || heap.is_instance_bag(target))
                        && !array
                        && target.type_rid() >= 1,
                    heap.is_dynamic_object(target),
                    array,
                    heap.typed_array_element(target).is_some(),
                )
            });
        if typed_instance {
            if let PropKey::Str(name) = &key {
                // A frozen instance refuses a write to a declared field.
                if self.instance_declared_member(target, name) == Some(true)
                    && zun_gc::with_current_heap(|heap| heap.dynamic_is_frozen(target))
                {
                    return Ok(Value::Bool(false));
                }
                if self.instance_member_set(target, name, value_arg)
                    || self.instance_member_setter_call(target, name, value_arg)?
                {
                    return Ok(Value::Bool(true));
                }
                if let Some(setter) = self.proto_chain_accessor_setter(target, name) {
                    let Some(setter) = setter else {
                        return Ok(Value::Bool(false));
                    };
                    let arg = unbox_arg(value_arg).unwrap_or_else(|_| value_arg.clone());
                    self.invoke_function_value(
                        setter,
                        self.with_this_arg(setter, target, vec![arg]),
                    )?;
                    return Ok(Value::Bool(true));
                }
            }
        }
        if is_typed_array {
            // A typed array's `length` is an accessor without a setter. A
            // canonical numeric key addresses an element: in range it is
            // written, out of range it is ignored, and either way the set
            // reports success and never creates an expando.
            if matches!(&key, PropKey::Str(name) if name == "length") {
                return Ok(Value::Bool(false));
            }
            let key_value = unbox_arg(&args[1]).unwrap_or_else(|_| args[1].clone());
            if self.typed_view_index_set(target, &key_value, value_arg)? {
                return Ok(Value::Bool(true));
            }
        }
        if !has_table && !is_array {
            zun_gc::with_current_heap_mut(|heap| {
                if heap.proxy_target(target).is_some() {
                    return;
                }
                if typed_instance {
                    heap.ensure_instance_bag(target);
                } else {
                    heap.ensure_dynamic_entry(target);
                }
            });
        }
        // Arrays: elements and `length`.
        let array_info = zun_gc::with_current_heap(|heap| {
            heap.array_len(target)
                .map(|len| (len, heap.array_integrity(target)))
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
                let op = if wanted < len {
                    ArrayOp::Remove
                } else {
                    ArrayOp::Write
                };
                if array_refusal(level, op).is_some() {
                    return Ok(Value::Bool(false));
                }
                zun_gc::with_current_heap_mut(|heap| {
                    heap.array_resize(target, wanted);
                    // An `arguments` object's `length` is a writable own
                    // property: the write takes effect where an index write
                    // does not.
                    heap.array_set_arguments_length(target, wanted);
                });
                return Ok(Value::Bool(true));
            }
            let Ok(index) = name.parse::<usize>() else {
                return Ok(Value::Bool(false));
            };
            let op = if index < len {
                ArrayOp::Write
            } else {
                ArrayOp::Add
            };
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
                    self.invoke_function_value(
                        setter,
                        self.with_this_arg(setter, target, vec![arg]),
                    )?;
                    return Ok(Value::Bool(true));
                }
                // An inherited read-only data property refuses the write. Only plain
                // dynamic ancestors are inspected, so no proxy trap runs for a plain set.
                let blocked = zun_gc::with_current_heap(|heap| {
                    if heap.dynamic_has(target, &name) {
                        return false;
                    }
                    let mut current = target;
                    for _ in 0..4096 {
                        let Some(next) = heap
                            .dynamic_prototype(current)
                            .and_then(|link| link.as_object())
                        else {
                            return false;
                        };
                        current = unsafe { zun_gc::Obj::from_addr(next as *mut u64) };
                        if heap.proxy_target(current).is_some() || !heap.is_dynamic_object(current) {
                            return false;
                        }
                        match heap.dynamic_descriptor(current, &name) {
                            Some(zun_core::PropertyDescriptor::Data { writable, .. }) => {
                                return !writable;
                            }
                            Some(_) => return false,
                            None => {}
                        }
                    }
                    false
                });
                if blocked {
                    return Ok(Value::Bool(false));
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

    /// The descriptor of `key` on `start` or the nearest object of its prototype
    /// chain that owns it, or `None` when no object on the chain does.
    fn find_property_desc(
        &mut self,
        start: zun_gc::Obj,
        key_value: &Value,
    ) -> Result<Option<Desc>, RunError> {
        let mut current = start;
        for _ in 0..4096 {
            let descriptor = self.call_managed_collection(
                "objectGetOwnPropertyDescriptor",
                &[Value::Object(current), key_value.clone()],
            )?;
            if let Some(desc_obj) = object(&descriptor) {
                return Ok(Some(Desc::parse(desc_obj)));
            }
            let parent =
                self.call_managed_collection("objectGetPrototypeOf", &[Value::Object(current)])?;
            match object(&parent) {
                Some(next) => current = next,
                None => break,
            }
        }
        Ok(None)
    }

    /// `Reflect.get(target, key, receiver)`: ECMAScript `OrdinaryGet`. The
    /// property is found on the target's chain; an accessor's getter runs with
    /// the receiver as `this`.
    pub(crate) fn reflect_get_receiver(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let Some(target) = args.first().and_then(object) else {
            return Ok(Value::Void);
        };
        let receiver_arg = args.get(2).ok_or(RunError::BadToken)?;
        let receiver = unbox_arg(receiver_arg).unwrap_or_else(|_| receiver_arg.clone());
        let is_proxy = zun_gc::with_current_heap(|heap| heap.proxy_target(target).is_some());
        let key = args.get(1).and_then(prop_key);
        if is_proxy || object(&receiver) == Some(target) {
            return match key {
                Some(PropKey::Str(name)) => self.dynamic_member_get(target, &name),
                _ => Ok(Value::Void),
            };
        }
        let Some(desc) = self.find_property_desc(target, args.get(1).ok_or(RunError::BadToken)?)?
        else {
            return Ok(Value::Void);
        };
        if desc.is_accessor() {
            let Some(getter) = desc.get else {
                return Ok(Value::Void);
            };
            return self.invoke_function_value(getter, self.with_this_value(getter, receiver, vec![]));
        }
        if let Some(handle) = desc.value_fn {
            return Ok(Value::Function(handle));
        }
        Ok(match desc.value {
            Some(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                .unwrap_or(Value::Any(any)),
            None => Value::Void,
        })
    }

    /// `Reflect.set(target, key, value, receiver)`: ECMAScript `OrdinarySet`. The
    /// property is looked up on the target and its prototype chain, but a data
    /// property is defined on the *receiver* and an accessor's setter runs with
    /// the receiver as `this`. Without a receiver (or with the target as the
    /// receiver) this is plain [`Self::reflect_set`].
    pub(crate) fn reflect_set_receiver(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let Some(receiver_arg) = args.get(3) else {
            return self.reflect_set(args);
        };
        let Some(target) = args.first().and_then(object) else {
            return Ok(Value::Bool(false));
        };
        let receiver = unbox_arg(receiver_arg).unwrap_or_else(|_| receiver_arg.clone());
        if object(&receiver) == Some(target) {
            return self.reflect_set(&args[..3]);
        }
        let Some(_) = args.get(1).and_then(prop_key) else {
            return Ok(Value::Bool(false));
        };
        let key_value = args[1].clone();
        let value_arg = args.get(2).ok_or(RunError::BadToken)?;
        let found = self.find_property_desc(target, &key_value)?;
        if let Some(desc) = found.as_ref().filter(|desc| desc.is_accessor()) {
            let Some(setter) = desc.set else {
                return Ok(Value::Bool(false));
            };
            let any = zun_gc::with_current_heap_mut(|heap| value_arg.box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            let arg = zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                .unwrap_or(Value::Any(any));
            self.invoke_function_value(setter, self.with_this_value(setter, receiver, vec![arg]))?;
            return Ok(Value::Bool(true));
        }
        if found
            .as_ref()
            .is_some_and(|desc| desc.writable == Some(false))
        {
            return Ok(Value::Bool(false));
        }
        // A data property: it lands on the receiver, which must be an object.
        let Some(receiver) = object(&receiver) else {
            return Ok(Value::Bool(false));
        };
        let existing = self.call_managed_collection(
            "objectGetOwnPropertyDescriptor",
            &[Value::Object(receiver), key_value.clone()],
        )?;
        let existing = object(&existing).map(Desc::parse);
        if let Some(existing) = &existing {
            if existing.is_accessor() || existing.writable == Some(false) {
                return Ok(Value::Bool(false));
            }
        }
        let desc_obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
        zun_gc::root!(desc_obj = desc_obj);
        zun_gc::with_current_heap_mut(|heap| {
            match value_arg {
                Value::Function(handle) => {
                    heap.dynamic_set_function(desc_obj.get(), "value", *handle);
                }
                other => {
                    let any = other.box_any(heap).unwrap_or_else(zun_core::Any::null);
                    heap.dynamic_set(desc_obj.get(), "value".to_owned(), any);
                }
            }
            if existing.is_none() {
                for flag in ["writable", "enumerable", "configurable"] {
                    heap.dynamic_set(desc_obj.get(), flag.to_owned(), zun_core::Any::bool(true));
                }
            }
        });
        // Through `Reflect.defineProperty`, so a proxy receiver reaches its
        // `defineProperty` trap and a typed receiver its expando bag.
        self.call_managed_collection(
            "reflectDefineProperty",
            &[Value::Object(receiver), key_value, Value::Object(desc_obj.get())],
        )
    }

    /// ToPropertyDescriptor: read a descriptor object's fields in spec order
    /// (`enumerable`, `configurable`, `value`, `writable`, `get`, `set`) through
    /// its getters and prototype chain, validate accessor callability and the
    /// accessor/data conflict, and return a fresh plain descriptor object.
    pub(crate) fn normalize_descriptor(
        &mut self,
        desc: zun_gc::Obj,
    ) -> Result<zun_gc::Obj, RunError> {
        let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
        // `forInHas` and `dynamic_member_get` run user getters and proxy traps, which can
        // collect: both the descriptor and the output object are read through guards.
        zun_gc::root!(desc = desc);
        zun_gc::root!(out = out);
        let (mut has_accessor, mut has_data) = (false, false);
        for field in [
            "enumerable",
            "configurable",
            "value",
            "writable",
            "get",
            "set",
        ] {
            let present = self.call_managed_collection(
                "forInHas",
                &[Value::Object(desc.get()), Value::Str(field.into())],
            )?;
            if !matches!(present, Value::Bool(true)) {
                continue;
            }
            let value = self.dynamic_member_get(desc.get(), field)?;
            match field {
                "enumerable" | "configurable" | "writable" => {
                    if field == "writable" {
                        has_data = true;
                    }
                    let flag = zun_gc::with_current_heap(|heap| {
                        super::collections::is_truthy(&value, heap)
                    });
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(out.get(), field.to_owned(), zun_core::Any::bool(flag));
                    });
                }
                "value" if matches!(value, Value::Function(_)) => {
                    has_data = true;
                    if let Value::Function(handle) = value {
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set_function(out.get(), "value", handle);
                        });
                    }
                }
                "value" => {
                    has_data = true;
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or_else(zun_core::Any::void);
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(out.get(), "value".to_owned(), boxed);
                    });
                }
                _ => {
                    has_accessor = true;
                    match value {
                        Value::Function(handle) => zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set_function(out.get(), field, handle);
                        }),
                        Value::Void => zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set(out.get(), field.to_owned(), zun_core::Any::void());
                        }),
                        Value::Any(any) if any.tag() == zun_core::AnyTag::Void => {
                            zun_gc::with_current_heap_mut(|heap| {
                                heap.dynamic_set(
                                    out.get(),
                                    field.to_owned(),
                                    zun_core::Any::void(),
                                );
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
        Ok(out.get())
    }

    /// `Object.defineProperty(obj, key, desc)` / `Reflect.defineProperty`.
    /// `strict` throws a `TypeError` on refusal and returns the object;
    /// otherwise a boolean.
    pub(crate) fn define_property_native(
        &mut self,
        args: &[Value],
        strict: bool,
    ) -> Result<Value, RunError> {
        let Some(target) = args.first().and_then(object) else {
            return Err(RunError::Native(
                "TypeError: Object.defineProperty called on non-object".into(),
            ));
        };
        let key = match args.get(1).and_then(prop_key) {
            Some(key) => key,
            // An object key is coerced with ToPropertyKey (ToPrimitive, string hint).
            None => match args.get(1).and_then(object) {
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
        // A function object keeps its own properties in its expando bag.
        let function_bag = match args.first().map(unbox_arg) {
            Some(Ok(Value::Function(handle))) => Some(super::function_expando_bag(handle)),
            _ => None,
        };
        // A class value keeps the properties scripts stored on it in its
        // expando bag; a declared static of a user class is dropped from its
        // slot. `prototype` is not configurable.
        if let Some(Value::Any(descriptor)) = args.first() {
            if let Some(token) = descriptor.as_class_descriptor() {
                let Some(PropKey::Str(name)) = args.get(1).and_then(prop_key) else {
                    return Ok(Value::Bool(true));
                };
                if let Some(bag) = Self::class_expando_peek(token) {
                    if zun_gc::with_current_heap(|heap| heap.dynamic_has_own(bag, &name)) {
                        if !zun_gc::with_current_heap(|heap| heap.dynamic_can_delete(bag, &name)) {
                            return Err(RunError::Native(format!(
                                "TypeError: Cannot delete property '{name}' of class"
                            )));
                        }
                        zun_gc::with_current_heap_mut(|heap| heap.dynamic_delete(bag, &name));
                        return Ok(Value::Bool(true));
                    }
                }
                if name != "prototype" {
                    if let Some(rid) = self.user_class_rid(token) {
                        self.class_delete_own(rid, &name);
                    }
                }
                return Ok(Value::Bool(true));
            }
        }
        let Some(target) = function_bag.or_else(|| args.first().and_then(object)) else {
            return Ok(Value::Bool(true));
        };
        let key = args.get(1).and_then(prop_key).or_else(|| {
            // An `any` key that is not a string, number or symbol (a boolean,
            // an object, ...) names the property its string form spells.
            let raw = args.get(1)?;
            self.dyn_to_string(raw).ok().map(PropKey::Str)
        });
        if let Some(PropKey::Sym(symbol)) = key {
            let removed =
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_delete_symbol(target, symbol));
            if !removed {
                return Err(RunError::Native(
                    "TypeError: Cannot delete property of #<Object>".into(),
                ));
            }
            return Ok(Value::Bool(true));
        }
        let Some(PropKey::Str(name)) = key else {
            return Ok(Value::Bool(true));
        };
        if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(target)) {
            // A named property of an array (`arr.join = fn`) lives in its
            // expando table and is removed like any dynamic property; only an
            // index or `length` is handled here.
            let is_index = name.parse::<usize>().is_ok();
            if !is_index && name != "length" {
                // A sealed or frozen array keeps its named properties too.
                let allowed = zun_gc::with_current_heap(|heap| {
                    heap.array_integrity(target) < 2
                        || !heap.dynamic_has(target, &name) && heap.dynamic_get_function(target, &name).is_none()
                }) && zun_gc::with_current_heap(|heap| heap.dynamic_can_delete(target, &name));
                if !allowed {
                    return Err(RunError::Native(format!(
                        "TypeError: Cannot delete property '{name}' of [object Array]"
                    )));
                }
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_delete(target, &name));
                return Ok(Value::Bool(true));
            }
            // `delete arr[i]` leaves a hole at an in-range index.
            if let Ok(index) = name.parse::<usize>() {
                if index < len {
                    if self
                        .guard_array(target, crate::integrity::ArrayOp::Remove)
                        .is_err()
                    {
                        return Err(RunError::Native(
                            "TypeError: Cannot delete property of [object Array]".into(),
                        ));
                    }
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.array_set(target, index, zun_core::Any::hole())
                    });
                }
            }
            return Ok(Value::Bool(true));
        }
        if name == "name" && self.error_delete_assigned_name(target) {
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
            // The level is recorded on the view's property table, so the
            // integrity queries and named stores see it; element writes are
            // unaffected, as the specification requires.
            zun_gc::with_current_heap_mut(|heap| {
                heap.ensure_dynamic_entry(obj);
                match level {
                    1 => {
                        heap.dynamic_prevent_extensions(obj);
                    }
                    2 => {
                        heap.dynamic_seal(obj);
                    }
                    _ => {
                        heap.dynamic_freeze(obj);
                    }
                }
            });
            return Ok(true);
        }
        Ok(false)
    }
}

impl Interp<'_> {
    /// The real `Name.prototype` object of the global builtin class `name`
    /// (`Map`, `Set`, `Array`, ...), when the realm exposes it as a descriptor.
    pub(crate) fn builtin_prototype_by_name(&self, name: &str) -> Option<zun_gc::Obj> {
        let descriptor = zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            heap.dynamic_get(global, name)
        })?;
        let token = descriptor.as_class_descriptor()?;
        Some(self.builtin_prototype_object(token))
    }
}

/// One own property of a user class constructor (see
/// [`Interp::class_own_props`]).
pub(crate) enum ClassOwn {
    /// An initialised static field.
    Field(u32, u32),
    /// A static method.
    Method(zun_core::FunctionHandle),
    /// A static accessor pair.
    Accessor(Option<zun_core::FunctionHandle>, Option<zun_core::FunctionHandle>),
}

impl Interp<'_> {
    /// Own static members of the user class `rid` in declaration order:
    /// initialised fields, methods and accessors (hidden `@this` helpers and
    /// the constructor are skipped).
    pub(crate) fn class_own_props(&self, rid: u32) -> Vec<(String, ClassOwn)> {
        let mut out: Vec<(String, ClassOwn)> = Vec::new();
        let Some(def) = self
            .module
            .tables
            .type_defs
            .get((rid as usize).wrapping_sub(1))
        else {
            return out;
        };
        for frid in def.field_start..def.field_end {
            let Some(field) = self
                .module
                .tables
                .field_defs
                .get((frid as usize).wrapping_sub(1))
            else {
                continue;
            };
            if !field.is_static {
                continue;
            }
            // A `#private` static is not a property: no reflection surface
            // may list it.
            if field.is_private {
                continue;
            }
            // A `static { ... }` block is lowered to a synthetic field named by its
            // source text; it is not a property.
            if self.module.strings.get(field.name).starts_with("static {") {
                continue;
            }
            let set = zun_gc::with_current_heap(|heap| {
                heap.static_get(rid, frid).is_some() || heap.static_get_function(rid, frid).is_some()
            });
            if set {
                out.push((
                    self.module.strings.get(field.name).to_owned(),
                    ClassOwn::Field(rid, frid),
                ));
            }
        }
        for mrid in def.method_start..def.method_end {
            let Some(method) = self
                .module
                .tables
                .method_defs
                .get((mrid as usize).wrapping_sub(1))
            else {
                continue;
            };
            if !method.is_static || method.body_offset == u32::MAX {
                continue;
            }
            let name = self.module.strings.get(method.name);
            let handle = zun_core::FunctionHandle::new(mrid, None);
            let (prop, is_get) = if let Some(p) = name.strip_prefix("get_") {
                (p, Some(true))
            } else if let Some(p) = name.strip_prefix("set_") {
                (p, Some(false))
            } else {
                (name, None)
            };
            let ident = !prop.is_empty()
                && prop
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
            if !ident {
                continue;
            }
            match is_get {
                None => out.push((name.to_owned(), ClassOwn::Method(handle))),
                Some(get) => {
                    if let Some((_, ClassOwn::Accessor(g, s))) =
                        out.iter_mut().find(|(n, _)| n == prop)
                    {
                        if get {
                            *g = Some(handle);
                        } else {
                            *s = Some(handle);
                        }
                    } else if get {
                        out.push((prop.to_owned(), ClassOwn::Accessor(Some(handle), None)));
                    } else {
                        out.push((prop.to_owned(), ClassOwn::Accessor(None, Some(handle))));
                    }
                }
            }
        }
        out
    }

    /// `Object.getOwnPropertyNames(Class)` for a user class descriptor.
    pub(crate) fn class_own_names(&self, rid: u32) -> Vec<String> {
        if let Some(members) = self.namespace_class_members(rid) {
            return members.iter().map(|m| (*m).to_owned()).collect();
        }
        let mut names = vec!["length".to_owned(), "name".to_owned(), "prototype".to_owned()];
        names.extend(self.class_own_props(rid).into_iter().map(|(n, _)| n));
        names
    }

    /// `Object.getOwnPropertyDescriptor(Class, key)` for a user class.
    pub(crate) fn class_own_descriptor(&self, rid: u32, key: &str) -> Option<zun_gc::Obj> {
        if self.namespace_class_members(rid).is_some_and(|members| !members.contains(&key)) {
            return None;
        }
        if key == "prototype" {
            let proto = self.class_prototype_object(rid);
            return Some(zun_gc::with_current_heap_mut(|heap| {
                let out = heap.alloc_dynamic();
                heap.dynamic_set(
                    out,
                    "value".to_owned(),
                    zun_core::Any::object(proto.addr() as usize),
                );
                for (n, f) in [("writable", false), ("enumerable", false), ("configurable", false)]
                {
                    heap.dynamic_set(out, n.to_owned(), zun_core::Any::bool(f));
                }
                out
            }));
        }
        let Some((_, own)) = self.class_own_props(rid).into_iter().find(|(n, _)| n == key) else {
            // `length` and `name` are the constructor's own read-only,
            // configurable data properties unless a static member replaces them.
            let value = match key {
                "length" => Value::Float(self.class_ctor_length(rid), zun_il::NumKind::F64),
                "name" => {
                    let def = self.module.tables.type_defs.get((rid as usize).wrapping_sub(1))?;
                    Value::Str(self.module.strings.get(def.name).into())
                }
                _ => return None,
            };
            return Some(Self::data_descriptor_object(value, false, true));
        };
        Some(zun_gc::with_current_heap_mut(|heap| {
            let out = heap.alloc_dynamic();
            let flag = |heap: &mut zun_gc::Heap, n: &str, f: bool| {
                heap.dynamic_set(out, n.to_owned(), zun_core::Any::bool(f));
            };
            match own {
                ClassOwn::Field(r, frid) => {
                    if let Some(any) = heap.static_get(r, frid) {
                        heap.dynamic_set(out, "value".to_owned(), any);
                    } else if let Some(handle) = heap.static_get_function(r, frid) {
                        heap.dynamic_set_function(out, "value", handle);
                    }
                    flag(heap, "writable", true);
                    flag(heap, "enumerable", true);
                    flag(heap, "configurable", true);
                }
                ClassOwn::Method(handle) => {
                    heap.dynamic_set_function(out, "value", handle);
                    flag(heap, "writable", true);
                    flag(heap, "enumerable", false);
                    flag(heap, "configurable", true);
                }
                ClassOwn::Accessor(get, set) => {
                    match get {
                        Some(h) => {
                            heap.dynamic_set_function(out, "get", h);
                        }
                        None => {
                            heap.dynamic_set(out, "get".to_owned(), zun_core::Any::void());
                        }
                    }
                    match set {
                        Some(h) => {
                            heap.dynamic_set_function(out, "set", h);
                        }
                        None => {
                            heap.dynamic_set(out, "set".to_owned(), zun_core::Any::void());
                        }
                    }
                    flag(heap, "enumerable", false);
                    flag(heap, "configurable", true);
                }
            }
            out
        }))
    }
}

impl Interp<'_> {
    /// The class a script re-parented the static side of `rid` to with
    /// `Object.setPrototypeOf(Class, Other)`, when one was recorded.
    pub(crate) fn class_static_proto_override(&self, rid: u32) -> Option<u32> {
        let key = format!("$classBag#{}", zun_natives::USER_CLASS_FLAG | rid);
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            let address = heap.dynamic_get(global, &key)?.as_object()?;
            let bag = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
            heap.dynamic_prototype(bag)?.as_class_descriptor()
        })
        .and_then(|token| self.user_class_rid(token))
    }
}
