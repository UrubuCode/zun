//! Own `length` / `name` / `prototype` of the built-in constructors, answered
//! from the realm global's class descriptors.
//!
//! A built-in constructor is a class descriptor whose token is not a user
//! class. Its name is the realm global key bound to that descriptor; its
//! `length` is the parameter count the ECMAScript specification gives it.

use super::value::Value;
use super::Interp;

/// `length` of the built-in constructors, as specified by ECMAScript (and the
/// Node `Buffer` surface). This mirrors the specification: the numbers are
/// data, not logic, and live only here. Typed-array constructors are all 3 and
/// are answered by [`intrinsic_ctor_length`] from the element table.
const INTRINSIC_CTOR_LENGTHS: &[(&str, u32)] = &[
    ("Object", 1),
    ("Array", 1),
    ("String", 1),
    ("Number", 1),
    ("Boolean", 1),
    ("Symbol", 0),
    ("BigInt", 1),
    ("Map", 0),
    ("Set", 0),
    ("WeakMap", 0),
    ("WeakSet", 0),
    ("WeakRef", 1),
    ("FinalizationRegistry", 1),
    ("Promise", 1),
    ("RegExp", 2),
    ("Date", 7),
    ("ArrayBuffer", 1),
    ("SharedArrayBuffer", 1),
    ("DataView", 1),
    ("Buffer", 3),
    ("Error", 1),
    ("EvalError", 1),
    ("RangeError", 1),
    ("ReferenceError", 1),
    ("SyntaxError", 1),
    ("TypeError", 1),
    ("URIError", 1),
    ("AggregateError", 2),
    ("Proxy", 2),
    ("Function", 1),
    // Web-platform constructors: the specified (WebIDL) length is the count of
    // required arguments, not the declared parameter count of the prelude class.
    ("URL", 1),
    ("URLSearchParams", 0),
    ("TextEncoder", 0),
    ("TextDecoder", 0),
    ("Blob", 0),
    ("File", 2),
    ("Headers", 0),
    ("Request", 1),
    ("Response", 0),
    ("FormData", 0),
    ("AbortController", 0),
    ("AbortSignal", 0),
    ("DOMException", 0),
    ("Event", 1),
    ("CustomEvent", 1),
    ("EventTarget", 0),
    ("ReadableStream", 0),
    ("WritableStream", 0),
    ("TransformStream", 0),
    ("parseInt", 2),
    ("parseFloat", 1),
    ("isNaN", 1),
    ("isFinite", 1),
    ("encodeURIComponent", 1),
    ("decodeURIComponent", 1),
    ("encodeURI", 1),
    ("decodeURI", 1),
    ("btoa", 1),
    ("atob", 1),
    ("escape", 1),
    ("unescape", 1),
];

/// The specified `length` of the built-in constructor called `name`.
pub(crate) fn intrinsic_ctor_length(name: &str) -> Option<u32> {
    if let Some(&(_, length)) = INTRINSIC_CTOR_LENGTHS.iter().find(|(n, _)| *n == name) {
        return Some(length);
    }
    zun_core::TypedElement::ALL
        .iter()
        .any(|kind| kind.js_name() == name)
        .then_some(3)
}

/// Names of the built-in `Error` constructors: their prototypes own `name` and
/// `message`.
const ERROR_FAMILY: &[&str] = &[
    "Error",
    "EvalError",
    "RangeError",
    "ReferenceError",
    "SyntaxError",
    "TypeError",
    "URIError",
    "AggregateError",
];

pub(crate) fn is_error_family(name: &str) -> bool {
    ERROR_FAMILY.contains(&name)
}

/// The realm global key bound to the class descriptor `token`, found on an
/// already-borrowed heap. A realm builtin is defined read-only, so a read-only
/// key wins over a writable one: a program's own `const C = Error` is stored on
/// the same global under the same descriptor and must not rename it.
pub(crate) fn builtin_class_name_in(heap: &mut zun_gc::Heap, token: u32) -> Option<String> {
    let global = heap.global_object();
    let keys: Vec<String> = heap
        .dynamic_keys(global)?
        .into_iter()
        .filter(|key| {
            heap.dynamic_get(global, key).and_then(|any| any.as_class_descriptor()) == Some(token)
        })
        .collect();
    keys.iter()
        .find(|key| !heap.dynamic_is_writable(global, key))
        .or_else(|| keys.first())
        .cloned()
}

/// Every realm global key bound to the class descriptor `token`. Several
/// function-like globals share the `Object` descriptor, so a lookup that needs
/// a specific one (statics) must consider all of them.
fn builtin_class_names_in(heap: &mut zun_gc::Heap, token: u32) -> Vec<String> {
    let global = heap.global_object();
    let Some(keys) = heap.dynamic_keys(global) else {
        return Vec::new();
    };
    keys.into_iter()
        .filter(|key| {
            heap.dynamic_get(global, key).and_then(|any| any.as_class_descriptor()) == Some(token)
        })
        .collect()
}

/// Built-in constructors the specification gives no `prototype` property
/// (`Proxy` is a constructor only for `new`): reading it yields `undefined`, it
/// is not an own key, and `instanceof` against it throws.
const PROTOTYPELESS_CTORS: &[&str] = &["Proxy"];

impl Interp<'_> {
    /// Whether the built-in constructor `token` owns a `prototype` property.
    pub(crate) fn builtin_ctor_has_prototype(&self, token: u32) -> bool {
        if token & zun_natives::USER_CLASS_FLAG != 0 {
            return true;
        }
        zun_gc::with_current_heap_mut(|heap| builtin_class_name_in(heap, token))
            .map_or(true, |name| !PROTOTYPELESS_CTORS.contains(&name.as_str()))
    }
}

