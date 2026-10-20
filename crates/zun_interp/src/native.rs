//! Conversion boundary between interpreter values and native intrinsics.

use super::value::{round_float, wrap_int, Value};

/// Borrow a [`Value`] as the neutral native argument representation.
pub(crate) fn to_native_arg(v: &Value) -> zun_natives::NativeArg<'_> {
    match v {
        Value::Int(n, _) => zun_natives::NativeArg::Int(*n),
        Value::Float(n, _) => zun_natives::NativeArg::Float(*n),
        Value::Bool(b) => zun_natives::NativeArg::Bool(*b),
        Value::Str(s) => zun_natives::NativeArg::Str(s),
        Value::Object(object) => {
            zun_natives::NativeArg::Any(zun_core::Any::object(object.addr() as usize))
        }
        Value::Any(value) => zun_natives::NativeArg::Any(*value),
        Value::Function(function) => zun_natives::NativeArg::Function(*function),
        Value::Void => zun_natives::NativeArg::Void,
        Value::Null => zun_natives::NativeArg::Void,
    }
}

/// Convert a native result back to an interpreter value using the intrinsic's
/// declared return type to preserve the integer or floating-point width.
pub(crate) fn from_native_value(v: zun_natives::NativeValue, id: u32) -> Value {
    use zun_natives::{NativeTy, NativeValue};
    match v {
        NativeValue::Void => Value::Void,
        NativeValue::Bool(b) => Value::Bool(b),
        NativeValue::Str(s) => Value::Str(s.into()),
        NativeValue::Any(value) => Value::Any(value),
        NativeValue::Float(n) => {
            let kind = match zun_natives::by_id(id).map(|f| f.ret) {
                Some(NativeTy::F32) => zun_il::NumKind::F32,
                _ => zun_il::NumKind::F64,
            };
            Value::Float(round_float(n, kind), kind)
        }
        NativeValue::Int(n) => {
            use zun_il::NumKind as K;
            let kind = match zun_natives::by_id(id).map(|f| f.ret) {
                Some(NativeTy::I8) => K::I8,
                Some(NativeTy::I16) => K::I16,
                Some(NativeTy::I64) => K::I64,
                Some(NativeTy::U8) => K::U8,
                Some(NativeTy::U16) => K::U16,
                Some(NativeTy::U32) => K::U32,
                Some(NativeTy::U64) => K::U64,
                _ => K::I32,
            };
            Value::Int(wrap_int(n, kind), kind)
        }
    }
}
