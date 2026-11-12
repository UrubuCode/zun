//! The by-name (dynamic) surface of typed arrays, `ArrayBuffer` and `DataView`.
//!
//! Statically typed code lowers `view.length`, `view[i]` and a handful of
//! methods to intrinsics. A view reached through `any` -- or used through a
//! method the typed surface does not model -- arrives here instead: property
//! reads, method calls, and `new ctor(...)` on a builtin constructor value.
//! The element kind is read from the view at run time, so one handler per
//! operation serves every kind.

use super::collections::unbox_arg;
use super::value::Value;
use super::{Interp, RunError};
use zun_core::TypedElement;
use zun_il::NumKind;

fn obj_of(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Object(object) => Some(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
        _ => None,
    }
}

fn num(value: f64) -> Value {
    Value::Float(value, NumKind::F64)
}

fn is_bigint_kind(kind: TypedElement) -> bool {
    matches!(kind.js_name(), "BigInt64Array" | "BigUint64Array")
}

/// ECMAScript ToIntegerOrInfinity over an already-converted number.
fn integer_of(n: f64) -> f64 {
    if n.is_nan() {
        0.0
    } else {
        n.trunc()
    }
}

/// Wraps `n` to a `bits`-wide integer (ToInt8/16/32 and the unsigned forms),
/// returned as an `i64` carrying the wrapped bit pattern.
fn wrap_bits(n: f64, bits: u32) -> i64 {
    if !n.is_finite() {
        return 0;
    }
    let modulus = (1u128 << bits) as f64;
    let wrapped = n.trunc().rem_euclid(modulus);
    wrapped as i64
}

