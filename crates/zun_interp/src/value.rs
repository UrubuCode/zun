//! The interpreter's runtime value model.
//!
//! Phase 1 is typed, so values are a small closed set: numbers (f64, as in JS),
//! booleans, strings, and a void/undefined marker for calls that return nothing.
//! This is the interpreter's own representation; the eventual object model and
//! GC heap (`zun_gc`) are separate concerns.

use std::rc::Rc;

/// A runtime value.
#[derive(Debug, Clone)]
pub enum Value {
    /// An explicitly dynamic tagged value.
    Any(zun_core::Any),
    /// An integer, stored in an i64 with its exact kind (width + signedness).
    Int(i64, zun_il::NumKind),
    /// A float, stored in an f64 with its kind (`F32` values are kept rounded
    /// to single precision).
    Float(f64, zun_il::NumKind),
    /// A boolean.
    Bool(bool),
    /// An immutable string. `Rc` keeps cloning a value cheap.
    Str(Rc<str>),
    /// A reference to an object in the runtime [`Heap`](zun_gc::Heap). Reference
    /// semantics (shared, mutable) come from the shared raw pointer.
    Object(zun_gc::Obj),
    /// The absence of a value (a void call's result).
    Void,
}

impl Value {
    /// Box a typed value into the runtime's explicit dynamic representation.
    pub fn box_any(&self, heap: &mut zun_gc::Heap) -> Option<zun_core::Any> {
        match self {
            Value::Int(value, _) => Some(zun_core::Any::int(*value)),
            Value::Float(value, _) => Some(zun_core::Any::float(*value)),
            Value::Bool(value) => Some(zun_core::Any::bool(*value)),
            Value::Object(object) => Some(zun_core::Any::object(object.addr() as usize)),
            Value::Str(value) => Some(zun_core::Any::string(heap.intern_string(value.clone()))),
            Value::Void => None,
            Value::Any(value) => Some(*value),
        }
    }

    /// Unbox a dynamic value, rejecting invalid tags or stale string handles.
    pub fn unbox_any(any: zun_core::Any, heap: &zun_gc::Heap) -> Option<Self> {
        match any.tag() {
            zun_core::AnyTag::Int => any.as_int().map(|value| Value::Int(value, zun_il::NumKind::I64)),
            zun_core::AnyTag::Float => any.as_float().map(|value| Value::Float(value, zun_il::NumKind::F64)),
            zun_core::AnyTag::Bool => any.as_bool().map(Value::Bool),
            zun_core::AnyTag::Object => any.as_object().filter(|address| *address != 0).map(|address| {
                Value::Object(unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
            }),
            zun_core::AnyTag::String => any
                .as_string()
                .and_then(|handle| heap.try_string(handle).cloned())
                .map(Value::Str),
        }
    }

    /// Interpret the value as a number, or panic if it is not one. The verifier
    /// and type checker guarantee operand types, so a mismatch here is a bug in
    /// code generation, not bad user input.
    pub fn as_number(&self) -> f64 {
        match self {
            Value::Float(n, _) => *n,
            other => panic!("expected float, found {other:?}"),
        }
    }

    /// The float kind of a float value (defaults to F64 for non-floats).
    pub fn float_kind(&self) -> zun_il::NumKind {
        match self {
            Value::Float(_, k) => *k,
            _ => zun_il::NumKind::F64,
        }
    }

    /// Interpret the value as an integer (i64 container), or panic.
    pub fn as_int(&self) -> i64 {
        match self {
            Value::Int(n, _) => *n,
            other => panic!("expected int, found {other:?}"),
        }
    }

    /// The integer kind of an int value (defaults to I32 for non-ints).
    pub fn int_kind(&self) -> zun_il::NumKind {
        match self {
            Value::Int(_, k) => *k,
            _ => zun_il::NumKind::I32,
        }
    }

    /// Interpret the value as a boolean, or panic (see [`Value::as_number`]).
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            other => panic!("expected bool, found {other:?}"),
        }
    }

    /// Interpret the value as a string, or panic (see [`Value::as_number`]).
    pub fn as_str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            other => panic!("expected string, found {other:?}"),
        }
    }

    /// Render the value the way `print` should show it.
    pub fn display(&self) -> String {
        match self {
            Value::Float(n, _) => format_number(*n),
            Value::Int(n, _) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Str(s) => s.to_string(),
            Value::Object(_) => "[object]".to_string(),
            Value::Void => "undefined".to_string(),
            Value::Any(_) => "[any]".to_string(),
        }
    }

    /// Interpret the value as an object reference, or panic (see
    /// [`Value::as_number`]).
    pub fn as_object(&self) -> zun_gc::Obj {
        match self {
            Value::Object(o) => *o,
            other => panic!("expected object, found {other:?}"),
        }
    }
}