impl Interp<'_> {
    /// The own value of a built-in constructor's `length` / `name`.
    pub(crate) fn builtin_ctor_own_value(&self, token: u32, key: &str) -> Option<Value> {
        if token & zun_natives::USER_CLASS_FLAG != 0 {
            return None;
        }
        let name = zun_gc::with_current_heap_mut(|heap| builtin_class_name_in(heap, token))?;
        match key {
            "name" => Some(Value::Str(name.into())),
            "length" => intrinsic_ctor_length(&name)
                .map(|n| Value::Float(f64::from(n), zun_il::NumKind::F64)),
            _ => None,
        }
    }

    /// The kind of the static method function value `key` on the built-in
    /// constructor `token` (`Object.keys`), if it has one.
    pub(crate) fn builtin_static_kind(&self, token: u32, key: &str) -> Option<u32> {
        if token & zun_natives::USER_CLASS_FLAG != 0 {
            return None;
        }
        zun_gc::with_current_heap_mut(|heap| builtin_class_names_in(heap, token))
            .iter()
            .find_map(|name| static_method_kind(name, key))
    }

    /// The constant `key` the built-in constructor `token` owns, if it has
    /// one: a number (`Number.EPSILON`) or a well-known symbol
    /// (`Symbol.iterator`). All of them are non-writable, non-enumerable and
    /// non-configurable.
    pub(crate) fn builtin_constant(&self, token: u32, key: &str) -> Option<Value> {
        if token & zun_natives::USER_CLASS_FLAG != 0 {
            return None;
        }
        zun_gc::with_current_heap_mut(|heap| builtin_class_names_in(heap, token)).iter().find_map(
            |name| {
                if let Some(number) = constant_value(name, key) {
                    return Some(Value::Float(number, zun_il::NumKind::F64));
                }
                symbol_constant(name, key)
                    .map(|symbol| Value::Any(zun_core::Any::symbol(symbol.id())))
            },
        )
    }

    /// The names of the constants the built-in constructor `token` owns.
    pub(crate) fn builtin_constant_names(&self, token: u32) -> Vec<&'static str> {
        if token & zun_natives::USER_CLASS_FLAG != 0 {
            return Vec::new();
        }
        zun_gc::with_current_heap_mut(|heap| builtin_class_names_in(heap, token))
            .iter()
            .flat_map(|name| {
                let mut names = constant_names(name);
                names.extend(symbol_constant_names(name));
                names
            })
            .collect()
    }

    /// The names of the static methods the built-in constructor `token` owns.
    pub(crate) fn builtin_static_names(&self, token: u32) -> Vec<&'static str> {
        if token & zun_natives::USER_CLASS_FLAG != 0 {
            return Vec::new();
        }
        zun_gc::with_current_heap_mut(|heap| builtin_class_names_in(heap, token))
            .iter()
            .map(|name| static_methods_of(name))
            .find(|methods| !methods.is_empty())
            .map(|methods| methods.into_iter().map(|(_, method)| method).collect())
            .unwrap_or_default()
    }

    /// `Object.getOwnPropertyDescriptor(BuiltinCtor, key)` for `prototype`,
    /// `length` and `name`.
    pub(crate) fn builtin_ctor_own_descriptor(&self, token: u32, key: &str) -> Option<zun_gc::Obj> {
        if self.user_class_rid(token).is_some_and(|rid| self.namespace_class_members(rid).is_some()) {
            return None;
        }
        let (value, writable, configurable) = match key {
            // A constructor's `prototype` is non-writable, non-configurable.
            "prototype" => {
                if !self.builtin_ctor_has_prototype(token) {
                    return None;
                }
                let proto = self.builtin_prototype_object(token);
                (Value::Any(zun_core::Any::object(proto.addr() as usize)), false, false)
            }
            // `length` and `name` are read-only but configurable.
            "length" | "name" => (self.builtin_ctor_own_value(token, key)?, false, true),
            // A constant is read-only, non-enumerable and non-configurable.
            _ if self.builtin_constant(token, key).is_some() => {
                (self.builtin_constant(token, key)?, false, false)
            }
            // A static method is writable, non-enumerable and configurable.
            _ => {
                let kind = self.builtin_static_kind(token, key)?;
                (Value::Function(zun_core::FunctionHandle::new_builtin_method(kind)), true, true)
            }
        };
        Some(Self::data_descriptor_object(value, writable, configurable))
    }

    /// The built-in constructor a `String` / `Number` / `Boolean` function
    /// value stands for: those three globals read as a value are the coercion
    /// functions (`xs.map(String)`), so the constructor surface is answered
    /// for the function handle.
    pub(crate) fn coerce_ctor_name(&self, handle: zun_core::FunctionHandle) -> Option<&'static str> {
        if handle.builtin_method().is_some()
            || handle.is_bound()
            || handle.generator_method().is_some()
        {
            return None;
        }
        let method = self
            .module
            .tables
            .method_defs
            .get((handle.method_rid() as usize).wrapping_sub(1))?;
        match self.module.strings.get(method.name) {
            "__coerceNumber" => Some("Number"),
            "__coerceString" => Some("String"),
            "__coerceBoolean" => Some("Boolean"),
            _ => None,
        }
    }

    /// The own constant `key` of the coercion function value `handle`
    /// (`Number.EPSILON`), if it stands for a constructor that owns one.
    pub(crate) fn coerce_ctor_constant(
        &self,
        handle: zun_core::FunctionHandle,
        key: &str,
    ) -> Option<f64> {
        constant_value(self.coerce_ctor_name(handle)?, key)
    }

    /// The own constant names of the coercion function value `handle`.
    pub(crate) fn coerce_ctor_constant_names(
        &self,
        handle: zun_core::FunctionHandle,
    ) -> Vec<&'static str> {
        self.coerce_ctor_name(handle).map(constant_names).unwrap_or_default()
    }

    /// The kind of the static method function value `key` of the coercion
    /// function value `handle` (`Number.isInteger`), if its constructor owns one.
    /// Keyed on the constructor the handle stands for, so `String` and `Boolean`
    /// never see `Number`'s statics.
    pub(crate) fn coerce_ctor_static_kind(
        &self,
        handle: zun_core::FunctionHandle,
        key: &str,
    ) -> Option<u32> {
        static_method_kind(self.coerce_ctor_name(handle)?, key)
    }

    /// The own static method names of the coercion function value `handle`.
    pub(crate) fn coerce_ctor_static_names(
        &self,
        handle: zun_core::FunctionHandle,
    ) -> Vec<&'static str> {
        self.coerce_ctor_name(handle)
            .map(|class| static_methods_of(class).into_iter().map(|(_, name)| name).collect())
            .unwrap_or_default()
    }

    /// The own static member `key` of the coercion function value `handle`: a
    /// constant (`Number.EPSILON`) or a static method function value.
    pub(crate) fn coerce_ctor_static_value(
        &self,
        handle: zun_core::FunctionHandle,
        key: &str,
    ) -> Option<Value> {
        if let Some(constant) = self.coerce_ctor_constant(handle, key) {
            return Some(Value::Float(constant, zun_il::NumKind::F64));
        }
        let kind = self.coerce_ctor_static_kind(handle, key)?;
        Some(Value::Function(zun_core::FunctionHandle::new_builtin_method(kind)))
    }

    /// The descriptor of the static method `key` of the coercion function value
    /// `handle`: writable, non-enumerable, configurable.
    pub(crate) fn coerce_ctor_static_descriptor(
        &self,
        handle: zun_core::FunctionHandle,
        key: &str,
    ) -> Option<zun_gc::Obj> {
        let kind = self.coerce_ctor_static_kind(handle, key)?;
        Some(Self::data_descriptor_object(
            Value::Function(zun_core::FunctionHandle::new_builtin_method(kind)),
            true,
            true,
        ))
    }

    /// `Object.getOwnPropertyDescriptor(Number, "prototype")` for a coercion
    /// function value: the realm constructor's own `prototype` descriptor.
    pub(crate) fn coerce_ctor_prototype_descriptor(
        &self,
        handle: zun_core::FunctionHandle,
    ) -> Option<zun_gc::Obj> {
        let class = self.coerce_ctor_name(handle)?;
        let token = zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            heap.dynamic_get(global, class).and_then(|any| any.as_class_descriptor())
        })?;
        self.builtin_ctor_own_descriptor(token, "prototype")
    }

    /// A read-only, non-enumerable, non-configurable constant descriptor.
    pub(crate) fn constant_descriptor_object(value: f64) -> zun_gc::Obj {
        Self::data_descriptor_object(Value::Float(value, zun_il::NumKind::F64), false, false)
    }

    pub(crate) fn data_descriptor_object(value: Value, writable: bool, configurable: bool) -> zun_gc::Obj {
        zun_gc::with_current_heap_mut(|heap| {
            let out = heap.alloc_dynamic();
            let any = value.box_any_cell(heap).unwrap_or_else(zun_core::Any::void);
            heap.dynamic_set(out, "value".to_owned(), any);
            heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(writable));
            heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(false));
            heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(configurable));
            out
        })
    }
}

