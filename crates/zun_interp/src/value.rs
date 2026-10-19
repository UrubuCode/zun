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
    /// A number (IEEE-754 double) — the `number`/`float` type.
    Number(f64),
    /// A 32-bit signed integer — the `int` type.
    Int(i32),
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
            Value::Number(n) => *n,
            other => panic!("expected number, found {other:?}"),
        }
    }

    /// Interpret the value as an `int`, or panic (see [`Value::as_number`]).
    pub fn as_int(&self) -> i32 {
        match self {
            Value::Int(n) => *n,
            other => panic!("expected int, found {other:?}"),
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
            Value::Number(n) => format_number(*n),
            Value::Int(n) => n.to_string(),
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

    #[test]
    fn integers_print_without_decimal() {
        assert_eq!(Value::Number(42.0).display(), "42");
        assert_eq!(Value::Number(-7.0).display(), "-7");
    }

    #[test]
    fn fractions_keep_their_decimal() {
        assert_eq!(Value::Number(3.5).display(), "3.5");
    }

    #[test]
    fn other_values_display() {
        assert_eq!(Value::Bool(true).display(), "true");
        assert_eq!(Value::Str("hi".into()).display(), "hi");
        assert_eq!(Value::Void.display(), "undefined");
    }
}
