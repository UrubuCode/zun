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