impl Interp<'_> {
    pub(crate) fn view_error(&mut self, class: &str, message: &str) -> RunError {
        match self.new_error_object(class, message) {
            Some(object) => RunError::Thrown(Value::Object(object)),
            None => RunError::Native(format!("{class}: {message}")),
        }
    }

    /// ECMAScript ToIndex: undefined is 0, a negative or out-of-range integer
    /// is a RangeError.
    fn view_to_index(&mut self, value: Option<&Value>, what: &str) -> Result<usize, RunError> {
        let value = match value {
            None | Some(Value::Void) => return Ok(0),
            Some(other) => unbox_arg(other)?,
        };
        if matches!(&value, Value::Object(o) if zun_gc::with_current_heap(|h| h.bigint_value(*o).is_some()))
        {
            return Err(self.view_error("TypeError", "Cannot convert a BigInt value to a number"));
        }
        let n = integer_of(self.dyn_to_number(&value)?);
        if n < 0.0 || n > 9007199254740991.0 {
            return Err(self.view_error("RangeError", &format!("Invalid {what}")));
        }
        Ok(n as usize)
    }

    /// A relative index argument clamped into `0..=len` (`undefined` takes
    /// `default`).
    fn view_relative(
        &mut self,
        value: Option<&Value>,
        len: usize,
        default: usize,
    ) -> Result<usize, RunError> {
        let value = match value {
            None | Some(Value::Void) => return Ok(default),
            Some(other) => unbox_arg(other)?,
        };
        let n = integer_of(self.dyn_to_number(&value)?);
        Ok(if n < 0.0 {
            (len as f64 + n).max(0.0) as usize
        } else {
            n.min(len as f64) as usize
        })
    }

    /// One element of `view` as the JavaScript value it denotes.
    fn view_read(&mut self, view: zun_gc::Obj, index: usize) -> Value {
        let kind = crate::typed_arrays::element_of(view);
        match crate::typed_arrays::read_element(view, index) {
            Some(Value::Int(n, k)) => match kind {
                Some(kind) if is_bigint_kind(kind) => {
                    let big = if matches!(k, NumKind::U64) {
                        zun_core::BigInt::from_u64(n as u64)
                    } else {
                        zun_core::BigInt::from_i64(n)
                    };
                    Value::Object(zun_gc::with_current_heap_mut(|h| h.alloc_bigint(big)))
                }
                _ => num(n as f64),
            },
            Some(Value::Float(f, _)) => num(f),
            Some(other) => other,
            None => Value::Void,
        }
    }

    /// Stores a JavaScript value into `view[index]`, converting it to the
    /// element type. An out-of-range index is ignored (as JS does).
    fn view_write(
        &mut self,
        view: zun_gc::Obj,
        index: usize,
        value: &Value,
    ) -> Result<(), RunError> {
        let Some(kind) = crate::typed_arrays::element_of(view) else {
            return Ok(());
        };
        let value = unbox_arg(value)?;
        let converted = if is_bigint_kind(kind) {
            // Zun has native 64-bit integers, so a plain int or float writes
            // directly into a BigInt64/BigUint64 view; a BigInt object and
            // bool also convert, matching JS.
            if let Value::Int(bits, _) = value {
                Value::Int(bits, NumKind::I64)
            } else if let Value::Float(n, _) = value {
                Value::Int(n as i64, NumKind::I64)
            } else {
                let big = match &value {
                    Value::Object(o) => zun_gc::with_current_heap(|h| h.bigint_value(*o).cloned()),
                    Value::Bool(b) => Some(zun_core::BigInt::from_u64(u64::from(*b))),
                    _ => None,
                };
                let Some(big) = big else {
                    return Err(self.view_error(
                        "TypeError",
                        "Cannot convert a value to a BigInt",
                    ));
                };
                let bits = big.as_uint_n(64).to_u64().unwrap_or(0);
                Value::Int(bits as i64, NumKind::I64)
            }
        } else {
            if matches!(&value, Value::Object(o) if zun_gc::with_current_heap(|h| h.bigint_value(*o).is_some()))
            {
                return Err(
                    self.view_error("TypeError", "Cannot convert a BigInt value to a number")
                );
            }
            let n = self.dyn_to_number(&value)?;
            match kind.js_name() {
                "Int8Array" | "Uint8Array" => Value::Int(wrap_bits(n, 8), NumKind::I64),
                "Int16Array" | "Uint16Array" => Value::Int(wrap_bits(n, 16), NumKind::I64),
                "Int32Array" | "Uint32Array" => Value::Int(wrap_bits(n, 32), NumKind::I64),
                "Uint8ClampedArray" => {
                    let clamped = if n.is_nan() {
                        0.0
                    } else {
                        let c = n.clamp(0.0, 255.0);
                        let f = c.floor();
                        let diff = c - f;
                        if diff < 0.5 {
                            f
                        } else if diff > 0.5 {
                            f + 1.0
                        } else if f % 2.0 == 0.0 {
                            f
                        } else {
                            f + 1.0
                        }
                    };
                    Value::Float(clamped, NumKind::F64)
                }
                _ => Value::Float(n, NumKind::F64),
            }
        };
        crate::typed_arrays::write_element(view, index, &converted);
        Ok(())
    }

    fn view_collect(&mut self, view: zun_gc::Obj) -> Vec<Value> {
        let len = crate::typed_arrays::element_len(view).unwrap_or(0);
        (0..len).map(|i| self.view_read(view, i)).collect()
    }

    fn view_new_like(
        &mut self,
        kind: TypedElement,
        values: &[Value],
    ) -> Result<zun_gc::Obj, RunError> {
        let view = crate::typed_arrays::alloc_of(kind, values.len())
            .ok_or_else(|| self.view_error("RangeError", "Invalid typed array length"))?;
        for (index, value) in values.iter().enumerate() {
            self.view_write(view, index, value)?;
        }
        Ok(view)
    }

    fn view_js_string(&mut self, value: &Value) -> Result<String, RunError> {
        self.dyn_to_string(value)
    }

    /// The name of the builtin buffer constructor a class-descriptor token
    /// denotes.
    fn builtin_buffer_ctor_name(&mut self, token: u32) -> Option<&'static str> {
        let mut names: Vec<&'static str> = TypedElement::ALL.iter().map(|k| k.js_name()).collect();
        names.push("ArrayBuffer");
        names.push("SharedArrayBuffer");
        names.push("DataView");
        zun_gc::with_current_heap_mut(|heap| {
            let global = heap.global_object();
            names.into_iter().find(|name| {
                heap.dynamic_get(global, name)
                    .and_then(|a| a.as_class_descriptor())
                    == Some(token)
            })
        })
    }

    /// The element kind of a typed-array constructor descriptor token.
    pub(crate) fn typed_ctor_kind(&mut self, token: u32) -> Option<TypedElement> {
        let name = self.builtin_buffer_ctor_name(token)?;
        TypedElement::ALL.iter().copied().find(|k| k.js_name() == name)
    }

    /// `new <ArrayBuffer | DataView | TypedArray>(...)` through a constructor
    /// value. `None` when the token is not one of those.
    pub(crate) fn construct_builtin_buffer(
        &mut self,
        token: u32,
        values: &[Value],
    ) -> Option<Result<Value, RunError>> {
        let name = self.builtin_buffer_ctor_name(token)?;
        Some(self.construct_buffer_named(name, values))
    }

    fn construct_buffer_named(&mut self, name: &str, values: &[Value]) -> Result<Value, RunError> {
        let args: Vec<Value> = values.iter().map(unbox_arg).collect::<Result<_, _>>()?;
        match name {
            "ArrayBuffer" | "SharedArrayBuffer" => {
                let length = self.view_to_index(args.first(), "array buffer length")?;
                let max = match args.get(1).and_then(obj_of) {
                    Some(options) => {
                        let any = zun_gc::with_current_heap(|h| h.dynamic_get(options, "maxByteLength"));
                        match any {
                            Some(any) => {
                                let value = zun_gc::with_current_heap(|h| Value::unbox_any(any, h))
                                    .unwrap_or(Value::Void);
                                if matches!(value, Value::Void) {
                                    None
                                } else {
                                    Some(self.view_to_index(Some(&value), "array buffer max length")?)
                                }
                            }
                            None => None,
                        }
                    }
                    None => None,
                };
                let buffer = match max {
                    Some(max) => {
                        if length > max {
                            return Err(self.view_error(
                                "RangeError",
                                "Invalid array buffer max length",
                            ));
                        }
                        zun_gc::with_current_heap_mut(|h| h.alloc_resizable_buffer(length, max))
                    }
                    None => {
                        if length > (1usize << 32) {
                            return Err(self.view_error("RangeError", "Array buffer allocation failed"));
                        }
                        Some(zun_gc::with_current_heap_mut(|h| h.alloc_buffer(length)))
                    }
                };
                let buffer = buffer
                    .ok_or_else(|| self.view_error("RangeError", "Invalid array buffer length"))?;
                Ok(Value::Object(buffer))
            }
            "DataView" => {
                let buffer = args
                    .first()
                    .and_then(obj_of)
                    .filter(|b| zun_gc::with_current_heap(|h| h.buffer_len(*b).is_some()));
                let Some(buffer) = buffer else {
                    return Err(self.view_error(
                        "TypeError",
                        "First argument to DataView constructor must be an ArrayBuffer",
                    ));
                };
                let offset = self.view_to_index(args.get(1), "DataView offset")?;
                let blen = zun_gc::with_current_heap(|h| h.buffer_len(buffer)).unwrap_or(0);
                if offset > blen {
                    return Err(self.view_error("RangeError", "Start offset is outside the bounds of the buffer"));
                }
                let view = match args.get(2) {
                    None | Some(Value::Void) => {
                        zun_gc::with_current_heap_mut(|h| h.alloc_data_view_tracking(buffer, offset))
                    }
                    Some(length) => {
                        let length = self.view_to_index(Some(length), "DataView length")?;
                        zun_gc::with_current_heap_mut(|h| h.alloc_data_view(buffer, offset, length))
                    }
                };
                let view = view.ok_or_else(|| {
                    self.view_error("RangeError", "Invalid DataView length")
                })?;
                Ok(Value::Object(view))
            }
            _ => {
                let kind = TypedElement::ALL
                    .iter()
                    .copied()
                    .find(|k| k.js_name() == name)
                    .ok_or(RunError::BadToken)?;
                self.construct_typed_named(kind, &args)
            }
        }
    }

    fn construct_typed_named(
        &mut self,
        kind: TypedElement,
        args: &[Value],
    ) -> Result<Value, RunError> {
        let size = kind.size();
        let first = args.first().cloned().unwrap_or(Value::Void);
        let Some(source) = obj_of(&first) else {
            // A length (ToIndex of any primitive).
            let length = self.view_to_index(Some(&first), "typed array length")?;
            if length.saturating_mul(size) > (1usize << 32) {
                return Err(self.view_error("RangeError", "Invalid typed array length"));
            }
            let view = crate::typed_arrays::alloc_of(kind, length)
                .ok_or_else(|| self.view_error("RangeError", "Invalid typed array length"))?;
            return Ok(Value::Object(view));
        };
        let is_buffer = zun_gc::with_current_heap(|h| h.buffer_len(source).is_some());
        if is_buffer {
            let blen = zun_gc::with_current_heap(|h| h.buffer_len(source)).unwrap_or(0);
            let offset = self.view_to_index(args.get(1), "offset")?;
            if offset % size != 0 {
                return Err(self.view_error(
                    "RangeError",
                    &format!("start offset of {} should be a multiple of {size}", kind.js_name()),
                ));
            }
            let view = match args.get(2) {
                None | Some(Value::Void) => {
                    let resizable = zun_gc::with_current_heap(|h| h.buffer_resizable(source))
                        .unwrap_or(false);
                    if offset > blen {
                        return Err(self.view_error("RangeError", &format!("Start offset {offset} is outside the bounds of the buffer")));
                    }
                    if resizable {
                        zun_gc::with_current_heap_mut(|h| {
                            h.alloc_typed_array_tracking(source, offset, kind)
                        })
                    } else {
                        if (blen - offset) % size != 0 {
                            return Err(self.view_error(
                                "RangeError",
                                &format!("byte length of {} should be a multiple of {size}", kind.js_name()),
                            ));
                        }
                        let length = (blen - offset) / size;
                        zun_gc::with_current_heap_mut(|h| {
                            h.alloc_typed_array(source, offset, length, kind)
                        })
                    }
                }
                Some(length) => {
                    let length = self.view_to_index(Some(length), "typed array length")?;
                    if offset.saturating_add(length.saturating_mul(size)) > blen {
                        return Err(self.view_error(
                            "RangeError",
                            &format!("Invalid typed array length: {length}"),
                        ));
                    }
                    zun_gc::with_current_heap_mut(|h| {
                        h.alloc_typed_array(source, offset, length, kind)
                    })
                }
            };
            let view = view
                .ok_or_else(|| self.view_error("RangeError", "Invalid typed array length"))?;
            return Ok(Value::Object(view));
        }
        // A typed array source: element-wise copy (BigInt kinds do not mix).
        if let Some(src_kind) = crate::typed_arrays::element_of(source) {
            if is_bigint_kind(src_kind) != is_bigint_kind(kind) {
                return Err(self.view_error(
                    "TypeError",
                    "Cannot mix BigInt and other types, use explicit conversions",
                ));
            }
            let values = self.view_collect(source);
            let view = self.view_new_like(kind, &values)?;
            return Ok(Value::Object(view));
        }
        // An iterable or array-like object.
        let drained = self.call_managed_collection("arrayFrom", &[Value::Object(source)])?;
        let array = obj_of(&drained).ok_or(RunError::BadToken)?;
        let items = self.read_array_values(array)?;
        let view = self.view_new_like(kind, &items)?;
        Ok(Value::Object(view))
    }

    /// Property read on a typed array, `ArrayBuffer` or `DataView` object.
    pub(crate) fn typed_view_get(&mut self, object: zun_gc::Obj, name: &str) -> Option<Value> {
        zun_gc::with_current_heap(|heap| {
            if let Some(kind) = heap.typed_array_element(object) {
                return match name {
                    "length" => heap.typed_array_len(object).map(|n| num(n as f64)),
                    "byteLength" => heap.typed_array_byte_len(object).map(|n| num(n as f64)),
                    "byteOffset" => heap.typed_array_byte_offset(object).map(|n| num(n as f64)),
                    "buffer" => heap.typed_array_buffer(object).map(Value::Object),
                    "BYTES_PER_ELEMENT" => Some(num(kind.size() as f64)),
                    _ => None,
                };
            }
            if let Some(length) = heap.buffer_len(object) {
                return match name {
                    "byteLength" => Some(num(length as f64)),
                    "maxByteLength" => Some(num(heap.buffer_max_byte_length(object).unwrap_or(length) as f64)),
                    "resizable" => Some(Value::Bool(heap.buffer_resizable(object).unwrap_or(false))),
                    "detached" => Some(Value::Bool(heap.buffer_detached(object).unwrap_or(false))),
                    _ => None,
                };
            }
            if heap.data_view_buffer(object).is_some() {
                return match name {
                    "byteLength" => heap.data_view_byte_len(object).map(|n| num(n as f64)),
                    "byteOffset" => heap.data_view_byte_offset(object).map(|n| num(n as f64)),
                    "buffer" => heap.data_view_buffer(object).map(Value::Object),
                    _ => None,
                };
            }
            None
        })
    }

    /// Method call on a typed array, `ArrayBuffer` or `DataView` object.
    /// `Ok(None)` when `object` is none of those or the member is unknown.
    pub(crate) fn typed_view_call(
        &mut self,
        object: zun_gc::Obj,
        member: &str,
        args: &[Value],
    ) -> Result<Option<Value>, RunError> {
        let args: Vec<Value> = args.iter().map(unbox_arg).collect::<Result<_, _>>()?;
        if crate::typed_arrays::element_of(object).is_some() {
            return self.typed_array_call(object, member, &args);
        }
        if zun_gc::with_current_heap(|h| h.buffer_len(object).is_some()) {
            return self.array_buffer_call(object, member, &args);
        }
        if zun_gc::with_current_heap(|h| h.data_view_buffer(object).is_some()) {
            return self.data_view_call(object, member, &args);
        }
        Ok(None)
    }

    fn array_buffer_call(
        &mut self,
        buffer: zun_gc::Obj,
        member: &str,
        args: &[Value],
    ) -> Result<Option<Value>, RunError> {
        let detached = zun_gc::with_current_heap(|h| h.buffer_detached(buffer)).unwrap_or(false);
        let len = zun_gc::with_current_heap(|h| h.buffer_len(buffer)).unwrap_or(0);
        match member {
            // Zun-specific no-argument accessor method.
            "byteLength" => return Ok(Some(num(len as f64))),
            "slice" => {
                if detached {
                    return Err(self.view_error("TypeError", "Cannot perform ArrayBuffer.prototype.slice on a detached ArrayBuffer"));
                }
                let begin = self.view_relative(args.first(), len, 0)?;
                let end = self.view_relative(args.get(1), len, len)?;
                let end = end.max(begin);
                let copy = zun_gc::with_current_heap_mut(|h| h.buffer_slice(buffer, begin, end));
                Ok(Some(Value::Object(copy.ok_or_else(|| {
                    self.view_error("RangeError", "Invalid array buffer slice")
                })?)))
            }
            "resize" => {
                let resizable = zun_gc::with_current_heap(|h| h.buffer_resizable(buffer)).unwrap_or(false);
                if !resizable || detached {
                    return Err(self.view_error("TypeError", "Method ArrayBuffer.prototype.resize called on incompatible receiver"));
                }
                let new_len = self.view_to_index(args.first(), "length")?;
                let ok = zun_gc::with_current_heap_mut(|h| h.buffer_resize(buffer, new_len));
                if ok != Some(true) {
                    return Err(self.view_error("RangeError", "ArrayBuffer.prototype.resize: Invalid length parameter"));
                }
                Ok(Some(Value::Void))
            }
            "transfer" | "transferToFixedLength" => {
                if detached {
                    return Err(self.view_error("TypeError", "Cannot perform ArrayBuffer.prototype.transfer on a detached ArrayBuffer"));
                }
                let new_len = match args.first() {
                    None | Some(Value::Void) => None,
                    other => Some(self.view_to_index(other, "length")?),
                };
                let fixed = member == "transferToFixedLength";
                let moved = zun_gc::with_current_heap_mut(|h| h.buffer_transfer(buffer, new_len, fixed));
                match moved {
                    Some(o) => Ok(Some(Value::Object(o))),
                    None => Err(self.view_error("RangeError", "Invalid array buffer length")),
                }
            }
            _ => Ok(None),
        }
    }

    fn data_view_call(
        &mut self,
        view: zun_gc::Obj,
        member: &str,
        args: &[Value],
    ) -> Result<Option<Value>, RunError> {
        // Zun-specific no-argument accessor methods (`v.byteLength()` etc.),
        // distinct from the JS property reads handled in `typed_view_get`.
        match member {
            "byteLength" => {
                let n = zun_gc::with_current_heap(|h| h.data_view_byte_len(view)).unwrap_or(0);
                return Ok(Some(num(n as f64)));
            }
            "byteOffset" => {
                let n = zun_gc::with_current_heap(|h| h.data_view_byte_offset(view)).unwrap_or(0);
                return Ok(Some(num(n as f64)));
            }
            "buffer" => {
                let b = zun_gc::with_current_heap(|h| h.data_view_buffer(view));
                return Ok(b.map(Value::Object));
            }
            _ => {}
        }
        let (is_set, kind) = if let Some(rest) = member.strip_prefix("get") {
            (false, rest)
        } else if let Some(rest) = member.strip_prefix("set") {
            (true, rest)
        } else {
            return Ok(None);
        };
        // `signed_name` ending in `n` marks Zun's native 64-bit integer forms
        // (`getInt64`/`setUint64`), which read and write plain ints rather than
        // the JS `BigInt64`/`BigUint64` that hand back a BigInt object.
        let (width, signed_name): (usize, &str) = match kind {
            "Int8" => (1, "i8"),
            "Uint8" => (1, "u8"),
            "Int16" => (2, "i16"),
            "Uint16" => (2, "u16"),
            "Int32" => (4, "i32"),
            "Uint32" => (4, "u32"),
            "Float32" => (4, "f32"),
            "Float64" => (8, "f64"),
            "BigInt64" => (8, "i64"),
            "BigUint64" => (8, "u64"),
            "Int64" => (8, "i64n"),
            "Uint64" => (8, "u64n"),
            _ => return Ok(None),
        };
        let offset = self.view_to_index(args.first(), "offset")?;
        let little = if is_set {
            args.get(2)
        } else {
            args.get(1)
        }
        .map(|v| zun_gc::with_current_heap(|h| crate::collections::is_truthy(v, h)))
        .unwrap_or(false);
        // Convert the value before the bounds check (JS order).
        let write_bits: Option<u64> = if is_set {
            let value = args.get(1).cloned().unwrap_or(Value::Void);
            if signed_name == "i64" || signed_name == "u64" {
                let big = match &value {
                    Value::Object(o) => zun_gc::with_current_heap(|h| h.bigint_value(*o).cloned()),
                    _ => None,
                };
                let Some(big) = big else {
                    return Err(self.view_error("TypeError", "Cannot convert a value to a BigInt"));
                };
                Some(big.as_uint_n(64).to_u64().unwrap_or(0))
            } else {
                let n = self.dyn_to_number(&value)?;
                Some(match signed_name {
                    "f32" => u64::from((n as f32).to_bits()),
                    "f64" => n.to_bits(),
                    "i8" | "u8" => wrap_bits(n, 8) as u64,
                    "i16" | "u16" => wrap_bits(n, 16) as u64,
                    "i64n" | "u64n" => n as i64 as u64,
                    _ => wrap_bits(n, 32) as u64,
                })
            }
        } else {
            None
        };
        let view_len = zun_gc::with_current_heap(|h| h.data_view_byte_len(view)).unwrap_or(0);
        if offset.checked_add(width).is_none_or(|end| end > view_len) {
            return Err(self.view_error("RangeError", "Offset is outside the bounds of the DataView"));
        }
        let base = zun_gc::with_current_heap(|h| h.data_view_byte_offset(view)).unwrap_or(0);
        let buffer = zun_gc::with_current_heap(|h| h.data_view_buffer(view)).ok_or(RunError::BadToken)?;
        let at = base + offset;
        if let Some(bits) = write_bits {
            zun_gc::with_current_heap_mut(|h| {
                for i in 0..width {
                    let shift = if little { i } else { width - 1 - i } * 8;
                    h.buffer_set(buffer, at + i, ((bits >> shift) & 0xff) as u8);
                }
            });
            return Ok(Some(Value::Void));
        }
        let mut bits: u64 = 0;
        zun_gc::with_current_heap(|h| {
            for i in 0..width {
                let byte = u64::from(h.buffer_get(buffer, at + i).unwrap_or(0));
                let shift = if little { i } else { width - 1 - i } * 8;
                bits |= byte << shift;
            }
        });
        let value = match signed_name {
            "i8" => num(f64::from(bits as u8 as i8)),
            "u8" => num(f64::from(bits as u8)),
            "i16" => num(f64::from(bits as u16 as i16)),
            "u16" => num(f64::from(bits as u16)),
            "i32" => num(f64::from(bits as u32 as i32)),
            "u32" => num(f64::from(bits as u32)),
            "f32" => num(f64::from(f32::from_bits(bits as u32))),
            "f64" => num(f64::from_bits(bits)),
            "i64n" => Value::Int(bits as i64, zun_il::NumKind::I64),
            "u64n" => Value::Int(bits as i64, zun_il::NumKind::U64),
            "i64" => Value::Object(zun_gc::with_current_heap_mut(|h| {
                h.alloc_bigint(zun_core::BigInt::from_i64(bits as i64))
            })),
            _ => Value::Object(zun_gc::with_current_heap_mut(|h| {
                h.alloc_bigint(zun_core::BigInt::from_u64(bits))
            })),
        };
        Ok(Some(value))
    }

    fn typed_array_call(
        &mut self,
        view: zun_gc::Obj,
        member: &str,
        args: &[Value],
    ) -> Result<Option<Value>, RunError> {
        let kind = crate::typed_arrays::element_of(view).ok_or(RunError::BadToken)?;
        let len = crate::typed_arrays::element_len(view).unwrap_or(0);
        let truthy = |v: &Value| zun_gc::with_current_heap(|h| crate::collections::is_truthy(v, h));
        let callback = |args: &[Value]| match args.first() {
            Some(Value::Function(handle)) => Some(*handle),
            _ => None,
        };
        let not_callable = |this: &mut Self| {
            this.view_error("TypeError", "callback is not a function")
        };
        match member {
            "at" => {
                let n = match args.first() {
                    Some(v) => integer_of(self.dyn_to_number(v)?),
                    None => 0.0,
                };
                let index = if n < 0.0 { len as f64 + n } else { n };
                if index < 0.0 || index >= len as f64 {
                    return Ok(Some(Value::Void));
                }
                Ok(Some(self.view_read(view, index as usize)))
            }
            "indexOf" | "lastIndexOf" | "includes" => {
                let needle = args.first().cloned().unwrap_or(Value::Void);
                let items = self.view_collect(view);
                let same = |a: &Value, b: &Value, zero: bool| -> bool {
                    zun_gc::with_current_heap(|h| match (a, b) {
                        (Value::Float(x, _), Value::Float(y, _)) => {
                            x == y || (zero && x.is_nan() && y.is_nan())
                        }
                        (Value::Object(x), Value::Object(y)) => {
                            match (h.bigint_value(*x), h.bigint_value(*y)) {
                                (Some(p), Some(q)) => p.cmp(q) == std::cmp::Ordering::Equal,
                                _ => false,
                            }
                        }
                        _ => false,
                    })
                };
                let includes = member == "includes";
                if member == "lastIndexOf" {
                    let from = match args.get(1) {
                        Some(v) => integer_of(self.dyn_to_number(v)?),
                        None => len as f64 - 1.0,
                    };
                    let start = if from < 0.0 { len as f64 + from } else { from.min(len as f64 - 1.0) };
                    let mut i = start;
                    while i >= 0.0 {
                        if same(&items[i as usize], &needle, false) {
                            return Ok(Some(num(i)));
                        }
                        i -= 1.0;
                    }
                    return Ok(Some(num(-1.0)));
                }
                let start = self.view_relative(args.get(1), len, 0)?;
                for (i, item) in items.iter().enumerate().skip(start) {
                    if same(item, &needle, includes) {
                        return Ok(Some(if includes { Value::Bool(true) } else { num(i as f64) }));
                    }
                }
                Ok(Some(if includes { Value::Bool(false) } else { num(-1.0) }))
            }
            "join" | "toString" | "toLocaleString" => {
                let sep = match args.first() {
                    None | Some(Value::Void) => ",".to_string(),
                    Some(v) if member == "join" => self.view_js_string(v)?,
                    _ => ",".to_string(),
                };
                let items = self.view_collect(view);
                let mut parts = Vec::with_capacity(items.len());
                for item in &items {
                    parts.push(self.view_js_string(item)?);
                }
                Ok(Some(Value::Str(parts.join(&sep).into())))
            }
            "reverse" => {
                let mut items = self.view_collect(view);
                items.reverse();
                for (i, v) in items.iter().enumerate() {
                    self.view_write(view, i, v)?;
                }
                Ok(Some(Value::Object(view)))
            }
            "toReversed" => {
                let mut items = self.view_collect(view);
                items.reverse();
                Ok(Some(Value::Object(self.view_new_like(kind, &items)?)))
            }
            "fill" => {
                let value = args.first().cloned().unwrap_or(Value::Void);
                let begin = self.view_relative(args.get(1), len, 0)?;
                let end = self.view_relative(args.get(2), len, len)?;
                // Convert once (it may throw), then store across the range.
                for i in begin..end {
                    self.view_write(view, i, &value)?;
                }
                Ok(Some(Value::Object(view)))
            }
            "set" => {
                let source = args.first().cloned().unwrap_or(Value::Void);
                let offset_n = match args.get(1) {
                    Some(v) => integer_of(self.dyn_to_number(v)?),
                    None => 0.0,
                };
                if offset_n < 0.0 {
                    return Err(self.view_error("RangeError", "offset is out of bounds"));
                }
                let offset = offset_n as usize;
                let items: Vec<Value> = match obj_of(&source) {
                    Some(src) if crate::typed_arrays::element_len(src).is_some() => {
                        self.view_collect(src)
                    }
                    Some(src) => {
                        let array = match zun_gc::with_current_heap(|h| h.array_len(src)) {
                            Some(_) => src,
                            None => {
                                let drained = self.call_managed_collection(
                                    "arrayFromArrayLike",
                                    &[Value::Object(src)],
                                )?;
                                obj_of(&drained).ok_or(RunError::BadToken)?
                            }
                        };
                        self.read_array_values(array)?
                    }
                    None => Vec::new(),
                };
                if offset.checked_add(items.len()).is_none_or(|end| end > len) {
                    return Err(self.view_error("RangeError", "offset is out of bounds"));
                }
                for (i, v) in items.iter().enumerate() {
                    self.view_write(view, offset + i, v)?;
                }
                Ok(Some(Value::Void))
            }
            "subarray" => {
                let begin = self.view_relative(args.first(), len, 0)?;
                let end = self.view_relative(args.get(1), len, len)?;
                let end = end.max(begin);
                let child = zun_gc::with_current_heap_mut(|h| h.typed_array_subarray(view, begin, end));
                Ok(Some(Value::Object(child.ok_or_else(|| {
                    self.view_error("RangeError", "Invalid typed array subarray")
                })?)))
            }
            "slice" => {
                let begin = self.view_relative(args.first(), len, 0)?;
                let end = self.view_relative(args.get(1), len, len)?;
                let items: Vec<Value> = self
                    .view_collect(view)
                    .into_iter()
                    .skip(begin)
                    .take(end.saturating_sub(begin))
                    .collect();
                Ok(Some(Value::Object(self.view_new_like(kind, &items)?)))
            }
            "copyWithin" => {
                let target = self.view_relative(args.first(), len, 0)?;
                let start = self.view_relative(args.get(1), len, 0)?;
                let end = self.view_relative(args.get(2), len, len)?;
                let count = end.saturating_sub(start).min(len - target);
                let items = self.view_collect(view);
                for i in 0..count {
                    self.view_write(view, target + i, &items[start + i])?;
                }
                Ok(Some(Value::Object(view)))
            }
            "with" => {
                let n = match args.first() {
                    Some(v) => integer_of(self.dyn_to_number(v)?),
                    None => 0.0,
                };
                let index = if n < 0.0 { len as f64 + n } else { n };
                if index < 0.0 || index >= len as f64 {
                    return Err(self.view_error("RangeError", "Invalid typed array index"));
                }
                let mut items = self.view_collect(view);
                let copy = self.view_new_like(kind, &items)?;
                self.view_write(copy, index as usize, &args.get(1).cloned().unwrap_or(Value::Void))?;
                items.clear();
                Ok(Some(Value::Object(copy)))
            }
            "sort" | "toSorted" => {
                let cmp = match args.first() {
                    None | Some(Value::Void) => None,
                    Some(Value::Function(h)) => Some(*h),
                    Some(_) => {
                        return Err(self.view_error(
                            "TypeError",
                            "The comparison function must be either a function or undefined",
                        ))
                    }
                };
                let mut items = self.view_collect(view);
                let target = if member == "sort" {
                    view
                } else {
                    self.view_new_like(kind, &items)?
                };
                let key = |v: &Value| -> f64 {
                    match v {
                        Value::Float(f, _) => *f,
                        _ => 0.0,
                    }
                };
                let big_key = |v: &Value| -> Option<zun_core::BigInt> {
                    match v {
                        Value::Object(o) => zun_gc::with_current_heap(|h| h.bigint_value(*o).cloned()),
                        _ => None,
                    }
                };
                if let Some(handle) = cmp {
                    let roots = [view, target];
                    let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                    // Insertion-based merge via a simple stable sort with a
                    // fallible comparator.
                    let mut failure: Option<RunError> = None;
                    let mut sorted: Vec<Value> = Vec::with_capacity(items.len());
                    for item in items.drain(..) {
                        let mut lo = 0usize;
                        let mut hi = sorted.len();
                        while lo < hi && failure.is_none() {
                            let mid = (lo + hi) / 2;
                            let result = self.invoke_callback(
                                handle,
                                vec![
                                    self.box_view_value(&sorted[mid]),
                                    self.box_view_value(&item),
                                ],
                            );
                            match result {
                                Ok(v) => {
                                    let n = self.dyn_to_number(&v)?;
                                    if n > 0.0 {
                                        hi = mid;
                                    } else {
                                        lo = mid + 1;
                                    }
                                }
                                Err(e) => failure = Some(e),
                            }
                        }
                        if let Some(e) = failure {
                            return Err(e);
                        }
                        sorted.insert(lo, item);
                    }
                    items = sorted;
                } else if is_bigint_kind(kind) {
                    items.sort_by(|a, b| match (big_key(a), big_key(b)) {
                        (Some(x), Some(y)) => x.cmp(&y),
                        _ => std::cmp::Ordering::Equal,
                    });
                } else {
                    items.sort_by(|a, b| {
                        let (x, y) = (key(a), key(b));
                        if x.is_nan() && y.is_nan() {
                            std::cmp::Ordering::Equal
                        } else if x.is_nan() {
                            std::cmp::Ordering::Greater
                        } else if y.is_nan() {
                            std::cmp::Ordering::Less
                        } else if x == 0.0 && y == 0.0 {
                            y.is_sign_negative().cmp(&x.is_sign_negative())
                        } else {
                            x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
                        }
                    });
                }
                for (i, v) in items.iter().enumerate() {
                    self.view_write(target, i, v)?;
                }
                Ok(Some(Value::Object(target)))
            }
            "forEach" | "map" | "filter" | "some" | "every" | "find" | "findIndex"
            | "findLast" | "findLastIndex" => {
                let Some(handle) = callback(args) else {
                    return Err(not_callable(self));
                };
                let this_arg = args.get(1).cloned();
                let _ = this_arg;
                let result = if member == "map" {
                    crate::typed_arrays::alloc_of(kind, len)
                } else {
                    Some(view)
                };
                let result = result.ok_or(RunError::BadToken)?;
                let roots = [view, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let reverse = matches!(member, "findLast" | "findLastIndex");
                let mut kept: Vec<Value> = Vec::new();
                for step in 0..len {
                    let index = if reverse { len - 1 - step } else { step };
                    let value = self.view_read(roots[0], index);
                    let produced = self.invoke_callback(
                        handle,
                        vec![
                            self.box_view_value(&value),
                            Value::Any(zun_core::Any::float(index as f64)),
                            Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                        ],
                    )?;
                    match member {
                        "forEach" => {}
                        "map" => self.view_write(roots[1], index, &produced)?,
                        "filter" => {
                            if truthy(&produced) {
                                kept.push(value);
                            }
                        }
                        "some" => {
                            if truthy(&produced) {
                                return Ok(Some(Value::Bool(true)));
                            }
                        }
                        "every" => {
                            if !truthy(&produced) {
                                return Ok(Some(Value::Bool(false)));
                            }
                        }
                        "find" | "findLast" => {
                            if truthy(&produced) {
                                return Ok(Some(value));
                            }
                        }
                        _ => {
                            if truthy(&produced) {
                                return Ok(Some(num(index as f64)));
                            }
                        }
                    }
                }
                Ok(Some(match member {
                    "forEach" | "find" | "findLast" => Value::Void,
                    "map" => Value::Object(roots[1]),
                    "filter" => Value::Object(self.view_new_like(kind, &kept)?),
                    "some" => Value::Bool(false),
                    "every" => Value::Bool(true),
                    _ => num(-1.0),
                }))
            }
            "reduce" | "reduceRight" => {
                let Some(handle) = callback(args) else {
                    return Err(not_callable(self));
                };
                let right = member == "reduceRight";
                let items = self.view_collect(view);
                let mut order: Vec<usize> = (0..len).collect();
                if right {
                    order.reverse();
                }
                let mut iter = order.into_iter();
                let mut acc = match args.get(1) {
                    Some(init) => init.clone(),
                    None => match iter.next() {
                        Some(i) => items[i].clone(),
                        None => {
                            return Err(self.view_error(
                                "TypeError",
                                "Reduce of empty array with no initial value",
                            ))
                        }
                    },
                };
                let roots = [view];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for i in iter {
                    acc = self.invoke_callback(
                        handle,
                        vec![
                            self.box_view_value(&acc),
                            self.box_view_value(&items[i]),
                            Value::Any(zun_core::Any::float(i as f64)),
                            Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                        ],
                    )?;
                }
                Ok(Some(acc))
            }
            "keys" | "values" | "entries" => {
                let items = self.view_collect(view);
                let mode: u8 = match member {
                    "values" => 0,
                    "keys" => 1,
                    _ => 2,
                };
                let iterator = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(items.len());
                    for (i, v) in items.iter().enumerate() {
                        if let Some(boxed) = v.box_any(heap) {
                            heap.array_set(array, i, boxed);
                        }
                    }
                    heap.alloc_array_iterator_mode(array, mode)
                });
                Ok(iterator.map(Value::Object))
            }
            _ => Ok(None),
        }
    }

    fn box_view_value(&self, value: &Value) -> Value {
        match zun_gc::with_current_heap_mut(|heap| value.box_any(heap)) {
            Some(any) => Value::Any(any),
            None => value.clone(),
        }
    }
}

