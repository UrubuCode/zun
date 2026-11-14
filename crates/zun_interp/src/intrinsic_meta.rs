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
/// already-borrowed heap.
pub(crate) fn builtin_class_name_in(heap: &mut zun_gc::Heap, token: u32) -> Option<String> {
    let global = heap.global_object();
    heap.dynamic_keys(global)?.into_iter().find(|key| {
        heap.dynamic_get(global, key).and_then(|any| any.as_class_descriptor()) == Some(token)
    })
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
        let (value, writable, configurable) = match key {
            // A constructor's `prototype` is non-writable, non-configurable.
            "prototype" => {
                let proto = self.builtin_prototype_object(token);
                (Value::Any(zun_core::Any::object(proto.addr() as usize)), false, false)
            }
            // `length` and `name` are read-only but configurable.
            "length" | "name" => (self.builtin_ctor_own_value(token, key)?, false, true),
            // A static method is writable, non-enumerable and configurable.
            _ => {
                let kind = self.builtin_static_kind(token, key)?;
                (Value::Function(zun_core::FunctionHandle::new_builtin_method(kind)), true, true)
            }
        };
        Some(zun_gc::with_current_heap_mut(|heap| {
            let out = heap.alloc_dynamic();
            let any = value.box_any_cell(heap).unwrap_or_else(zun_core::Any::void);
            heap.dynamic_set(out, "value".to_owned(), any);
            heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(writable));
            heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(false));
            heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(configurable));
            out
        }))
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
            ("toSorted", 1), ("toSpliced", 2), ("with", 2), ("toString", 0),
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
        ],
    ),
    (
        "Number",
        &[("toExponential", 1), ("toFixed", 1), ("toPrecision", 1), ("toString", 1), ("valueOf", 0)],
    ),
    ("BigInt", &[("toString", 0), ("valueOf", 0)]),
    ("Symbol", &[("toString", 0), ("valueOf", 0)]),
    ("WeakRef", &[("deref", 0)]),
    ("WeakMap", &[("delete", 1), ("get", 1), ("set", 2), ("has", 1)]),
    ("WeakSet", &[("delete", 1), ("has", 1), ("add", 1)]),
    ("Promise", &[("then", 2), ("catch", 1), ("finally", 1)]),
    ("RegExp", &[("exec", 1), ("test", 1)]),
    ("Function", &[("apply", 2), ("bind", 1), ("call", 1)]),
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
        heap.dynamic_set_method(proto, name, zun_core::FunctionHandle::new_builtin_method(kind));
    }
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

/// The static methods each built-in namespace or constructor exposes as
/// function values: `(name, specified length, native)`. As with
/// [`PROTOTYPE_METHODS`], names and counts mirror the specification. `native`
/// is the `zun_natives` collection intrinsic the method runs through the
/// ordinary managed-call path (one implementation); an empty string marks a
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
];

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
}
