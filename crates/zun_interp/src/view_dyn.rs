//! Typed-array / `Buffer` / `ArrayBuffer` members reached through an
//! `any`-typed receiver.
//!
//! A statically typed `buf.toString(..)`, `buf.length` and `buf[i]` never do a
//! run-time member lookup: the checker lowers them to direct intrinsics from
//! the receiver's compile-time type. Through `any` there is no such type, so
//! the dynamic arms (`dynamicCall*`, `LoadDynamic`, `LoadDynamicValue`,
//! `StoreDynamicValue`) special-case a concrete view receiver and reuse the
//! same natives. The member -> native mapping (with the optional-argument
//! defaults the checker applies at compile time) is the shared
//! [`zun_natives::view_member`] table.

use super::value::Value;
use super::{Interp, RunError};
use zun_gc::Obj;
use zun_il::NumKind;
use zun_natives::{NativeTy, ViewArg};

/// What a numeric property of a view answers, or `None` when `name` is not one.
/// `ArrayBuffer` answers `byteLength` (and `maxByteLength`/`resizable`) too.
pub(crate) fn view_property(object: Obj, name: &str) -> Option<Value> {
    zun_gc::with_current_heap(|heap| {
        if heap.typed_array_element(object).is_some() {
            return match name {
                "length" => heap.typed_array_len(object).map(number),
                "byteLength" => heap.typed_array_byte_len(object).map(number),
                "byteOffset" => heap.typed_array_byte_offset(object).map(number),
                "buffer" => heap.typed_array_buffer(object).map(Value::Object),
                _ => None,
            };
        }
        match name {
            "byteLength" => heap.buffer_len(object).map(number),
            "maxByteLength" => heap.buffer_max_byte_length(object).map(number),
            "resizable" => heap.buffer_resizable(object).map(Value::Bool),
            _ => None,
        }
    })
}

fn number(value: usize) -> Value {
    Value::Float(value as f64, NumKind::F64)
}

/// An array index key (`v[3]`, `v["3"]`) as an element position.
pub(crate) fn element_index(key: &Value) -> Option<usize> {
    match key {
        Value::Int(n, _) if *n >= 0 => Some(*n as usize),
        Value::Float(n, _) if *n >= 0.0 && n.fract() == 0.0 => Some(*n as usize),
        Value::Str(text) => text.parse::<usize>().ok().filter(|at| at.to_string() == **text),
        _ => None,
    }
}

/// Store one JS value into element `index` of a view with the ECMAScript
/// conversion for its kind: an integer kind wraps modulo 2^n (`300` into a
/// `Uint8Array` stores `44`), the clamped and float kinds convert as usual.
/// `None` when the index is out of range or the value is not numeric.
pub(crate) fn write_js_element(view: Obj, index: usize, value: &Value) -> Option<bool> {
    use zun_core::TypedElement as T;
    let element = zun_gc::with_current_heap(|heap| heap.typed_array_element(view))?;
    let value = match (value, element) {
        (Value::Float(n, _), kind) if !matches!(kind, T::F32 | T::F64 | T::U8Clamped) => {
            Value::Int(zun_core::to_int_wrapping(*n), NumKind::I64)
        }
        (other, _) => other.clone(),
    };
    crate::typed_arrays::write_element(view, index, &value)
}

/// Read element `index` of a view as an interpreter value (`undefined` when
/// out of range, as in JS).
pub(crate) fn read_element_or_undefined(view: Obj, index: usize) -> Value {
    crate::typed_arrays::read_element(view, index).unwrap_or(Value::Void)
}

/// Coerce a dynamic argument to what the native's declared parameter wants:
/// JS numbers arrive as floats, the natives take exact integer widths.
fn coerce(value: Value, param: Option<&NativeTy>) -> Value {
    use NativeTy::*;
    let kind = match param {
        Some(I8) => Some(NumKind::I8),
        Some(I16) => Some(NumKind::I16),
        Some(I32) => Some(NumKind::I32),
        Some(I64) => Some(NumKind::I64),
        Some(U8) => Some(NumKind::U8),
        Some(U16) => Some(NumKind::U16),
        Some(U32) => Some(NumKind::U32),
        Some(U64) => Some(NumKind::U64),
        _ => None,
    };
    match (value, kind, param) {
        (Value::Float(n, _), Some(kind), _) => Value::Int(n as i64, kind),
        (Value::Int(n, _), Some(kind), _) => Value::Int(n, kind),
        (Value::Int(n, _), None, Some(F64)) => Value::Float(n as f64, NumKind::F64),
        (other, _, _) => other,
    }
}