/// The numeric value of a property key that addresses a typed-array element
/// (a number, or a canonical numeric string), or `None` for an ordinary key.
fn typed_numeric_key(key: &Value) -> Option<f64> {
    match key {
        Value::Int(n, _) => Some(*n as f64),
        Value::Float(f, _) => Some(*f),
        Value::Str(text) => {
            let text: &str = text;
            if text == "-0" {
                return Some(-0.0);
            }
            let parsed: f64 = match text {
                "NaN" => f64::NAN,
                "Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                _ => {
                    if text.is_empty()
                        || !text
                            .chars()
                            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '.' | 'e' | 'E' | '+'))
                    {
                        return None;
                    }
                    text.parse::<f64>().ok()?
                }
            };
            let canonical = zun_natives::format_float(parsed);
            (canonical == text).then_some(parsed)
        }
        _ => None,
    }
}

impl Interp<'_> {
    /// `view[key]` on a typed array: `Some(value)` when `key` is numeric (an
    /// out-of-range index reads `undefined`), `None` for an ordinary key.
    pub(crate) fn typed_view_index_get(
        &mut self,
        view: zun_gc::Obj,
        key: &Value,
    ) -> Option<Value> {
        crate::typed_arrays::element_of(view)?;
        let n = typed_numeric_key(key)?;
        let len = crate::typed_arrays::element_len(view).unwrap_or(0);
        if n.fract() != 0.0 || n < 0.0 || n >= len as f64 || (n == 0.0 && n.is_sign_negative()) {
            return Some(Value::Void);
        }
        Some(self.view_read(view, n as usize))
    }

    /// `view[key] = value` on a typed array. `Ok(true)` when `key` is numeric
    /// (the value is converted even when the index is out of range, which then
    /// stores nothing), `Ok(false)` for an ordinary key.
    pub(crate) fn typed_view_index_set(
        &mut self,
        view: zun_gc::Obj,
        key: &Value,
        value: &Value,
    ) -> Result<bool, RunError> {
        if crate::typed_arrays::element_of(view).is_none() {
            return Ok(false);
        }
        let Some(n) = typed_numeric_key(key) else {
            return Ok(false);
        };
        let len = crate::typed_arrays::element_len(view).unwrap_or(0);
        let in_range =
            n.fract() == 0.0 && n >= 0.0 && n < len as f64 && !(n == 0.0 && n.is_sign_negative());
        // The value is converted first, so a throwing `valueOf` surfaces even
        // for an index that is out of range.
        if in_range {
            self.view_write(view, n as usize, value)?;
        } else {
            let scratch = crate::typed_arrays::alloc_of(
                crate::typed_arrays::element_of(view).ok_or(RunError::BadToken)?,
                1,
            );
            if let Some(scratch) = scratch {
                self.view_write(scratch, 0, value)?;
            }
        }
        Ok(true)
    }
}

