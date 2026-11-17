//! The validated, non-accessor `nodeBuffer` intrinsics: allocation, `from`,
//! `fill`, `toString`, `write`, `slice`, `copy`, `concat`, `compare`,
//! `equals`, `indexOf` and friends, each following Node's `lib/buffer.js`
//! argument handling (see `node_buffer_checked` for the shared validators).

use super::node_buffer::{self as nb, buffer_from_bytes, view_bytes, view_len};
use super::node_buffer_checked::{
    as_number, inspect_primitive, number_text, raw, ALLOC_LIMIT, K_MAX_LENGTH,
};
use super::value::Value;
use super::{Interp, RunError};
use zun_gc::Obj;
use zun_il::NumKind;

/// Node's `normalizeEncoding`, strict: `None` for a name Node does not know.
pub(crate) fn strict_encoding(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "utf8" | "utf-8" => "utf8",
        "ucs2" | "ucs-2" | "utf16le" | "utf-16le" => "utf16le",
        "latin1" | "binary" => "latin1",
        "base64" => "base64",
        "base64url" => "base64url",
        "hex" => "hex",
        "ascii" => "ascii",
        _ => return None,
    })
}

fn is_present(value: &Value) -> bool {
    !matches!(value, Value::Void)
}

fn trunc(number: f64) -> f64 {
    if number.is_nan() {
        0.0
    } else {
        number.trunc()
    }
}

/// `adjustOffset` from `lib/buffer.js` (the `slice`/`subarray` clamp).
fn adjust_offset(number: f64, length: usize) -> usize {
    let offset = trunc(number);
    if offset == 0.0 {
        return 0;
    }
    if offset < 0.0 {
        let shifted = offset + length as f64;
        return if shifted > 0.0 { shifted as usize } else { 0 };
    }
    if offset < length as f64 {
        offset as usize
    } else {
        length
    }
}

fn is_u8_view(object: Obj) -> bool {
    zun_gc::with_current_heap(|heap| {
        heap.typed_array_element(object) == Some(zun_core::TypedElement::U8)
    })
}

fn byte_view(value: &Value) -> Option<Obj> {
    match value {
        Value::Object(object) if is_u8_view(*object) => Some(*object),
        _ => None,
    }
}

fn to_byte(number: f64) -> u8 {
    if number.is_finite() {
        (number.trunc() as i128).rem_euclid(256) as u8
    } else {
        0
    }
}

