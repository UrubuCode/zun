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
            _ => return None,
        };
        Some(zun_gc::with_current_heap_mut(|heap| {
            let out = heap.alloc_dynamic();
            let any = value.box_any(heap).unwrap_or_else(zun_core::Any::void);
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
