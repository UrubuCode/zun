//! Node's argument validation for the `Buffer` surface.
//!
//! The `nodeBuffer` natives that take a size, offset, byte count or value
//! receive those arguments **boxed as `any`** (an integer-typed parameter would
//! have already coerced `-1`, `NaN` and `1.01` into something plausible), so the
//! checks Node runs in `lib/buffer.js` / `lib/internal/buffer.js` can run here
//! on the raw JavaScript value: `validateNumber`, `boundsError`, `checkInt`, and
//! the `ERR_INVALID_ARG_TYPE` / `ERR_OUT_OF_RANGE` / `ERR_BUFFER_OUT_OF_BOUNDS`
//! errors they throw, with Node's `code`, `name` and message text.
//!
//! The statically typed call (`buf.readUInt8(o)`) and the by-name call through
//! an `any` (`buf[name](o)`) both reach the same native, so both share these
//! checks.

use super::value::Value;
use super::{Interp, RunError};
use zun_gc::Obj;
use zun_il::NumKind;

/// `buffer.kMaxLength` (2^53 - 1), the bound Node validates sizes against.
pub(crate) const K_MAX_LENGTH: f64 = 9_007_199_254_740_991.0;

/// An allocation past this many bytes cannot be satisfied by the host heap.
pub(crate) const ALLOC_LIMIT: f64 = 4_294_967_296.0 * 2.0;

/// The raw JavaScript value of an argument slot (`undefined` when absent).
pub(crate) fn raw(args: &[Value], index: usize) -> Value {
    match args.get(index) {
        None => Value::Void,
        Some(Value::Any(any)) => {
            zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)).unwrap_or(Value::Void)
        }
        Some(other) => other.clone(),
    }
}

pub(crate) fn as_number(value: &Value) -> Option<f64> {
    match value {
        Value::Float(number, _) => Some(*number),
        Value::Int(number, _) => Some(*number as f64),
        _ => None,
    }
}

fn is_bigint(value: &Value) -> bool {
    matches!(value, Value::Object(object)
        if zun_gc::with_current_heap(|heap| heap.bigint_value(*object).is_some()))
}

pub(crate) fn number_text(number: f64) -> String {
    if number == 0.0 {
        return "0".into();
    }
    zun_natives::js_to_radix_string(number, None).unwrap_or_else(|_| number.to_string())
}

/// `addNumericalSeparator`: `_` between thousands groups.
fn with_separators(digits: &str) -> String {
    let (sign, body) = match digits.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", digits),
    };
    let mut out = String::new();
    let len = body.len();
    for (index, ch) in body.chars().enumerate() {
        if index > 0 && (len - index) % 3 == 0 {
            out.push('_');
        }
        out.push(ch);
    }
    format!("{sign}{out}")
}

/// `util.inspect`-ish rendering of a primitive for `Received type string ('x')`.
pub(crate) fn inspect_primitive(value: &Value) -> String {
    match value {
        Value::Str(text) => {
            let mut quote = '\'';
            if text.contains('\'') {
                quote = if !text.contains('"') {
                    '"'
                } else if !text.contains('`') && !text.contains("${") {
                    '`'
                } else {
                    '\''
                };
            }
            let escaped = text
                .replace('\\', "\\\\")
                .replace('\n', "\\n")
                .replace('\t', "\\t")
                .replace('\r', "\\r");
            let escaped = if quote == '\'' {
                escaped.replace('\'', "\\'")
            } else {
                escaped
            };
            format!("{quote}{escaped}{quote}")
        }
        Value::Bool(flag) => flag.to_string(),
        Value::Float(number, _) => {
            if *number == 0.0 && number.is_sign_negative() {
                "-0".into()
            } else {
                number_text(*number)
            }
        }
        Value::Int(number, _) => number.to_string(),
        other => format!("{other:?}"),
    }
}