impl Interp<'_> {
    /// `key in view` for a typed array: `Some(answer)` when `view` is one. A
    /// numeric key is a present element only when in range (no chain lookup);
    /// any other key answers for the geometry accessors and the shared methods.
    pub(crate) fn typed_view_has(&mut self, view: zun_gc::Obj, key: &Value) -> Option<bool> {
        crate::typed_arrays::element_of(view)?;
        if let Some(n) = typed_numeric_key(key) {
            let len = crate::typed_arrays::element_len(view).unwrap_or(0);
            return Some(
                n.fract() == 0.0
                    && n >= 0.0
                    && n < len as f64
                    && !(n == 0.0 && n.is_sign_negative()),
            );
        }
        let Value::Str(name) = key else {
            return Some(false);
        };
        Some(matches!(
            name.as_ref(),
            "length"
                | "byteLength"
                | "byteOffset"
                | "buffer"
                | "BYTES_PER_ELEMENT"
                | "constructor"
                | "toString"
                | "valueOf"
                | "hasOwnProperty"
                | "at"
                | "copyWithin"
                | "entries"
                | "every"
                | "fill"
                | "filter"
                | "find"
                | "findIndex"
                | "findLast"
                | "findLastIndex"
                | "forEach"
                | "includes"
                | "indexOf"
                | "join"
                | "keys"
                | "lastIndexOf"
                | "map"
                | "reduce"
                | "reduceRight"
                | "reverse"
                | "set"
                | "slice"
                | "some"
                | "sort"
                | "subarray"
                | "toLocaleString"
                | "toReversed"
                | "toSorted"
                | "values"
                | "with"
        ))
    }
}