/// Prelude classes that stand for a namespace object (`JSON`) rather than a
/// constructor, with the members the specification gives that object. The
/// class also holds internal helper statics and, being a class, would answer
/// `length` / `name` / `prototype`; neither is an own property of the
/// namespace, so own-key and descriptor queries are restricted to this list.
const NAMESPACE_CLASS_MEMBERS: &[(&str, &[&str])] =
    &[("JSON", &["isRawJSON", "parse", "rawJSON", "stringify"])];

impl Interp<'_> {
    /// The specified own members of the prelude class `rid` when it stands for
    /// a namespace object.
    pub(crate) fn namespace_class_members(&self, rid: u32) -> Option<&'static [&'static str]> {
        let def = self.module.tables.type_defs.get((rid as usize).wrapping_sub(1))?;
        let name = self.module.strings.get(def.name);
        NAMESPACE_CLASS_MEMBERS.iter().find(|(class, _)| *class == name).map(|(_, m)| *m)
    }
}

/// Install the own `name` and `message` data properties an `Error`-family
/// prototype carries (writable, non-enumerable, configurable).
pub(crate) fn define_error_prototype_fields(
    heap: &mut zun_gc::Heap,
    proto: zun_gc::Obj,
    class: &str,
) {
    let name = zun_core::Any::string(heap.intern_string(class));
    let message = zun_core::Any::string(heap.intern_string(""));
    heap.dynamic_define_data(proto, "name", name, true, false, true);
    heap.dynamic_define_data(proto, "message", message, true, false, true);
}

/// First kind of the generic built-in prototype method values. A kind past this
/// base indexes [`prototype_method_at`]; the call itself is routed by
/// `proto_methods.rs` through the same dynamic dispatch a member call on an
/// `any` receiver uses.
pub(crate) const BUILTIN_PROTO_BASE: u32 = 1000;