/// Field value-type code for an object reference (beyond the `JitKind` codes).
pub const FIELD_OBJECT: u8 = 11;
/// Field value-type code for a string reference.
pub const FIELD_STRING: u8 = 12;

/// Decode a raw field slot into a [`Value`], given the field's value-type code
/// (from `FieldDef.kind`) and the heap (for string handles).
pub fn decode_field(kind: u8, slot: u64, heap: &zun_gc::Heap) -> Value {
    use zun_il::{JitKind, NumKind};
    match kind {
        FIELD_OBJECT => Value::Object(unsafe { zun_gc::Obj::from_addr(slot as *mut u64) }),
        FIELD_STRING => Value::Str(heap.string(slot as u32).clone()),
        _ => match JitKind::from_byte(kind) {
            JitKind::Num(NumKind::F64) => Value::Float(f64::from_bits(slot), NumKind::F64),
            JitKind::Num(NumKind::F32) => {
                Value::Float(f32::from_bits(slot as u32) as f64, NumKind::F32)
            }
            JitKind::Num(n) => Value::Int(slot as i64, n),
            JitKind::Bool => Value::Bool(slot != 0),
            JitKind::Object => Value::Object(unsafe { zun_gc::Obj::from_addr(slot as *mut u64) }),
            JitKind::Other => Value::Void,
        },
    }
}

/// Encode a [`Value`] into a raw field slot for the field's value-type code,
/// interning strings into the heap as needed.
pub fn encode_field(kind: u8, v: &Value, heap: &mut zun_gc::Heap) -> u64 {
    use zun_il::{JitKind, NumKind};
    match kind {
        FIELD_OBJECT => v.as_object().addr() as u64,
        FIELD_STRING => match v {
            Value::Str(s) => heap.intern_string(s.clone()) as u64,
            _ => 0,
        },
        _ => match JitKind::from_byte(kind) {
            JitKind::Num(NumKind::F64) => v.as_number().to_bits(),
            JitKind::Num(NumKind::F32) => (v.as_number() as f32).to_bits() as u64,
            JitKind::Num(_) => v.as_int() as u64,
            JitKind::Bool => matches!(v, Value::Bool(true)) as u64,
            JitKind::Object => v.as_object().addr() as u64,
            JitKind::Other => 0,
        },
    }
}

/// Wrap an i64 into the value range of `kind` (two's-complement for signed,
/// modular for unsigned), returning the canonical i64 representative.
pub fn wrap_int(v: i64, kind: zun_il::NumKind) -> i64 {
    let (bits, signed) = kind.int_parts().expect("int kind");
    if bits == 64 {
        return v;
    }
    let mask = (1i128 << bits) - 1;
    let m = (v as i128) & mask;
    if signed && (m & (1i128 << (bits - 1))) != 0 {
        (m - (1i128 << bits)) as i64
    } else {
        m as i64
    }
}

/// Round an f64 to `kind`'s precision (a no-op for F64, single-precision for F32).
pub fn round_float(v: f64, kind: zun_il::NumKind) -> f64 {
    match kind {
        zun_il::NumKind::F32 => v as f32 as f64,
        _ => v,
    }
}

/// Format a number using JS-like rules: integers print without a trailing `.0`.
fn format_number(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use zun_il::NumKind;

    #[test]
    fn integers_print_without_decimal() {
        assert_eq!(Value::Float(42.0, NumKind::F64).display(), "42");
        assert_eq!(Value::Int(-7, NumKind::I32).display(), "-7");
    }

    #[test]
    fn fractions_keep_their_decimal() {
        assert_eq!(Value::Float(3.5, NumKind::F64).display(), "3.5");
    }

    #[test]
    fn wrap_unsigned_and_signed() {
        assert_eq!(wrap_int(256, NumKind::U8), 0);
        assert_eq!(wrap_int(255, NumKind::U8), 255);
        assert_eq!(wrap_int(128, NumKind::I8), -128);
        assert_eq!(wrap_int(-1, NumKind::U8), 255);
    }

    #[test]
    fn other_values_display() {
        assert_eq!(Value::Bool(true).display(), "true");
        assert_eq!(Value::Str("hi".into()).display(), "hi");
        assert_eq!(Value::Void.display(), "undefined");
    }

    #[test]
    fn any_box_and_unbox_roundtrip() {
        let mut heap = zun_gc::Heap::new();
        for value in [Value::Int(7, NumKind::I32), Value::Float(2.5, NumKind::F64), Value::Bool(true)] {
            let any = value.box_any(&mut heap).expect("boxable");
            assert!(Value::unbox_any(any, &heap).is_some());
        }
        let value = Value::Str("hello".into());
        let any = value.box_any(&mut heap).expect("boxable");
        assert!(matches!(Value::unbox_any(any, &heap), Some(Value::Str(text)) if &*text == "hello"));
        assert!(Value::Void.box_any(&mut heap).is_none());
    }
}