impl Interp<'_> {
    /// `getEncodingOps(encoding)` / `normalizeEncoding`: `undefined`, `null`
    /// and `''` are utf8; an unknown name is `ERR_UNKNOWN_ENCODING`.
    fn nb_encoding(&mut self, encoding: &Value) -> Result<&'static str, RunError> {
        match encoding {
            Value::Void | Value::Null => Ok("utf8"),
            Value::Str(text) if text.is_empty() => Ok("utf8"),
            Value::Str(text) => match strict_encoding(text) {
                Some(name) => Ok(name),
                None => Err(self.nb_unknown_encoding(encoding)),
            },
            other => Err(self.nb_unknown_encoding(other)),
        }
    }

    fn nb_unknown_encoding(&mut self, encoding: &Value) -> RunError {
        let shown = match encoding {
            Value::Str(text) => text.to_string(),
            Value::Float(..) | Value::Int(..) => inspect_primitive(encoding),
            Value::Bool(flag) => flag.to_string(),
            Value::Null => "null".into(),
            Value::Void => "undefined".into(),
            _ => "[object Object]".into(),
        };
        self.nb_error(
            "TypeError",
            "ERR_UNKNOWN_ENCODING",
            format!("Unknown encoding: {shown}"),
        )
    }

    /// `validateInteger(value, name, min, max)` (also what `validateOffset` is).
    fn nb_validate_integer(
        &mut self,
        value: &Value,
        name: &str,
        min: f64,
        max: f64,
    ) -> Result<f64, RunError> {
        let number = self.nb_validate_number(value, name, None, None)?;
        if number.is_nan() || number.is_infinite() || number.fract() != 0.0 {
            return Err(self.nb_out_of_range(name, "an integer", value));
        }
        if number < min || number > max {
            let range = format!(">= {} && <= {}", number_text(min), number_text(max));
            return Err(self.nb_out_of_range(name, &range, value));
        }
        Ok(number)
    }

    fn nb_offset(&mut self, value: &Value, name: &str, max: f64) -> Result<usize, RunError> {
        Ok(self.nb_validate_integer(value, name, 0.0, max)? as usize)
    }

    fn nb_not_u8(&mut self, name: &str, value: &Value) -> RunError {
        self.nb_invalid_type(name, "an instance of Buffer or Uint8Array", value)
    }

    fn nb_u8_arg(&mut self, value: &Value, name: &str) -> Result<Obj, RunError> {
        match byte_view(value) {
            Some(view) => Ok(view),
            None => Err(self.nb_not_u8(name, value)),
        }
    }

    fn nb_oob(&mut self) -> RunError {
        self.nb_error(
            "RangeError",
            "ERR_BUFFER_OUT_OF_BOUNDS",
            "Attempt to access memory outside buffer bounds".into(),
        )
    }

    /// The bytes a `fill(value, ..., encoding)` repeats, or `ERR_INVALID_ARG_VALUE`.
    fn nb_fill_pattern(&mut self, value: &Value, encoding: &Value) -> Result<Vec<u8>, RunError> {
        let invalid = |this: &mut Self| {
            let message = format!(
                "The argument 'value' is invalid. Received {}",
                inspect_primitive(value)
            );
            this.nb_error("TypeError", "ERR_INVALID_ARG_VALUE", message)
        };
        match value {
            Value::Str(text) => {
                let encoding = self.nb_encoding(encoding)?;
                if text.is_empty() {
                    return Ok(vec![0]);
                }
                let bytes = nb::encode(text, encoding);
                if bytes.is_empty() {
                    return Err(invalid(self));
                }
                Ok(bytes)
            }
            Value::Object(object) if is_u8_view(*object) => {
                let bytes = view_bytes(*object).unwrap_or_default();
                if bytes.is_empty() {
                    return Err(invalid(self));
                }
                Ok(bytes)
            }
            other => {
                let number = self.nb_to_number(other)?;
                Ok(vec![to_byte(number)])
            }
        }
    }

    fn nb_new_buffer(&mut self, bytes: &[u8]) -> Result<Value, RunError> {
        buffer_from_bytes(bytes).ok_or(RunError::BadToken)
    }

    /// `Buffer.alloc(size, fill, encoding)`.
    fn nb_alloc(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let size = self.nb_size(&raw(args, 0))?;
        let fill = raw(args, 1);
        let mut bytes = vec![0u8; size];
        let zero = !is_present(&fill) || as_number(&fill) == Some(0.0);
        if !zero && size > 0 {
            let pattern = self.nb_fill_pattern(&fill, &raw(args, 2))?;
            bytes = nb::repeat_to(&pattern, size);
        }
        self.nb_new_buffer(&bytes)
    }

    /// `buf.fill(value, offset, end, encoding)` (`_fill`).
    fn nb_fill(&mut self, view: Obj, args: &[Value]) -> Result<Value, RunError> {
        let len = view_len(view).unwrap_or(0);
        let value = raw(args, 1);
        let mut offset = raw(args, 2);
        let mut end = raw(args, 3);
        let mut encoding = raw(args, 4);
        if matches!(value, Value::Str(_)) {
            if !is_present(&offset) || matches!(offset, Value::Str(_)) {
                encoding = offset.clone();
                offset = Value::Void;
                end = Value::Void;
            } else if matches!(end, Value::Str(_)) {
                encoding = end.clone();
                end = Value::Void;
            }
            if !matches!(encoding, Value::Void | Value::Null | Value::Str(_)) {
                return Err(self.nb_invalid_type("encoding", "of type string", &encoding));
            }
            self.nb_encoding(&encoding)?;
        } else {
            encoding = Value::Void;
        }
        let (start, stop);
        if !is_present(&offset) {
            start = 0;
            stop = len;
        } else {
            start = self.nb_offset(&offset, "offset", K_MAX_LENGTH)?;
            stop = if !is_present(&end) {
                len
            } else {
                self.nb_offset(&end, "end", len as f64)?
            };
            if start >= stop {
                return Ok(Value::Object(view));
            }
        }
        if start > stop || stop > len {
            return Err(self.nb_oob());
        }
        let pattern = self.nb_fill_pattern(&value, &encoding)?;
        let filled = nb::repeat_to(&pattern, stop - start);
        zun_gc::with_current_heap_mut(|heap| {
            for (index, byte) in filled.iter().enumerate() {
                heap.u8_array_set(view, start + index, *byte);
            }
        });
        Ok(Value::Object(view))
    }

    /// `buf.toString(encoding, start, end)`.
    fn nb_to_string(&mut self, view: Obj, args: &[Value]) -> Result<Value, RunError> {
        let bytes = view_bytes(view).ok_or(RunError::BadToken)?;
        let len = bytes.len();
        let encoding = raw(args, 1);
        let start_arg = raw(args, 2);
        let end_arg = raw(args, 3);
        let text = |s: String| Ok(Value::Str(zun_core::ZStr::from(s.as_str())));
        if !is_present(&encoding) && !is_present(&start_arg) && !is_present(&end_arg) {
            return text(nb::decode(&bytes, "utf8"));
        }
        let start_number = self.nb_to_number(&start_arg)?;
        let start = if start_number <= 0.0 {
            0
        } else if start_number >= len as f64 {
            return text(String::new());
        } else {
            trunc(start_number) as usize
        };
        let end = if !is_present(&end_arg) {
            len
        } else {
            let end_number = self.nb_to_number(&end_arg)?;
            if end_number > len as f64 {
                len
            } else {
                let t = trunc(end_number);
                if t < 0.0 {
                    0
                } else {
                    t as usize
                }
            }
        };
        if end <= start {
            return text(String::new());
        }
        let encoding = match &encoding {
            Value::Void => "utf8",
            other => self.nb_encoding(other)?,
        };
        text(nb::decode(&bytes[start..end], encoding))
    }

    /// `buf.write(string, offset, length, encoding)`.
    fn nb_write_string(&mut self, view: Obj, args: &[Value]) -> Result<Value, RunError> {
        let len = view_len(view).unwrap_or(0);
        let string = raw(args, 1);
        let offset = raw(args, 2);
        let length = raw(args, 3);
        let mut encoding = raw(args, 4);
        let Value::Str(text) = &string else {
            return Err(self.nb_invalid_type("argument", "of type string", &string));
        };
        let start;
        let count;
        if !is_present(&offset) {
            start = 0;
            count = len;
        } else if !is_present(&length) && matches!(offset, Value::Str(_)) {
            encoding = offset.clone();
            start = 0;
            count = len;
        } else {
            start = self.nb_offset(&offset, "offset", len as f64)?;
            let remaining = len - start;
            if !is_present(&length) {
                count = remaining;
            } else if matches!(length, Value::Str(_)) {
                encoding = length.clone();
                count = remaining;
            } else {
                let wanted = self.nb_offset(&length, "length", len as f64)?;
                count = wanted.min(remaining);
            }
        }
        let enc = self.nb_encoding(&encoding)?;
        let mut bytes = nb::encode(text, enc);
        if bytes.len() > count {
            let mut cut = count;
            match enc {
                "utf8" => {
                    while cut > 0 && (bytes[cut] & 0xc0) == 0x80 {
                        cut -= 1;
                    }
                }
                "utf16le" => cut -= cut % 2,
                _ => {}
            }
            bytes.truncate(cut);
        }
        zun_gc::with_current_heap_mut(|heap| {
            for (index, byte) in bytes.iter().enumerate() {
                heap.u8_array_set(view, start + index, *byte);
            }
        });
        Ok(Value::Int(bytes.len() as i64, NumKind::U64))
    }

    /// `buf.slice(start, end)` / `buf.subarray(start, end)`.
    fn nb_slice(&mut self, view: Obj, args: &[Value]) -> Result<Value, RunError> {
        let len = view_len(view).unwrap_or(0);
        let start_arg = raw(args, 1);
        let end_arg = raw(args, 2);
        let start = adjust_offset(self.nb_to_number(&start_arg)?, len);
        let end = if is_present(&end_arg) {
            adjust_offset(self.nb_to_number(&end_arg)?, len)
        } else {
            len
        };
        let end = end.max(start);
        let child =
            zun_gc::with_current_heap_mut(|heap| heap.typed_array_subarray(view, start, end))
                .ok_or(RunError::BadToken)?;
        Ok(Value::Object(child))
    }

    /// `toInteger(value, default)` from `lib/buffer.js`.
    fn nb_to_integer(&mut self, value: &Value, default: f64) -> Result<f64, RunError> {
        let number = self.nb_to_number(value)?;
        if !number.is_nan() && (-9_007_199_254_740_991.0..=9_007_199_254_740_991.0).contains(&number)
        {
            Ok(number.trunc())
        } else {
            Ok(default)
        }
    }

    /// `buf.copy(target, targetStart, sourceStart, sourceEnd)`.
    fn nb_copy(&mut self, source: Obj, args: &[Value]) -> Result<Value, RunError> {
        let target_value = raw(args, 1);
        let target = self.nb_u8_arg(&target_value, "target")?;
        let source_len = view_len(source).unwrap_or(0);
        let target_len = view_len(target).unwrap_or(0);
        let target_start_arg = raw(args, 2);
        let source_start_arg = raw(args, 3);
        let source_end_arg = raw(args, 4);
        let target_start = if !is_present(&target_start_arg) {
            0.0
        } else {
            let n = self.nb_to_integer(&target_start_arg, 0.0)?;
            if n < 0.0 {
                return Err(self.nb_out_of_range(
                    "targetStart",
                    ">= 0",
                    &Value::Float(n, NumKind::F64),
                ));
            }
            n
        };
        let source_start = if !is_present(&source_start_arg) {
            0.0
        } else {
            let n = self.nb_to_integer(&source_start_arg, 0.0)?;
            if n < 0.0 || n > source_len as f64 {
                let range = format!(">= 0 && <= {source_len}");
                return Err(self.nb_out_of_range(
                    "sourceStart",
                    &range,
                    &Value::Float(n, NumKind::F64),
                ));
            }
            n
        };
        let mut source_end = if !is_present(&source_end_arg) {
            source_len as f64
        } else {
            let n = self.nb_to_integer(&source_end_arg, 0.0)?;
            if n < 0.0 {
                return Err(self.nb_out_of_range(
                    "sourceEnd",
                    ">= 0",
                    &Value::Float(n, NumKind::F64),
                ));
            }
            n
        };
        if target_start >= target_len as f64 || source_start >= source_end {
            return Ok(Value::Int(0, NumKind::U64));
        }
        if source_end - source_start > target_len as f64 - target_start {
            source_end = source_start + target_len as f64 - target_start;
        }
        let mut count = source_end - source_start;
        let available = source_len as f64 - source_start;
        if count > available {
            count = available;
        }
        if count <= 0.0 {
            return Ok(Value::Int(0, NumKind::U64));
        }
        let bytes = view_bytes(source).ok_or(RunError::BadToken)?;
        let (ss, ts, n) = (source_start as usize, target_start as usize, count as usize);
        zun_gc::with_current_heap_mut(|heap| {
            for i in 0..n {
                heap.u8_array_set(target, ts + i, bytes[ss + i]);
            }
        });
        Ok(Value::Int(n as i64, NumKind::U64))
    }

    fn nb_array_items(array: Obj) -> Option<Vec<zun_core::Any>> {
        zun_gc::with_current_heap(|heap| {
            let len = heap.array_len(array)?;
            (0..len).map(|i| heap.array_get(array, i)).collect::<Option<Vec<_>>>()
        })
    }

    /// `Buffer.concat(list, length)`.
    fn nb_concat(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let list = raw(args, 0);
        let items = match &list {
            Value::Object(object) => Self::nb_array_items(*object),
            _ => None,
        };
        let Some(items) = items else {
            return Err(self.nb_invalid_type("list", "an instance of Array", &list));
        };
        if items.is_empty() {
            return self.nb_new_buffer(&[]);
        }
        let mut views = Vec::with_capacity(items.len());
        for (index, any) in items.iter().enumerate() {
            let value = zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                .unwrap_or(Value::Void);
            let Some(view) = byte_view(&value) else {
                return Err(self.nb_not_u8(&format!("list[{index}]"), &value));
            };
            views.push(view);
        }
        let length_arg = raw(args, 1);
        let total = if is_present(&length_arg) {
            self.nb_offset(&length_arg, "length", K_MAX_LENGTH)?
        } else {
            views.iter().map(|v| view_len(*v).unwrap_or(0)).sum()
        };
        if total as f64 > ALLOC_LIMIT {
            return Err(self.view_error("RangeError", "Array buffer allocation failed"));
        }
        let mut out = Vec::with_capacity(total);
        for view in views {
            if out.len() >= total {
                break;
            }
            let bytes = view_bytes(view).unwrap_or_default();
            let room = total - out.len();
            out.extend_from_slice(&bytes[..bytes.len().min(room)]);
        }
        out.resize(total, 0);
        self.nb_new_buffer(&out)
    }

    fn nb_compare_bytes(a: &[u8], b: &[u8]) -> f64 {
        match a.cmp(b) {
            std::cmp::Ordering::Less => -1.0,
            std::cmp::Ordering::Equal => 0.0,
            std::cmp::Ordering::Greater => 1.0,
        }
    }

    fn nb_array_bytes(&mut self, array: Obj) -> Result<Option<Vec<u8>>, RunError> {
        let Some(items) = Self::nb_array_items(array) else {
            return Ok(None);
        };
        let mut bytes = Vec::with_capacity(items.len());
        for any in items {
            let item = zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                .unwrap_or(Value::Void);
            let n = self.nb_to_number(&item)?;
            bytes.push(to_byte(n));
        }
        Ok(Some(bytes))
    }

    /// `Buffer.from(value, encodingOrOffset, length)` for a run-time value.
    fn nb_from(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let value = raw(args, 0);
        let second = raw(args, 1);
        let third = raw(args, 2);
        const EXPECTED: &str =
            "of type string or an instance of Buffer, ArrayBuffer, or Array or an Array-like Object";
        match &value {
            Value::Str(text) => {
                let encoding = match &second {
                    Value::Str(name) if !name.is_empty() => self.nb_encoding(&second)?,
                    _ => "utf8",
                };
                if text.is_empty() {
                    return self.nb_new_buffer(&[]);
                }
                self.nb_new_buffer(&nb::encode(text, encoding))
            }
            Value::Object(object) => {
                let buffer_len = zun_gc::with_current_heap(|heap| heap.buffer_len(*object));
                if let Some(buffer_len) = buffer_len {
                    let offset = if is_present(&second) {
                        let n = self.nb_to_number(&second)?;
                        if n.is_nan() {
                            0.0
                        } else {
                            n
                        }
                    } else {
                        0.0
                    };
                    let max_length = buffer_len as f64 - offset;
                    if max_length < 0.0 {
                        return Err(self.nb_error(
                            "RangeError",
                            "ERR_BUFFER_OUT_OF_BOUNDS",
                            "\"offset\" is outside of buffer bounds".into(),
                        ));
                    }
                    let length = if is_present(&third) {
                        let n = self.nb_to_number(&third)?;
                        if n > 0.0 {
                            if n > max_length {
                                return Err(self.nb_error(
                                    "RangeError",
                                    "ERR_BUFFER_OUT_OF_BOUNDS",
                                    "\"length\" is outside of buffer bounds".into(),
                                ));
                            }
                            n
                        } else {
                            0.0
                        }
                    } else {
                        max_length
                    };
                    let view = zun_gc::with_current_heap_mut(|heap| {
                        heap.alloc_typed_array(
                            *object,
                            offset.max(0.0) as usize,
                            length as usize,
                            zun_core::TypedElement::U8,
                        )
                    });
                    return view.map(Value::Object).ok_or(RunError::BadToken);
                }
                if is_u8_view(*object) {
                    let bytes = view_bytes(*object).unwrap_or_default();
                    return self.nb_new_buffer(&bytes);
                }
                if zun_gc::with_current_heap(|heap| heap.typed_array_element(*object).is_some()) {
                    let values = self.view_collect(*object);
                    let bytes: Vec<u8> = values
                        .iter()
                        .map(|item| to_byte(as_number(item).unwrap_or(0.0)))
                        .collect();
                    return self.nb_new_buffer(&bytes);
                }
                if let Some(bytes) = self.nb_array_bytes(*object)? {
                    return self.nb_new_buffer(&bytes);
                }
                let get = |name: &str| {
                    zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get(*object, name)
                            .and_then(|any| Value::unbox_any(any, heap))
                    })
                };
                if let Some(length) = get("length") {
                    if let Some(n) = as_number(&length) {
                        let mut bytes = Vec::new();
                        let count = if n.is_finite() && n > 0.0 { n as usize } else { 0 };
                        for index in 0..count {
                            let item = get(&index.to_string()).unwrap_or(Value::Void);
                            let n = self.nb_to_number(&item)?;
                            bytes.push(to_byte(n));
                        }
                        return self.nb_new_buffer(&bytes);
                    }
                    if !matches!(length, Value::Void) {
                        return self.nb_new_buffer(&[]);
                    }
                }
                if matches!(get("type"), Some(Value::Str(ref t)) if &**t == "Buffer") {
                    if let Some(Value::Object(data)) = get("data") {
                        if let Some(bytes) = self.nb_array_bytes(data)? {
                            return self.nb_new_buffer(&bytes);
                        }
                    }
                }
                Err(self.nb_invalid_type("first argument", EXPECTED, &value))
            }
            _ => Err(self.nb_invalid_type("first argument", EXPECTED, &value)),
        }
    }

    /// `Buffer.byteLength(value, encoding)`.
    fn nb_byte_length_of(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let value = raw(args, 0);
        let encoding = raw(args, 1);
        let number = |n: usize| Ok(Value::Int(n as i64, NumKind::U64));
        const EXPECTED: &str = "of type string or an instance of Buffer or ArrayBuffer";
        match &value {
            Value::Str(text) => {
                if text.is_empty() {
                    return number(0);
                }
                let name = match &encoding {
                    Value::Str(name) => strict_encoding(name).unwrap_or("utf8"),
                    _ => "utf8",
                };
                if name == "ascii" {
                    return number(text.encode_utf16().count());
                }
                number(nb::encode(text, name).len())
            }
            Value::Object(object) => {
                let size = zun_gc::with_current_heap(|heap| {
                    heap.typed_array_byte_len(*object)
                        .or_else(|| heap.buffer_len(*object))
                        .or_else(|| heap.data_view_byte_len(*object))
                });
                match size {
                    Some(size) => number(size),
                    None => Err(self.nb_invalid_type("string", EXPECTED, &value)),
                }
            }
            _ => Err(self.nb_invalid_type("string", EXPECTED, &value)),
        }
    }

    /// `buf.compare(target, targetStart, targetEnd, sourceStart, sourceEnd)`.
    fn nb_compare_range(&mut self, source: Obj, args: &[Value]) -> Result<Value, RunError> {
        let target_value = raw(args, 1);
        let target = self.nb_u8_arg(&target_value, "target")?;
        let source_bytes = view_bytes(source).unwrap_or_default();
        let target_bytes = view_bytes(target).unwrap_or_default();
        let float = |n: f64| Ok(Value::Float(n, NumKind::F64));
        let explicit = (2..=5).any(|i| is_present(&raw(args, i)));
        if !explicit {
            return float(Self::nb_compare_bytes(&source_bytes, &target_bytes));
        }
        let t_len = target_bytes.len();
        let s_len = source_bytes.len();
        let target_start = match raw(args, 2) {
            Value::Void => 0,
            v => self.nb_offset(&v, "targetStart", K_MAX_LENGTH)?,
        };
        let target_end = match raw(args, 3) {
            Value::Void => t_len,
            v => self.nb_offset(&v, "targetEnd", t_len as f64)?,
        };
        let source_start = match raw(args, 4) {
            Value::Void => 0,
            v => self.nb_offset(&v, "sourceStart", K_MAX_LENGTH)?,
        };
        let source_end = match raw(args, 5) {
            Value::Void => s_len,
            v => self.nb_offset(&v, "sourceEnd", s_len as f64)?,
        };
        if source_start >= source_end {
            return float(if target_start >= target_end { 0.0 } else { -1.0 });
        }
        if target_start >= target_end {
            return float(1.0);
        }
        if source_start > s_len {
            let shown = Value::Float(source_start as f64, NumKind::F64);
            return Err(self.nb_out_of_range("sourceStart", &format!(">= 0 && <= {s_len}"), &shown));
        }
        if target_start > t_len {
            let shown = Value::Float(target_start as f64, NumKind::F64);
            return Err(self.nb_out_of_range("targetStart", &format!(">= 0 && <= {t_len}"), &shown));
        }
        let source_slice = &source_bytes[source_start..source_end.min(s_len)];
        let target_slice = &target_bytes[target_start..target_end.min(t_len)];
        float(Self::nb_compare_bytes(source_slice, target_slice))
    }

    /// `Buffer.copyBytesFrom(view, offset, length)`.
    fn nb_copy_bytes_from(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let view_value = raw(args, 0);
        let element = match &view_value {
            Value::Object(object) => zun_gc::with_current_heap(|heap| {
                heap.typed_array_element(*object).map(|kind| (*object, kind.size()))
            }),
            _ => None,
        };
        let Some((view, size)) = element else {
            return Err(self.nb_invalid_type("view", "an instance of TypedArray", &view_value));
        };
        let view_length = zun_gc::with_current_heap(|heap| heap.typed_array_len(view)).unwrap_or(0);
        if view_length == 0 {
            return self.nb_new_buffer(&[]);
        }
        let offset_arg = raw(args, 1);
        let length_arg = raw(args, 2);
        let mut first = 0usize;
        let mut last = view_length;
        if is_present(&offset_arg) || is_present(&length_arg) {
            if is_present(&offset_arg) {
                first = self.nb_validate_integer(&offset_arg, "offset", 0.0, K_MAX_LENGTH)? as usize;
                if first >= view_length {
                    return self.nb_new_buffer(&[]);
                }
            }
            if is_present(&length_arg) {
                let length = self.nb_validate_integer(&length_arg, "length", 0.0, K_MAX_LENGTH)? as usize;
                last = (first + length).min(view_length);
            }
        }
        let bytes = zun_gc::with_current_heap(|heap| {
            let buffer = heap.typed_array_buffer(view)?;
            let base = heap.typed_array_byte_offset(view)?;
            let mut out = Vec::with_capacity((last - first) * size);
            for index in (base + first * size)..(base + last * size) {
                out.push(heap.buffer_get(buffer, index)?);
            }
            Some(out)
        })
        .ok_or(RunError::BadToken)?;
        self.nb_new_buffer(&bytes)
    }

    /// `swap16` / `swap32` / `swap64`: in place, on a multiple of the width.
    fn nb_swap(&mut self, tail: &str, view: Obj) -> Result<Value, RunError> {
        let width = match tail {
            "Swap16" => 2usize,
            "Swap32" => 4,
            _ => 8,
        };
        let len = view_len(view).unwrap_or(0);
        if len % width != 0 {
            return Err(self.nb_error(
                "RangeError",
                "ERR_INVALID_BUFFER_SIZE",
                format!("Buffer size must be a multiple of {}-bits", width * 8),
            ));
        }
        zun_gc::with_current_heap_mut(|heap| {
            let mut base = 0;
            while base < len {
                for i in 0..width / 2 {
                    let a = heap.u8_array_get(view, base + i)?;
                    let b = heap.u8_array_get(view, base + width - 1 - i)?;
                    heap.u8_array_set(view, base + i, b)?;
                    heap.u8_array_set(view, base + width - 1 - i, a)?;
                }
                base += width;
            }
            Some(())
        });
        Ok(Value::Object(view))
    }

    /// `buf.toJSON()` -> `{ type: 'Buffer', data: [..bytes] }`.
    pub(crate) fn nb_to_json(&mut self, view: Obj, _args: &[Value]) -> Result<Value, RunError> {
        let bytes = view_bytes(view).ok_or(RunError::BadToken)?;
        let object = zun_gc::with_current_heap_mut(|heap| {
            let data = heap.alloc_array(0);
            for byte in &bytes {
                heap.array_push(data, zun_core::Any::int(i64::from(*byte)));
            }
            let record = heap.alloc_dynamic();
            let kind = Value::Str("Buffer".into()).box_any(heap).unwrap_or_else(zun_core::Any::void);
            heap.dynamic_set(record, "type", kind);
            let data = Value::Object(data).box_any(heap).unwrap_or_else(zun_core::Any::void);
            heap.dynamic_set(record, "data", data);
            Value::Object(record).box_any(heap)
        });
        Ok(Value::Any(object.unwrap_or_else(zun_core::Any::void)))
    }

    fn nb_equals(&mut self, view: Obj, args: &[Value]) -> Result<Value, RunError> {
        let other = self.nb_u8_arg(&raw(args, 1), "otherBuffer")?;
        Ok(Value::Bool(view_bytes(view) == view_bytes(other)))
    }

    /// `indexOf` / `lastIndexOf` / `includes` (`bidirectionalIndexOf`).
    fn nb_search(&mut self, name: &str, view: Obj, args: &[Value]) -> Result<Value, RunError> {
        let haystack = view_bytes(view).ok_or(RunError::BadToken)?;
        let needle_value = raw(args, 1);
        let mut offset = raw(args, 2);
        let mut encoding = raw(args, 3);
        let backward = name == "LastIndexOf";
        if matches!(offset, Value::Str(_)) {
            encoding = offset.clone();
            offset = Value::Void;
        }
        let mut position = match &offset {
            Value::Void => f64::NAN,
            other => self.nb_to_number(other).unwrap_or(f64::NAN),
        };
        position = position.clamp(-2_147_483_648.0, 2_147_483_647.0);
        if position.is_nan() {
            position = if backward { haystack.len() as f64 } else { 0.0 };
        }
        let position = position.trunc() as i64;
        let needle: Vec<u8> = match &needle_value {
            Value::Float(..) | Value::Int(..) => {
                let n = as_number(&needle_value).unwrap_or(0.0);
                vec![(n as i64 as u32 & 0xff) as u8]
            }
            Value::Str(text) => {
                let enc = match &encoding {
                    Value::Void => "utf8",
                    other => match other {
                        Value::Str(t) => match strict_encoding(t) {
                            Some(n) => n,
                            None => return Err(self.nb_unknown_encoding(other)),
                        },
                        other => return Err(self.nb_unknown_encoding(other)),
                    },
                };
                nb::encode(text, enc)
            }
            other => match byte_view(other) {
                Some(view) => view_bytes(view).unwrap_or_default(),
                None => {
                    return Err(self.nb_invalid_type(
                        "value",
                        "one of type number or string or an instance of Buffer or Uint8Array",
                        other,
                    ))
                }
            },
        };
        let found = nb::buffer_search(&haystack, &needle, position, backward);
        match name {
            "Includes" => Ok(Value::Bool(found.is_some())),
            _ => Ok(Value::Float(found.map(|i| i as f64).unwrap_or(-1.0), NumKind::F64)),
        }
    }

    /// The validated (non-accessor) `nodeBuffer` intrinsics, or `None`.
    pub(crate) fn node_buffer_ops(
        &mut self,
        tail: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        // The receiver arrives as a bare object from typed code and boxed in an
        // `Any` when it came through a dynamic lane (a view built by a dynamic
        // `new Uint8Array(buffer, ...)`, as `TextDecoder.decode` does).
        let view = || match args.first() {
            Some(Value::Object(view)) => Some(*view),
            Some(value @ Value::Any(_)) => super::collections::object(value),
            _ => None,
        };
        macro_rules! on_view {
            ($method:ident) => {
                match view() {
                    Some(v) => self.$method(v, args),
                    None => Err(RunError::BadToken),
                }
            };
        }
        Some(match tail {
            "Alloc" => self.nb_alloc(args),
            "AllocUnsafe" => (|| {
                let size = self.nb_size(&raw(args, 0))?;
                self.nb_new_buffer(&vec![0u8; size])
            })(),
            "FromAny" => self.nb_from(args),
            "Swap16" | "Swap32" | "Swap64" => match view() {
                Some(v) => self.nb_swap(tail, v),
                None => Err(RunError::BadToken),
            },
            "CopyBytesFrom" => self.nb_copy_bytes_from(args),
            "Ctor" => (|| {
                let arg = raw(args, 0);
                if as_number(&arg).is_some() {
                    if matches!(raw(args, 1), Value::Str(_)) {
                        return Err(self.nb_invalid_type("string", "of type string", &arg));
                    }
                    return self.nb_alloc(&[arg]);
                }
                self.nb_from(args)
            })(),
            "Concat" => self.nb_concat(args),
            "ByteLength" => self.nb_byte_length_of(args),
            "IsEncoding" => Ok(Value::Bool(
                matches!(raw(args, 0), Value::Str(text) if !text.is_empty() && strict_encoding(&text).is_some()),
            )),
            "Compare" => (|| {
                let a = self.nb_u8_arg(&raw(args, 0), "buf1")?;
                let b = self.nb_u8_arg(&raw(args, 1), "buf2")?;
                let x = view_bytes(a).unwrap_or_default();
                let y = view_bytes(b).unwrap_or_default();
                Ok(Value::Float(Self::nb_compare_bytes(&x, &y), NumKind::F64))
            })(),
            "CompareRange" => on_view!(nb_compare_range),
            "Equals" => on_view!(nb_equals),
            "ToJSON" => on_view!(nb_to_json),
            "Fill" => on_view!(nb_fill),
            "ToString" => on_view!(nb_to_string),
            "Write" => on_view!(nb_write_string),
            "Slice" => on_view!(nb_slice),
            "Copy" => on_view!(nb_copy),
            "IndexOf" | "LastIndexOf" | "Includes" => match view() {
                Some(v) => self.nb_search(tail, v, args),
                None => Err(RunError::BadToken),
            },
            _ => return None,
        })
    }
}