/// The prototype methods each built-in class exposes as function values, with
/// the `length` ECMAScript gives each. This mirrors the specification: the names
/// and counts are data, not logic, and live only here. They cannot be derived
/// from the `zun_natives` registry because a native's parameter list is the Zun
/// typed signature (receiver, defaulted trailing arguments), not the specified
/// `length`. `TypedArray` is the shared `%TypedArray%.prototype`.
const PROTOTYPE_METHODS: &[(&str, &[(&str, u32)])] = &[
    (
        "Array",
        &[
            ("at", 1), ("concat", 1), ("copyWithin", 2), ("fill", 1), ("find", 1),
            ("findIndex", 1), ("findLast", 1), ("findLastIndex", 1), ("lastIndexOf", 1),
            ("pop", 0), ("push", 1), ("reverse", 0), ("shift", 0), ("unshift", 1),
            ("slice", 2), ("sort", 1), ("splice", 2), ("includes", 1), ("indexOf", 1),
            ("join", 1), ("keys", 0), ("entries", 0), ("values", 0), ("forEach", 1),
            ("filter", 1), ("flat", 0), ("flatMap", 1), ("map", 1), ("every", 1),
            ("some", 1), ("reduce", 1), ("reduceRight", 1), ("toReversed", 0),
            ("toSorted", 1), ("toSpliced", 2), ("with", 2), ("toString", 0), ("toLocaleString", 0),
        ],
    ),
    (
        "String",
        &[
            ("at", 1), ("charAt", 1), ("charCodeAt", 1), ("codePointAt", 1), ("concat", 1),
            ("endsWith", 1), ("includes", 1), ("indexOf", 1), ("isWellFormed", 0),
            ("lastIndexOf", 1), ("localeCompare", 1), ("match", 1), ("matchAll", 1),
            ("normalize", 0), ("padEnd", 1), ("padStart", 1), ("repeat", 1), ("replace", 2),
            ("replaceAll", 2), ("search", 1), ("slice", 2), ("split", 2), ("substr", 2),
            ("substring", 2), ("startsWith", 1), ("toString", 0), ("toWellFormed", 0),
            ("trim", 0), ("trimStart", 0), ("trimEnd", 0), ("toLowerCase", 0),
            ("toUpperCase", 0), ("valueOf", 0),
            // Annex B legacy HTML methods; mirrors `zun_natives::STRING_HTML_METHODS`
            // (a test below keeps the two in step).
            ("anchor", 1), ("big", 0), ("blink", 0), ("bold", 0), ("fixed", 0),
            ("fontcolor", 1), ("fontsize", 1), ("italics", 0), ("link", 1), ("small", 0),
            ("strike", 0), ("sub", 0), ("sup", 0),
        ],
    ),
    (
        "Number",
        &[("toExponential", 1), ("toFixed", 1), ("toPrecision", 1), ("toString", 1), ("valueOf", 0), ("toLocaleString", 0)],
    ),
    ("BigInt", &[("toString", 0), ("valueOf", 0)]),
    ("Symbol", &[("toString", 0), ("valueOf", 0), ("[Symbol.toPrimitive]", 1)]),
    ("WeakRef", &[("deref", 0)]),
    ("WeakMap", &[("delete", 1), ("get", 1), ("set", 2), ("has", 1)]),
    ("WeakSet", &[("delete", 1), ("has", 1), ("add", 1)]),
    ("Promise", &[("then", 2), ("catch", 1), ("finally", 1)]),
    ("RegExp", &[("exec", 1), ("test", 1)]),
    ("Function", &[("apply", 2), ("bind", 1), ("call", 1), ("toString", 0)]),
    (
        "TypedArray",
        &[
            ("entries", 0), ("keys", 0), ("values", 0), ("at", 1), ("copyWithin", 2),
            ("every", 1), ("fill", 1), ("filter", 1), ("find", 1), ("findIndex", 1),
            ("findLast", 1), ("findLastIndex", 1), ("forEach", 1), ("includes", 1),
            ("indexOf", 1), ("join", 1), ("lastIndexOf", 1), ("map", 1), ("reverse", 0),
            ("reduce", 1), ("reduceRight", 1), ("set", 1), ("slice", 2), ("some", 1),
            ("sort", 1), ("subarray", 2), ("toReversed", 0), ("toSorted", 1), ("with", 2),
        ],
    ),
    ("Boolean", &[("toString", 0), ("valueOf", 0)]),
    // `Object.prototype`'s methods are not installed as own properties (the
    // prototype object answers them through its member read); the table only
    // gives the unbound method values their kind, name and length.
    (
        "Object",
        &[
            ("toString", 0), ("toLocaleString", 0), ("valueOf", 0), ("hasOwnProperty", 1),
            ("isPrototypeOf", 1), ("propertyIsEnumerable", 1), ("__defineGetter__", 2),
            ("__defineSetter__", 2), ("__lookupGetter__", 1), ("__lookupSetter__", 1),
        ],
    ),
];

/// The methods the prototype of the built-in class `class` carries as
/// `(kind, name)` pairs, in specification order.
pub(crate) fn prototype_methods_of(class: &str) -> Vec<(u32, &'static str)> {
    let mut kind = BUILTIN_PROTO_BASE;
    let mut found = Vec::new();
    for (name, methods) in PROTOTYPE_METHODS {
        for (method, _) in *methods {
            if *name == class {
                found.push((kind, *method));
            }
            kind += 1;
        }
    }
    found
}

/// `(class, method name, specified length)` of a prototype method value kind.
pub(crate) fn prototype_method_at(kind: u32) -> Option<(&'static str, &'static str, u32)> {
    let mut index = kind.checked_sub(BUILTIN_PROTO_BASE)? as usize;
    for (class, methods) in PROTOTYPE_METHODS {
        if index < methods.len() {
            let (name, length) = methods[index];
            return Some((class, name, length));
        }
        index -= methods.len();
    }
    None
}