impl<'a> Interp<'a> {
    /// Call `member` on the typed-array view `view` with already-unboxed
    /// `args`. `Ok(None)` when the receiver is not a view or the member is not
    /// one of its methods (the caller then carries on to "is not a function").
    pub(crate) fn dynamic_view_call(
        &mut self,
        view: Obj,
        member: &str,
        args: &[Value],
    ) -> Result<Option<Value>, RunError> {
        let Some(element) = zun_gc::with_current_heap(|heap| heap.typed_array_element(view)) else {
            return Ok(None);
        };
        let is_u8 = element == zun_core::TypedElement::U8;
        let present = |slot: usize| args.get(slot).filter(|value| !matches!(value, Value::Void));
        let length = crate::typed_arrays::element_len(view).unwrap_or(0);
        let relative = |value: Option<&Value>, default: usize| -> usize {
            let Some(value) = value else { return default };
            let at = match value {
                Value::Int(n, _) => *n,
                Value::Float(n, _) if n.is_nan() => 0,
                Value::Float(n, _) => *n as i64,
                _ => 0,
            };
            if at < 0 { (length as i64 + at).max(0) as usize } else { (at as usize).min(length) }
        };
        match member {
            // Indexed `set(index, value)` (the bulk `set(source, offset?)` is
            // the table's `typedArraySetFrom`).
            "set" if args.len() == 2 && !matches!(args[0], Value::Object(_)) => {
                let index = element_index(&args[0]).ok_or(RunError::BadToken)?;
                write_js_element(view, index, &args[1]).ok_or(RunError::BadToken)?;
                return Ok(Some(Value::Void));
            }
            // `fill(value, start?, end?)` on any kind; a one-argument `fill` on
            // a `Uint8Array` is the Buffer form handled by the table.
            "fill" if !(is_u8 && args.len() == 1) => {
                let value = args.first().ok_or(RunError::BadToken)?;
                let begin = relative(present(1), 0);
                let end = relative(present(2), length);
                let range = zun_gc::with_current_heap(|heap| heap.typed_array_fill_range(view, begin, end));
                for index in range.ok_or(RunError::BadToken)? {
                    write_js_element(view, index, value).ok_or(RunError::BadToken)?;
                }
                return Ok(Some(Value::Object(view)));
            }
            // `Array.prototype.toString` on a non-byte view is `join(",")`.
            "toString" if !is_u8 => {
                let native = [Value::Object(view), Value::Str(",".into())];
                return self.call_managed_collection("typedArrayJoin", &native).map(Some);
            }
            "subarray" => {
                let begin = relative(present(0), 0);
                let end = relative(present(1), length);
                let child = zun_gc::with_current_heap_mut(|heap| heap.typed_array_subarray(view, begin, end))
                    .ok_or(RunError::BadToken)?;
                return Ok(Some(Value::Object(child)));
            }
            _ => {}
        }
        let Some(spec) = zun_natives::view_member(member, is_u8, args.len()) else {
            return Ok(None);
        };
        // Apply the checker's optional-argument defaults, in JS call order.
        let mut slots = Vec::with_capacity(spec.args.len());
        for (slot, kind) in spec.args.iter().enumerate() {
            let value = match (present(slot), kind) {
                (Some(value), ViewArg::RequiredAny) => {
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or_else(zun_core::Any::void);
                    Value::Any(boxed)
                }
                (Some(value), _) => value.clone(),
                (None, ViewArg::U64(default)) => Value::Int(*default as i64, NumKind::U64),
                (None, ViewArg::I64(default)) => Value::Int(*default, NumKind::I64),
                (None, ViewArg::Str(default)) => Value::Str((*default).into()),
                (None, ViewArg::Required | ViewArg::RequiredAny) => return Ok(None),
            };
            slots.push(value);
        }
        if spec.value_first && slots.len() >= 2 {
            slots.swap(0, 1);
        }
        let params = zun_natives::resolve(spec.module, &spec.native).map(|(_, info)| info.params);
        let mut native_args = Vec::with_capacity(slots.len() + 1);
        native_args.push(Value::Object(view));
        for (slot, value) in slots.into_iter().enumerate() {
            let param = params.and_then(|params| params.get(slot + 1));
            native_args.push(coerce(value, param));
        }
        self.call_managed_collection(&spec.native, &native_args).map(Some)
    }
}
