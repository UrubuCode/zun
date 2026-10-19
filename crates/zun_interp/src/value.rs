//! The interpreter's runtime value model.
//!
//! Phase 1 is typed, so values are a small closed set: numbers (f64, as in JS),
//! booleans, strings, and a void/undefined marker for calls that return nothing.
//! This is the interpreter's own representation; the eventual object model and
//! GC heap (`zun_gc`) are separate concerns.

use std::cell::RefCell;
use std::rc::Rc;

/// A heap object: its class's TypeDef RID plus its field slots. `Rc<RefCell>`
/// gives reference semantics (two bindings share one object) and interior
/// mutability for field stores — a stand-in until the real GC heap lands.
#[derive(Debug)]
pub struct Object {
    /// 1-based TypeDef RID of the object's class.
    pub type_rid: u32,
    /// Field values, indexed by field slot.
    pub fields: RefCell<Vec<Value>>,
}

/// A runtime value.
#[derive(Debug, Clone)]
pub enum Value {
    /// An integer, stored in an i64 with its exact kind (width + signedness).
    Int(i64, zun_il::NumKind),
    /// A float, stored in an f64 with its kind (`F32` values are kept rounded
    /// to single precision).
    Float(f64, zun_il::NumKind),
    /// A boolean.
    Bool(bool),
    /// An immutable string. `Rc` keeps cloning a value cheap.
    Str(Rc<str>),
    /// A heap object reference (shared, mutable).
    Object(Rc<Object>),
    /// The absence of a value (a void call's result).
    Void,
}

impl Value {
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
        }
    }

    /// Interpret the value as an object reference, or panic (see
    /// [`Value::as_number`]).
    pub fn as_object(&self) -> &Rc<Object> {
        match self {
            Value::Object(o) => o,
            other => panic!("expected object, found {other:?}"),
        }
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
}