/// Install the prototype method values of `class` on `proto`: writable,
/// non-enumerable, configurable, exactly like a built-in's own methods.
pub(crate) fn install_prototype_methods(heap: &mut zun_gc::Heap, proto: zun_gc::Obj, class: &str) {
    for (kind, name) in prototype_methods_of(class) {
        // A well-known-symbol-keyed method lives under that symbol, not under
        // its display name. `Symbol.prototype[Symbol.toPrimitive]` is the only
        // one so far; the name in the table is what `.name` must answer.
        if class == "Symbol" && name == "[Symbol.toPrimitive]" {
            let id = zun_core::WellKnownSymbol::ToPrimitive.id();
            heap.dynamic_set_symbol_method(
                proto,
                id,
                zun_core::FunctionHandle::new_builtin_method(kind),
            );
            // Non-writable, unlike an ordinary prototype method.
            heap.dynamic_set_symbol_enumerable(proto, id, false);
            heap.dynamic_set_symbol_attributes(proto, id, Some(false), Some(true));
            continue;
        }
        heap.dynamic_set_method(proto, name, zun_core::FunctionHandle::new_builtin_method(kind));
        // In the specification `Array.prototype[Symbol.iterator]` *is* the same
        // function object as `Array.prototype.values`, so it is installed under
        // both keys.
        if class == "Array" && name == "values" {
            heap.dynamic_set_symbol_method(
                proto,
                zun_core::WellKnownSymbol::Iterator.id(),
                zun_core::FunctionHandle::new_builtin_method(kind),
            );
        }
    }
    if class == "Array" {
        install_array_unscopables(heap, proto);
    }
}

/// The member names `Array.prototype[Symbol.unscopables]` marks, exactly as in
/// the specification: every `Array.prototype` method added after ES5, so a
/// `with` block over an array does not shadow an outer binding of the same
/// name.
const ARRAY_UNSCOPABLES: &[&str] = &[
    "at",
    "copyWithin",
    "entries",
    "fill",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "flat",
    "flatMap",
    "includes",
    "keys",
    "toReversed",
    "toSorted",
    "toSpliced",
    "values",
];

/// Install `Array.prototype[Symbol.unscopables]`: a null-prototype object whose
/// members are all `true`.
///
/// Zun has no `with` statement, so this changes no behaviour -- but the object
/// is observable (`Object.keys`, `getPrototypeOf`, the values), and reading it
/// answered `undefined`, which made every reflective probe of it throw.
fn install_array_unscopables(heap: &mut zun_gc::Heap, proto: zun_gc::Obj) {
    let list = heap.alloc_dynamic();
    // An *explicit* null prototype is `Some(Any::null())`; `None` means "not
    // set", which falls back to `Object.prototype`.
    heap.dynamic_set_prototype(list, Some(zun_core::Any::null()));
    for name in ARRAY_UNSCOPABLES {
        heap.dynamic_set(list, *name, zun_core::Any::bool(true));
    }
    let value = zun_core::Any::object(list.addr() as usize);
    let id = zun_core::WellKnownSymbol::Unscopables.id();
    heap.dynamic_set_symbol(proto, id, value);
    // Non-writable, non-enumerable, configurable, as the specification says.
    heap.dynamic_set_symbol_enumerable(proto, id, false);
    heap.dynamic_set_symbol_attributes(proto, id, Some(false), Some(true));
}

/// First kind of the built-in static method values (`Math.max`, `Object.keys`,
/// `Reflect.get`, ...). Kinds in `BUILTIN_PROTO_BASE..BUILTIN_STATIC_BASE` are
/// prototype methods, so the prototype table must stay below 1000 entries.
pub(crate) const BUILTIN_STATIC_BASE: u32 = 2000;

/// The own data constants of the `Math` namespace object (non-writable,
/// non-enumerable, non-configurable).
pub(crate) const MATH_CONSTANTS: &[(&str, f64)] = &[
    ("E", std::f64::consts::E),
    ("LN10", std::f64::consts::LN_10),
    ("LN2", std::f64::consts::LN_2),
    ("LOG10E", std::f64::consts::LOG10_E),
    ("LOG2E", std::f64::consts::LOG2_E),
    ("PI", std::f64::consts::PI),
    ("SQRT1_2", std::f64::consts::FRAC_1_SQRT_2),
    ("SQRT2", std::f64::consts::SQRT_2),
];

/// The own data constants of the `Number` constructor (non-writable,
/// non-enumerable, non-configurable), as `(name, value)`.
const NUMBER_CONSTANTS: &[(&str, f64)] = &[
    ("MAX_VALUE", f64::MAX),
    ("MIN_VALUE", 5e-324),
    ("NaN", f64::NAN),
    ("NEGATIVE_INFINITY", f64::NEG_INFINITY),
    ("POSITIVE_INFINITY", f64::INFINITY),
    ("MAX_SAFE_INTEGER", 9_007_199_254_740_991.0),
    ("MIN_SAFE_INTEGER", -9_007_199_254_740_991.0),
    ("EPSILON", f64::EPSILON),
];

/// The constants a built-in constructor or namespace owns, by global name.
fn constants_of(class: &str) -> &'static [(&'static str, f64)] {
    match class {
        "Number" => NUMBER_CONSTANTS,
        _ => &[],
    }
}

/// The value of the constant `key` of the built-in `class`.
fn constant_value(class: &str, key: &str) -> Option<f64> {
    constants_of(class).iter().find(|(n, _)| *n == key).map(|(_, v)| *v)
}

