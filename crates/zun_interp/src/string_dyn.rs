//! Dynamic `String.prototype` methods with full ECMAScript argument coercion.
//!
//! The statically-typed string natives take exact `string`/integer arguments.
//! JS code passes optional, `undefined`, fractional, negative, or non-number
//! arguments; those calls are routed here, where every argument goes through
//! `ToIntegerOrInfinity` / `ToString` exactly as the specification describes.
//! Indices are UTF-16 code units.

use super::collections::{js_string_to_number, unbox_arg};
use super::value::Value;
use super::{Interp, RunError};

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn from_units(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// ToIntegerOrInfinity on an already-numeric value (`-0` folds to `+0`).
fn integer_or_infinity(number: f64) -> f64 {
    if number.is_nan() {
        0.0
    } else if number.is_infinite() {
        number
    } else {
        number.trunc() + 0.0
    }
}

/// Clamp an integer-or-infinity into `[0, len]`.
fn clamp_index(number: f64, len: usize) -> usize {
    if number <= 0.0 {
        0
    } else if number >= len as f64 {
        len
    } else {
        number as usize
    }
}

/// Resolve a relative index (negative counts from the end) into `[0, len]`.
fn relative_index(number: f64, len: usize) -> usize {
    if number < 0.0 {
        let resolved = len as f64 + number;
        if resolved <= 0.0 {
            0
        } else {
            resolved as usize
        }
    } else {
        clamp_index(number, len)
    }
}

fn find_from(haystack: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    (from..=last).find(|&start| haystack[start..start + needle.len()] == *needle)
}

fn number(value: f64) -> Value {
    Value::Float(value, zun_il::NumKind::F64)
}

impl<'a> Interp<'a> {
    /// ECMAScript ToNumber over any value (objects run OrdinaryToPrimitive).
    pub(super) fn dyn_to_number(&mut self, value: &Value) -> Result<f64, RunError> {
        Ok(match unbox_arg(value)? {
            Value::Int(n, _) => n as f64,
            Value::Float(n, _) => n,
            Value::Bool(flag) => f64::from(u8::from(flag)),
            Value::Null => 0.0,
            Value::Void => f64::NAN,
            Value::Str(text) => js_string_to_number(&text),
            Value::Object(obj) => match self.ordinary_to_primitive(obj, "number")? {
                Some(Value::Str(text)) => js_string_to_number(&text),
                Some(Value::Int(n, _)) => n as f64,
                Some(Value::Float(n, _)) => n,
                Some(Value::Bool(flag)) => f64::from(u8::from(flag)),
                Some(Value::Null) => 0.0,
                _ => f64::NAN,
            },
            _ => f64::NAN,
        })
    }

    /// ECMAScript ToString over any value (objects run OrdinaryToPrimitive).
    pub(super) fn dyn_to_string(&mut self, value: &Value) -> Result<String, RunError> {
        Ok(match unbox_arg(value)? {
            Value::Object(obj) => match self.ordinary_to_primitive(obj, "string")? {
                Some(primitive) => primitive.display(),
                None => "[object Object]".to_string(),
            },
            other => other.display(),
        })
    }

    fn dyn_integer(&mut self, value: &Value) -> Result<f64, RunError> {
        Ok(integer_or_infinity(self.dyn_to_number(value)?))
    }

    /// Whether an argument is `undefined` (an absent argument arrives as one).
    fn is_undefined(value: &Value) -> bool {
        matches!(unbox_arg(value), Ok(Value::Void))
    }

    /// Run `recv.name(...args)` with JS argument coercion. Unknown methods
    /// return `None` so the caller can fall back to its other dispatch.
    pub(super) fn string_method_dyn(
        &mut self,
        recv: &str,
        name: &str,
        args: &[Value],
    ) -> Result<Option<Value>, RunError> {
        let absent = Value::Void;
        let a0 = args.first().unwrap_or(&absent).clone();
        let a1 = args.get(1).unwrap_or(&absent).clone();
        let s = units(recv);
        let len = s.len();
        let text = |u: &[u16]| Value::Str(from_units(u).into());
        Ok(Some(match name {
            "substring" => {
                let start = clamp_index(self.dyn_integer(&a0)?, len);
                let end = if Self::is_undefined(&a1) {
                    len
                } else {
                    clamp_index(self.dyn_integer(&a1)?, len)
                };
                text(&s[start.min(end)..start.max(end)])
            }
            "slice" => {
                let start = relative_index(self.dyn_integer(&a0)?, len);
                let end = if Self::is_undefined(&a1) {
                    len
                } else {
                    relative_index(self.dyn_integer(&a1)?, len)
                };
                if start >= end {
                    Value::Str("".into())
                } else {
                    text(&s[start..end])
                }
            }
            "substr" => {
                let start = relative_index(self.dyn_integer(&a0)?, len);
                let count = if Self::is_undefined(&a1) {
                    len as f64
                } else {
                    self.dyn_integer(&a1)?
                };
                let end = (start as f64 + count.max(0.0)).min(len as f64) as usize;
                if start >= end {
                    Value::Str("".into())
                } else {
                    text(&s[start..end])
                }
            }
            "indexOf" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let from = clamp_index(self.dyn_integer(&a1)?, len);
                match find_from(&s, &needle, from) {
                    Some(index) => number(index as f64),
                    None => number(-1.0),
                }
            }
            "lastIndexOf" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let position = self.dyn_to_number(&a1)?;
                let from = if position.is_nan() {
                    len
                } else {
                    clamp_index(integer_or_infinity(position), len)
                };
                if needle.len() > len {
                    number(-1.0)
                } else {
                    let start = from.min(len - needle.len());
                    match (0..=start)
                        .rev()
                        .find(|&at| s[at..at + needle.len()] == *needle)
                    {
                        Some(index) => number(index as f64),
                        None => number(-1.0),
                    }
                }
            }
            "includes" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let from = clamp_index(self.dyn_integer(&a1)?, len);
                Value::Bool(find_from(&s, &needle, from).is_some())
            }
            "startsWith" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let from = clamp_index(self.dyn_integer(&a1)?, len);
                Value::Bool(from + needle.len() <= len && s[from..from + needle.len()] == *needle)
            }
            "endsWith" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let end = if Self::is_undefined(&a1) {
                    len
                } else {
                    clamp_index(self.dyn_integer(&a1)?, len)
                };
                Value::Bool(needle.len() <= end && s[end - needle.len()..end] == *needle)
            }
            "padStart" | "padEnd" => {
                let target = self.dyn_integer(&a0)?;
                let fill = if Self::is_undefined(&a1) {
                    units(" ")
                } else {
                    units(&self.dyn_to_string(&a1)?)
                };
                if target <= len as f64 || fill.is_empty() {
                    Value::Str(recv.into())
                } else {
                    let missing = target as usize - len;
                    let padding: Vec<u16> = fill.iter().cycle().take(missing).copied().collect();
                    let mut out = Vec::with_capacity(target as usize);
                    if name == "padStart" {
                        out.extend_from_slice(&padding);
                        out.extend_from_slice(&s);
                    } else {
                        out.extend_from_slice(&s);
                        out.extend_from_slice(&padding);
                    }
                    text(&out)
                }
            }
            "charAt" => {
                let index = self.dyn_integer(&a0)?;
                if index >= 0.0 && index < len as f64 {
                    text(&s[index as usize..index as usize + 1])
                } else {
                    Value::Str("".into())
                }
            }
            "charCodeAt" => {
                let index = self.dyn_integer(&a0)?;
                if index >= 0.0 && index < len as f64 {
                    number(f64::from(s[index as usize]))
                } else {
                    number(f64::NAN)
                }
            }
            "at" => {
                let index = self.dyn_integer(&a0)?;
                let resolved = if index < 0.0 { len as f64 + index } else { index };
                if resolved >= 0.0 && resolved < len as f64 {
                    text(&s[resolved as usize..resolved as usize + 1])
                } else {
                    Value::Void
                }
            }
            "toString" | "valueOf" => Value::Str(recv.into()),
            "toUpperCase" => Value::Str(recv.to_uppercase().into()),
            "toLowerCase" => Value::Str(recv.to_lowercase().into()),
            _ => return Ok(None),
        }))
    }
}