impl Interp<'_> {
    /// A Node-style error: `class` is `TypeError`/`RangeError`/`Error`, `code`
    /// the `ERR_*` identifier. Node's `name` stays the class name.
    pub(crate) fn nb_error(&mut self, class: &str, code: &str, message: String) -> RunError {
        let Some(object) = self.new_error_object(class, &message) else {
            return RunError::Native(format!("{class}: {message}"));
        };
        zun_gc::with_current_heap_mut(|heap| {
            // A typed class instance (the error) keeps expando properties in
            // its instance bag, exactly as `err.code = ..` does in user code.
            heap.ensure_instance_bag(object);
            let code = Value::Str(code.into()).box_any(heap).unwrap_or_else(zun_core::Any::void);
            heap.dynamic_assign(object, "code".to_string(), code);
        });
        RunError::Thrown(Value::Object(object))
    }

    /// The `Received ...` tail Node appends to `ERR_INVALID_ARG_TYPE`.
    pub(crate) fn nb_received(&self, value: &Value) -> String {
        match value {
            Value::Void => " Received undefined".into(),
            Value::Null => " Received null".into(),
            Value::Function(_) => " Received function".into(),
            Value::Object(object) => {
                if is_bigint(value) {
                    let text = zun_gc::with_current_heap(|heap| {
                        heap.bigint_value(*object).map(|b| b.to_string())
                    })
                    .unwrap_or_default();
                    return format!(" Received type bigint ({text}n)");
                }
                let class = zun_gc::with_current_heap(|heap| {
                    if heap.array_len(*object).is_some() {
                        "Array"
                    } else if let Some(kind) = heap.typed_array_element(*object) {
                        match kind {
                            zun_core::TypedElement::U8 => "Uint8Array",
                            _ => "TypedArray",
                        }
                    } else if heap.buffer_len(*object).is_some() {
                        "ArrayBuffer"
                    } else {
                        "Object"
                    }
                });
                format!(" Received an instance of {class}")
            }
            other => {
                let kind = match other {
                    Value::Str(_) => "string",
                    Value::Bool(_) => "boolean",
                    Value::Float(..) | Value::Int(..) => "number",
                    _ => "object",
                };
                let mut shown = inspect_primitive(other);
                if shown.chars().count() > 28 {
                    shown = format!("{}...", shown.chars().take(25).collect::<String>());
                }
                format!(" Received type {kind} ({shown})")
            }
        }
    }

    /// `new ERR_INVALID_ARG_TYPE(name, expected, value)` with `expected` already
    /// phrased (`of type number`, `an instance of Buffer or Uint8Array`, ...).
    pub(crate) fn nb_invalid_type(&mut self, name: &str, expected: &str, value: &Value) -> RunError {
        let subject = if name.ends_with(" argument") {
            format!("The {name}")
        } else if name.contains('.') {
            format!("The \"{name}\" property")
        } else {
            format!("The \"{name}\" argument")
        };
        let message = format!("{subject} must be {expected}.{}", self.nb_received(value));
        self.nb_error("TypeError", "ERR_INVALID_ARG_TYPE", message)
    }

    /// `new ERR_OUT_OF_RANGE(name, range, received)`.
    pub(crate) fn nb_out_of_range(&mut self, name: &str, range: &str, received: &Value) -> RunError {
        let shown = match received {
            Value::Object(object) if is_bigint(received) => {
                let text = zun_gc::with_current_heap(|heap| {
                    heap.bigint_value(*object).map(|b| b.to_string())
                })
                .unwrap_or_default();
                format!("{}n", with_separators(&text))
            }
            Value::Float(..) | Value::Int(..) => {
                let number = as_number(received).unwrap_or(f64::NAN);
                let text = number_text(number);
                if number.fract() == 0.0 && number.abs() > 4_294_967_296.0 {
                    with_separators(&text)
                } else {
                    text
                }
            }
            Value::Str(_) => inspect_primitive(received),
            other => inspect_primitive(other),
        };
        let message =
            format!("The value of \"{name}\" is out of range. It must be {range}. Received {shown}");
        self.nb_error("RangeError", "ERR_OUT_OF_RANGE", message)
    }

    /// `validateNumber(value, name[, min[, max]])`.
    pub(crate) fn nb_validate_number(
        &mut self,
        value: &Value,
        name: &str,
        min: Option<f64>,
        max: Option<f64>,
    ) -> Result<f64, RunError> {
        let Some(number) = as_number(value) else {
            return Err(self.nb_invalid_type(name, "of type number", value));
        };
        let bad = min.is_some_and(|m| number < m)
            || max.is_some_and(|m| number > m)
            || ((min.is_some() || max.is_some()) && number.is_nan());
        if bad {
            let range = match (min, max) {
                (Some(lo), Some(hi)) => format!(">= {} && <= {}", number_text(lo), number_text(hi)),
                (Some(lo), None) => format!(">= {}", number_text(lo)),
                (None, Some(hi)) => format!("<= {}", number_text(hi)),
                (None, None) => unreachable!(),
            };
            return Err(self.nb_out_of_range(name, &range, value));
        }
        Ok(number)
    }

    /// `boundsError(value, length, type)` from `lib/internal/buffer.js`.
    pub(crate) fn nb_bounds_error(&mut self, value: &Value, length: i64, ty: Option<&str>) -> RunError {
        let number = as_number(value);
        if number.is_none_or(|n| n.floor() != n) {
            if let Err(error) = self.nb_validate_number(value, ty.unwrap_or("offset"), None, None) {
                return error;
            }
            return self.nb_out_of_range(ty.unwrap_or("offset"), "an integer", value);
        }
        if length < 0 {
            return self.nb_error(
                "RangeError",
                "ERR_BUFFER_OUT_OF_BOUNDS",
                "Attempt to access memory outside buffer bounds".into(),
            );
        }
        let range = format!(">= {} and <= {length}", if ty.is_some() { 1 } else { 0 });
        self.nb_out_of_range(ty.unwrap_or("offset"), &range, value)
    }

    /// `+value`: JavaScript ToNumber for the primitives a Buffer writer sees.
    pub(crate) fn nb_to_number(&mut self, value: &Value) -> Result<f64, RunError> {
        Ok(match value {
            Value::Float(number, _) => *number,
            Value::Int(number, _) => *number as f64,
            Value::Bool(flag) => f64::from(u8::from(*flag)),
            Value::Null => 0.0,
            Value::Void => f64::NAN,
            Value::Str(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    0.0
                } else if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
                    u64::from_str_radix(hex, 16).map(|n| n as f64).unwrap_or(f64::NAN)
                } else {
                    trimmed.parse::<f64>().unwrap_or(f64::NAN)
                }
            }
            _ if is_bigint(value) => {
                return Err(self.view_error("TypeError", "Cannot convert a BigInt value to a number"));
            }
            _ => f64::NAN,
        })
    }

    /// The offset of a fixed-width access: `validateNumber(offset)` (an absent
    /// or `undefined` offset is `0`), then Node's `this[offset]` /
    /// `this[offset + width - 1]` probe, which `boundsError`s on a miss.
    pub(crate) fn nb_access_offset(
        &mut self,
        view: Obj,
        offset: &Value,
        width: usize,
    ) -> Result<usize, RunError> {
        let offset = if matches!(offset, Value::Void) {
            Value::Float(0.0, NumKind::F64)
        } else {
            offset.clone()
        };
        let number = self.nb_validate_number(&offset, "offset", None, None)?;
        let len = super::node_buffer::view_len(view).unwrap_or(0) as i64;
        let last = width as i64 - 1;
        if number.fract() != 0.0 || number < 0.0 || number + last as f64 >= len as f64 {
            return Err(self.nb_bounds_error(&offset, len - width as i64, None));
        }
        Ok(number as usize)
    }

    /// `validateNumber(size, 'size', 0, kMaxLength)` plus the host's own limit.
    pub(crate) fn nb_size(&mut self, size: &Value) -> Result<usize, RunError> {
        let number = self.nb_validate_number(size, "size", Some(0.0), Some(K_MAX_LENGTH))?;
        if number > ALLOC_LIMIT {
            return Err(self.view_error("RangeError", "Array buffer allocation failed"));
        }
        Ok(number as usize)
    }

    fn nb_int_range(width: usize, signed: bool) -> (f64, f64) {
        let bits = (width * 8) as i32;
        if signed {
            (-(2f64.powi(bits - 1)), 2f64.powi(bits - 1) - 1.0)
        } else {
            (0.0, 2f64.powi(bits) - 1.0)
        }
    }

    /// `checkInt` + the writer bounds probe for an integer of `width` bytes.
    fn nb_write_int(
        &mut self,
        view: Obj,
        value: &Value,
        offset: &Value,
        width: usize,
        signed: bool,
        little_endian: bool,
    ) -> Result<Value, RunError> {
        let number = self.nb_to_number(value)?;
        let (min, max) = Self::nb_int_range(width, signed);
        let byte_length = width - 1;
        let out_of_range = |this: &mut Self| {
            let range = if byte_length > 3 {
                if !signed {
                    format!(">= 0 and < 2 ** {}", (byte_length + 1) * 8)
                } else {
                    format!(
                        ">= -(2 ** {}) and < 2 ** {}",
                        (byte_length + 1) * 8 - 1,
                        (byte_length + 1) * 8 - 1
                    )
                }
            } else {
                format!(">= {} and <= {}", number_text(min), number_text(max))
            };
            this.nb_out_of_range("value", &range, value)
        };
        let target = if width == 1 {
            // `writeU_Int8` validates the offset before the value.
            self.nb_validate_number(
                &if matches!(offset, Value::Void) { Value::Float(0.0, NumKind::F64) } else { offset.clone() },
                "offset",
                None,
                None,
            )?;
            if number > max || number < min {
                return Err(out_of_range(self));
            }
            self.nb_access_offset(view, offset, 1)?
        } else {
            if number > max || number < min {
                return Err(out_of_range(self));
            }
            self.nb_access_offset(view, offset, width)?
        };
        let bits = if number.is_nan() || number.is_infinite() {
            0u64
        } else {
            number.trunc() as i128 as u64
        };
        super::node_buffer::write_uint(view, target, bits, width, little_endian)
            .ok_or(RunError::BadToken)?;
        Ok(Value::Int((target + width) as i64, NumKind::U64))
    }

    /// `readBig(U)Int64` / `writeBig(U)Int64`.
    fn nb_bigint_access(
        &mut self,
        view: Obj,
        args: &[Value],
        write: bool,
        signed: bool,
        little_endian: bool,
    ) -> Result<Value, RunError> {
        if !write {
            let offset = self.nb_access_offset(view, &raw(args, 1), 8)?;
            let bits = super::node_buffer::read_uint(view, offset, 8, little_endian)
                .ok_or(RunError::BadToken)?;
            let big = if signed {
                zun_core::BigInt::from_i64(bits as i64)
            } else {
                zun_core::BigInt::from_u64(bits)
            };
            return Ok(zun_gc::with_current_heap_mut(|heap| {
                let object = heap.alloc_bigint(big);
                Value::Object(object).box_any(heap)
            })
            .map(Value::Any)
            .unwrap_or(Value::Void));
        }
        let value = raw(args, 2);
        let offset_value = raw(args, 1);
        let big = match &value {
            Value::Object(object) => {
                zun_gc::with_current_heap(|heap| heap.bigint_value(*object).cloned())
            }
            _ => None,
        };
        let Some(big) = big else {
            return Err(self.view_error("TypeError", "Cannot convert value to a BigInt"));
        };
        let (min, max) = if signed {
            (
                zun_core::BigInt::from_i64(i64::MIN),
                zun_core::BigInt::from_i64(i64::MAX),
            )
        } else {
            (zun_core::BigInt::zero(), zun_core::BigInt::from_u64(u64::MAX))
        };
        if big.cmp(&min) == std::cmp::Ordering::Less || big.cmp(&max) == std::cmp::Ordering::Greater {
            let range = if signed {
                ">= -(2n ** 63n) and < 2n ** 63n"
            } else {
                ">= 0n and < 2n ** 64n"
            };
            return Err(self.nb_out_of_range("value", range, &value));
        }
        let offset = self.nb_access_offset(view, &offset_value, 8)?;
        let bits = big.as_uint_n(64).to_u64().unwrap_or(0);
        super::node_buffer::write_uint(view, offset, bits, 8, little_endian)
            .ok_or(RunError::BadToken)?;
        Ok(Value::Int((offset + 8) as i64, NumKind::U64))
    }

    /// Validate `byteLength` of the variable-width accessors.
    fn nb_byte_length(&mut self, value: &Value) -> Result<usize, RunError> {
        match as_number(value) {
            Some(n) if n.fract() == 0.0 && (1.0..=6.0).contains(&n) => Ok(n as usize),
            _ => Err(self.nb_bounds_error(value, 6, Some("byteLength"))),
        }
    }

    /// Run the validated `nodeBuffer` reader/writer/allocator intrinsics, or
    /// `None` when `name` is not one of them (the plain natives handle it).
    pub(crate) fn node_buffer_checked(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        let tail = name.strip_prefix("nodeBuffer")?;
        if let Some(result) = self.node_buffer_ops(tail, args) {
            return Some(result);
        }
        let is_read = tail.starts_with("Read");
        let is_write = tail.starts_with("Write") && tail != "Write" && !tail.starts_with("WriteString");
        if !is_read && !is_write {
            return None;
        }
        let view = match args.first() {
            Some(Value::Object(view)) => *view,
            _ => return Some(Err(RunError::BadToken)),
        };
        let kind = &tail[if is_read { 4 } else { 5 }..];
        let little_endian = !kind.ends_with("BE");
        let outcome = (|| -> Result<Value, RunError> {
            // Variable-width integers.
            if matches!(kind, "UIntLE" | "UIntBE" | "IntLE" | "IntBE") {
                let signed = kind.starts_with("Int");
                if is_read {
                    let offset_value = raw(args, 1);
                    if matches!(offset_value, Value::Void) {
                        return Err(self.nb_invalid_type("offset", "of type number", &offset_value));
                    }
                    let width = self.nb_byte_length(&raw(args, 2))?;
                    let offset = self.nb_access_offset(view, &offset_value, width)?;
                    let bits = super::node_buffer::read_uint(view, offset, width, little_endian)
                        .ok_or(RunError::BadToken)?;
                    let value = if signed {
                        super::node_buffer::sign_extend(bits, width) as f64
                    } else {
                        bits as f64
                    };
                    return Ok(Value::Float(value, NumKind::F64));
                }
                let width = self.nb_byte_length(&raw(args, 3))?;
                // The variable-width writers have no default offset.
                let offset_value = raw(args, 1);
                if matches!(offset_value, Value::Void) {
                    let number = self.nb_to_number(&raw(args, 2))?;
                    let (min, max) = Self::nb_int_range(width, signed);
                    if number > max || number < min {
                        // `checkInt` reports the value first.
                        return self.nb_write_int(view, &raw(args, 2), &Value::Float(0.0, NumKind::F64), width, signed, little_endian);
                    }
                    return Err(self.nb_invalid_type("offset", "of type number", &offset_value));
                }
                return self.nb_write_int(view, &raw(args, 2), &raw(args, 1), width, signed, little_endian);
            }
            if kind.starts_with("Big") {
                let signed = kind.starts_with("BigInt");
                return self.nb_bigint_access(view, args, is_write, signed, little_endian);
            }
            if kind.starts_with("Float") || kind.starts_with("Double") {
                let width = if kind.starts_with("Float") { 4 } else { 8 };
                if is_read {
                    let offset = self.nb_access_offset(view, &raw(args, 1), width)?;
                    let bits = super::node_buffer::read_uint(view, offset, width, little_endian)
                        .ok_or(RunError::BadToken)?;
                    let number = if width == 4 {
                        f64::from(f32::from_bits(bits as u32))
                    } else {
                        f64::from_bits(bits)
                    };
                    return Ok(Value::Float(number, NumKind::F64));
                }
                let number = self.nb_to_number(&raw(args, 2))?;
                let offset = self.nb_access_offset(view, &raw(args, 1), width)?;
                let bits = if width == 4 {
                    u64::from((number as f32).to_bits())
                } else {
                    number.to_bits()
                };
                super::node_buffer::write_uint(view, offset, bits, width, little_endian)
                    .ok_or(RunError::BadToken)?;
                return Ok(Value::Int((offset + width) as i64, NumKind::U64));
            }
            // Fixed-width integers: `UInt8`, `Int16LE`, ...
            let signed = !kind.starts_with("UInt");
            let width = if kind.contains("32") {
                4
            } else if kind.contains("16") {
                2
            } else if kind.contains('8') {
                1
            } else {
                return Err(RunError::BadToken);
            };
            if is_read {
                let offset = self.nb_access_offset(view, &raw(args, 1), width)?;
                let bits = super::node_buffer::read_uint(view, offset, width, little_endian)
                    .ok_or(RunError::BadToken)?;
                let (value, kind) = if signed {
                    (
                        super::node_buffer::sign_extend(bits, width),
                        match width {
                            1 => NumKind::I8,
                            2 => NumKind::I16,
                            _ => NumKind::I32,
                        },
                    )
                } else {
                    (
                        bits as i64,
                        match width {
                            1 => NumKind::U8,
                            2 => NumKind::U16,
                            _ => NumKind::U32,
                        },
                    )
                };
                return Ok(Value::Int(value, kind));
            }
            self.nb_write_int(view, &raw(args, 2), &raw(args, 1), width, signed, little_endian)
        })();
        Some(outcome)
    }
}