/// The well-known symbol the `Symbol` constructor owns under `key`
/// (`Symbol.iterator`).
fn symbol_constant(class: &str, key: &str) -> Option<zun_core::WellKnownSymbol> {
    if class != "Symbol" {
        return None;
    }
    zun_core::WellKnownSymbol::from_member(key)
}

/// The member names of the well-known symbols `class` owns: every one the
/// runtime seeds, named by its description without the `Symbol.` prefix.
fn symbol_constant_names(class: &str) -> Vec<&'static str> {
    if class != "Symbol" {
        return Vec::new();
    }
    zun_core::WellKnownSymbol::ALL
        .iter()
        .filter_map(|symbol| symbol.description().strip_prefix("Symbol."))
        .collect()
}

/// The names of the constants of the built-in `class`.
fn constant_names(class: &str) -> Vec<&'static str> {
    constants_of(class).iter().map(|(n, _)| *n).collect()
}

/// The static methods each built-in namespace or constructor exposes as
/// function values: `(name, specified length, native)`. As with
/// [`PROTOTYPE_METHODS`], names and counts mirror the specification. `native`
/// is the `zun_natives` intrinsic the method runs through the ordinary
/// managed-call path (one implementation), written `module:name` when it is
/// not in the `collections` group; an empty string marks a
/// member `static_methods.rs` adapts by name (`Math` and the few members whose
/// result shape differs from their native).
const STATIC_METHODS: &[(&str, &[(&str, u32, &str)])] = &[
    (
        "Math",
        &[
            ("abs", 1, ""), ("acos", 1, ""), ("acosh", 1, ""), ("asin", 1, ""), ("asinh", 1, ""),
            ("atan", 1, ""), ("atanh", 1, ""), ("atan2", 2, ""), ("ceil", 1, ""), ("cbrt", 1, ""),
            ("expm1", 1, ""), ("clz32", 1, ""), ("cos", 1, ""), ("cosh", 1, ""), ("exp", 1, ""),
            ("floor", 1, ""), ("fround", 1, ""), ("f16round", 1, ""), ("hypot", 2, ""),
            ("imul", 2, ""), ("log", 1, ""), ("log1p", 1, ""), ("log2", 1, ""), ("log10", 1, ""),
            ("max", 2, ""), ("min", 2, ""), ("pow", 2, ""), ("random", 0, ""), ("round", 1, ""),
            ("sign", 1, ""), ("sin", 1, ""), ("sinh", 1, ""), ("sqrt", 1, ""), ("tan", 1, ""),
            ("tanh", 1, ""), ("trunc", 1, ""),
        ],
    ),
    (
        "Object",
        &[
            ("assign", 2, ""), ("create", 2, ""), ("defineProperties", 2, "objectDefineProperties"),
            ("defineProperty", 3, "objectDefineProperty"), ("entries", 1, "objectEntries"),
            ("freeze", 1, "objectFreeze"), ("fromEntries", 1, "objectFromEntries"),
            ("getOwnPropertyDescriptor", 2, "objectGetOwnPropertyDescriptor"),
            ("getOwnPropertyDescriptors", 1, "objectGetOwnPropertyDescriptors"),
            ("getOwnPropertyNames", 1, "objectGetOwnPropertyNames"),
            ("getOwnPropertySymbols", 1, "objectGetOwnPropertySymbols"),
            ("getPrototypeOf", 1, "objectGetPrototypeOf"), ("groupBy", 2, "objectGroupBy"),
            ("hasOwn", 2, "objectHasOwn"), ("is", 2, "objectIs"),
            ("isExtensible", 1, "objectIsExtensible"), ("isFrozen", 1, "objectIsFrozen"),
            ("isSealed", 1, "objectIsSealed"), ("keys", 1, "objectKeys"),
            ("preventExtensions", 1, "objectPreventExtensions"), ("seal", 1, "objectSeal"),
            ("setPrototypeOf", 2, "objectSetPrototypeOf"), ("values", 1, "objectValues"),
        ],
    ),
    (
        "Reflect",
        &[
            ("defineProperty", 3, "reflectDefineProperty"), ("deleteProperty", 2, "reflectDelete"),
            ("apply", 3, ""), ("construct", 2, ""), ("get", 2, ""),
            ("getOwnPropertyDescriptor", 2, "objectGetOwnPropertyDescriptor"),
            ("getPrototypeOf", 1, "objectGetPrototypeOf"), ("has", 2, "reflectHas"),
            ("isExtensible", 1, "objectIsExtensible"), ("ownKeys", 1, "reflectOwnKeys"),
            ("preventExtensions", 1, ""), ("set", 3, "reflectSet"),
            ("setPrototypeOf", 2, "reflectSetPrototypeOf"),
        ],
    ),
    // `Number`'s statics. `Number` read as a value is the coercion function, so
    // they are reached through `coerce_ctor_static_kind`; `static_methods.rs`
    // runs them on the strict `number` natives (empty native name).
    (
        "Number",
        &[
            ("isFinite", 1, ""), ("isInteger", 1, ""), ("isNaN", 1, ""),
            ("isSafeInteger", 1, ""), ("parseFloat", 1, ""), ("parseInt", 2, ""),
        ],
    ),
    // `Array`'s statics. `of` and `from` are generic over `this`, so
    // `static_methods.rs` adapts them by name. `fromAsync` is absent on purpose:
    // it has no value form yet, and a listed function must also behave.
    ("Array", &[("from", 1, ""), ("isArray", 1, ""), ("of", 0, "")]),
    // `String`'s statics. Like `Number`, `String` read as a value is the
    // coercion function, and `static_methods.rs` adapts the three by name: the
    // natives are single-code helpers the typed call sites fold onto.
    ("String", &[("fromCharCode", 1, ""), ("fromCodePoint", 1, ""), ("raw", 1, "")]),
    // `BigInt.asIntN`/`asUintN` and `ArrayBuffer.isView` were callable but were
    // not *values*: `typeof BigInt.asIntN` read `undefined`, the descriptor was
    // absent, and `Object.getOwnPropertyNames(BigInt)` raised `BadToken`. The
    // same lists-vs-behaves shape that `6f201f27` fixed for prototype methods,
    // `4141e09e` for the `Number` statics and `3fa7a60b` for the `Array` ones.
    ("BigInt", &[("asIntN", 2, "bigIntAsIntN"), ("asUintN", 2, "bigIntAsUintN")]),
    // `Symbol.for` / `Symbol.keyFor`; the well-known symbols the constructor
    // also owns are constants (see [`symbol_constant`]).
    ("Symbol", &[("for", 1, "symbolFor"), ("keyFor", 1, "symbolKeyFor")]),
    // `isView`'s native lives in the `buffer` group rather than `collections`,
    // so its row names the module (see [`static_native`]).
    ("ArrayBuffer", &[("isView", 1, "buffer:isView")]),
    // `from` / `of` of every typed-array constructor. The specification keeps
    // them on the shared `%TypedArray%` intrinsic each constructor inherits
    // from; here each constructor owns its pair, so `hasOwnProperty` answers
    // `true` where Node says `false`. `static_methods.rs` adapts them by name.
    ("Int8Array", TYPED_ARRAY_STATICS),
    ("Uint8Array", TYPED_ARRAY_STATICS),
    ("Uint8ClampedArray", TYPED_ARRAY_STATICS),
    ("Int16Array", TYPED_ARRAY_STATICS),
    ("Uint16Array", TYPED_ARRAY_STATICS),
    ("Int32Array", TYPED_ARRAY_STATICS),
    ("Uint32Array", TYPED_ARRAY_STATICS),
    ("Float32Array", TYPED_ARRAY_STATICS),
    ("Float64Array", TYPED_ARRAY_STATICS),
    ("BigInt64Array", TYPED_ARRAY_STATICS),
    ("BigUint64Array", TYPED_ARRAY_STATICS),
];

