//! `zun:math` — deterministic numeric intrinsics.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn float_arg(args: &[NativeArg], name: &str) -> Result<f64, NativeError> {
    match args.first() {
        Some(NativeArg::Float(value)) => Ok(*value),
        Some(NativeArg::Int(value)) => Ok(*value as f64),
        _ => Err(NativeError(format!("{name}: expected a number"))),
    }
}

fn abs(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Float(float_arg(args, "abs")?.abs()))
}

fn sqrt(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Float(float_arg(args, "sqrt")?.sqrt()))
}

fn min(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let left = float_arg(args, "min")?;
    let right = match args.get(1) {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("min: expected two numbers".to_string())),
    };
    Ok(NativeValue::Float(left.min(right)))
}

fn max(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let left = float_arg(args, "max")?;
    let right = match args.get(1) {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("max: expected two numbers".to_string())),
    };
    Ok(NativeValue::Float(left.max(right)))
}

fn receiver_value(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.get(1) {
        Some(NativeArg::Float(value)) => Ok(NativeValue::Float(*value)),
        Some(NativeArg::Int(value)) => Ok(NativeValue::Float(*value as f64)),
        _ => Err(NativeError("receiverValue: expected a numeric value".to_string())),
    }
}

/// The runtime-backed numeric portion of the standard library.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:math",
    functions: &[
        NativeFn { name: "abs", params: &[F64], ret: F64, imp: abs },
        NativeFn { name: "sqrt", params: &[F64], ret: F64, imp: sqrt },
        NativeFn { name: "min", params: &[F64, F64], ret: F64, imp: min },
        NativeFn { name: "max", params: &[F64, F64], ret: F64, imp: max },
        NativeFn { name: "sqrtNumber", params: &[F64], ret: F64, imp: sqrt },
        NativeFn { name: "minNumber", params: &[F64, F64], ret: F64, imp: min },
        NativeFn { name: "maxNumber", params: &[F64, F64], ret: F64, imp: max },
        NativeFn { name: "receiverValue", params: &[Opaque, F64], ret: F64, imp: receiver_value },
    ],
};