/// The static methods every typed-array constructor carries.
const TYPED_ARRAY_STATICS: &[(&str, u32, &str)] = &[("from", 1, ""), ("of", 0, "")];

/// Split a [`STATIC_METHODS`] native entry into `(module, name)`. A bare name
/// lives in the `collections` group; `module:name` names another group.
pub(crate) fn static_native(native: &'static str) -> (&'static str, &'static str) {
    native.split_once(':').unwrap_or(("collections", native))
}

/// The static methods of the built-in `class` as `(kind, name)` pairs.
pub(crate) fn static_methods_of(class: &str) -> Vec<(u32, &'static str)> {
    let mut kind = BUILTIN_STATIC_BASE;
    let mut found = Vec::new();
    for (name, methods) in STATIC_METHODS {
        for (method, _, _) in *methods {
            if *name == class {
                found.push((kind, *method));
            }
            kind += 1;
        }
    }
    found
}

/// The kind of the static method `class.name`, if the class has one.
pub(crate) fn static_method_kind(class: &str, name: &str) -> Option<u32> {
    static_methods_of(class).into_iter().find(|(_, method)| *method == name).map(|(kind, _)| kind)
}

/// `(class, method name, specified length, native)` of a static method kind.
pub(crate) fn static_method_at(
    kind: u32,
) -> Option<(&'static str, &'static str, u32, &'static str)> {
    let mut index = kind.checked_sub(BUILTIN_STATIC_BASE)? as usize;
    for (class, methods) in STATIC_METHODS {
        if index < methods.len() {
            let (name, length, native) = methods[index];
            return Some((class, name, length, native));
        }
        index -= methods.len();
    }
    None
}

/// Install the namespace members of `class` (`Math`, `Reflect`) on `object`:
/// writable, non-enumerable, configurable methods and frozen constants, exactly
/// like a built-in's own properties.
pub(crate) fn install_namespace_members(heap: &mut zun_gc::Heap, object: zun_gc::Obj, class: &str) {
    if class == "Math" {
        for (name, value) in MATH_CONSTANTS {
            heap.dynamic_define_data(object, *name, zun_core::Any::float(*value), false, false, false);
        }
    }
    for (kind, name) in static_methods_of(class) {
        heap.dynamic_set_method(object, name, zun_core::FunctionHandle::new_builtin_method(kind));
    }
    // `Object.prototype.toString.call(Math)` is `[object Math]`.
    let tag = zun_core::WellKnownSymbol::ToStringTag.id();
    let text = zun_core::Any::string(heap.intern_string(class));
    heap.dynamic_define_symbol(object, tag, text, false);
    heap.dynamic_mark_symbol_readonly(object, tag);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prototype_method_kinds_round_trip() {
        let methods = prototype_methods_of("WeakRef");
        assert_eq!(methods.len(), 1);
        let (kind, name) = methods[0];
        assert_eq!(prototype_method_at(kind), Some(("WeakRef", name, 0)));
        assert_eq!(name, "deref");
    }

    #[test]
    fn static_method_kinds_round_trip_and_stay_clear_of_prototype_kinds() {
        let mut seen = std::collections::HashSet::new();
        for (class, methods) in STATIC_METHODS {
            for (kind, name) in static_methods_of(class) {
                assert!(kind >= BUILTIN_STATIC_BASE, "{class}.{name} overlaps prototype kinds");
                assert!(seen.insert(kind), "duplicate kind for {class}.{name}");
                let (found_class, found_name, _, _) = static_method_at(kind).expect("kind resolves");
                assert_eq!((found_class, found_name), (*class, name));
                assert_eq!(static_method_kind(class, name), Some(kind));
            }
            assert_eq!(static_methods_of(class).len(), methods.len());
        }
        assert_eq!(static_method_at(BUILTIN_STATIC_BASE + seen.len() as u32), None);
        // Every named native resolves in the module its row names.
        for (class, methods) in STATIC_METHODS {
            for (name, _, native) in *methods {
                if native.is_empty() {
                    continue;
                }
                let (module, function) = static_native(native);
                assert!(
                    zun_natives::resolve(module, function).is_some(),
                    "{class}.{name}: no native `{function}` in `{module}`"
                );
            }
        }
        let prototype_kinds: usize = PROTOTYPE_METHODS.iter().map(|(_, m)| m.len()).sum();
        assert!(BUILTIN_PROTO_BASE as usize + prototype_kinds <= BUILTIN_STATIC_BASE as usize);
    }

    #[test]
    fn every_kind_is_unique_and_resolves_to_its_class() {
        let mut seen = std::collections::HashSet::new();
        for (class, methods) in PROTOTYPE_METHODS {
            for (kind, name) in prototype_methods_of(class) {
                assert!(seen.insert(kind), "duplicate kind for {class}.{name}");
                let (found_class, found_name, _) = prototype_method_at(kind).expect("kind resolves");
                assert_eq!((found_class, found_name), (*class, name));
            }
            assert_eq!(prototype_methods_of(class).len(), methods.len());
        }
        assert_eq!(prototype_method_at(BUILTIN_PROTO_BASE + seen.len() as u32), None);
    }

    #[test]
    fn string_html_methods_are_reflected_with_their_specified_length() {
        let reflected = prototype_methods_of("String");
        for (name, _, attribute) in zun_natives::STRING_HTML_METHODS {
            let kind = reflected
                .iter()
                .find(|(_, method)| method == name)
                .map(|(kind, _)| *kind)
                .unwrap_or_else(|| panic!("String.prototype.{name} is not reflected"));
            let (_, _, length) = prototype_method_at(kind).expect("kind resolves");
            assert_eq!(length, u32::from(!attribute.is_empty()), "String.prototype.{name}.length");
        }
    }
}


/// The builtin-method kind behind `Array.prototype.values`, which is also the
/// function `Array.prototype[Symbol.iterator]` is installed as. An array
/// instance's `@@iterator` has to read back as that same function object.
pub(crate) fn array_values_kind() -> Option<u32> {
    // Read on every `arr[Symbol.iterator]`; the table walk allocates, and the
    // answer never changes.
    static KIND: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *KIND.get_or_init(|| {
        prototype_methods_of("Array")
            .into_iter()
            .find(|(_, name)| *name == "values")
            .map(|(kind, _)| kind)
    })
}

impl Interp<'_> {
    /// Before an equality test: a coercion function value that stands for a
    /// constructor (`Boolean` read as a value is the `__coerceBoolean` helper,
    /// so that `xs.map(Boolean)` has something to call) is replaced by that
    /// constructor's class value.
    ///
    /// The helper already *is* the constructor as far as `name`, `length` and
    /// its own constants go (`coerce_ctor_name`); identity was the one place
    /// that still saw two values, so `Boolean.prototype.constructor ===
    /// Boolean` was false while the same test through `globalThis` passed.
    /// Only an operand that is a bare function value is examined, so an
    /// ordinary comparison pays one tag test.
    pub(crate) fn unify_constructor_operands(&self, stack: &mut [Value]) {
        let len = stack.len();
        if len < 2 {
            return;
        }
        for index in [len - 2, len - 1] {
            let Value::Function(handle) = &stack[index] else {
                continue;
            };
            let Some(name) = self.coerce_ctor_name(*handle) else {
                continue;
            };
            let descriptor = zun_gc::with_current_heap_mut(|heap| {
                let global = heap.global_object();
                heap.dynamic_get(global, name)
            });
            if let Some(descriptor) = descriptor.filter(|any| any.as_class_descriptor().is_some()) {
                stack[index] = Value::Any(descriptor);
            }
        }
    }
}

/// Platform functions whose *value* is carried by a mangled prelude helper,
/// paired with the public name they report.
///
/// These are lowered at the call site, so the value a script reads is the
/// helper's. One table, consulted by everything that has to see through the
/// mangling: the reported `name`, and whether the value is a constructor (a
/// platform function is not, and owns no `prototype`).
///
/// Interim: a typed `display_name` on the method row replaces it.
const VALUE_HELPERS: &[(&str, &str)] = &[
    ("__zunSetTimeout", "setTimeout"),
    ("__zunSetInterval", "setInterval"),
    ("__zunClearTimeout", "clearTimeout"),
    ("__zunClearInterval", "clearInterval"),
    ("__zunQueueMicrotask", "queueMicrotask"),
    ("__zunSetImmediate", "setImmediate"),
    ("__zunBufferCompare", "compare"),
    ("__zunParseInt", "parseInt"),
    ("__zunParseFloat", "parseFloat"),
];

/// The public name of the platform function the mangled prelude helper
/// `mangled` carries, or `None` when it is an ordinary function.
pub(crate) fn value_helper_public_name(mangled: &str) -> Option<&'static str> {
    VALUE_HELPERS
        .iter()
        .find(|(helper, _)| *helper == mangled)
        .map(|(_, public)| *public)
}
