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

fn unary_float(args: &[NativeArg], name: &str, operation: fn(f64) -> f64) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError(format!("{name}: expected a number"))),
    };
    Ok(NativeValue::Float(operation(value)))
}

fn floor(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "floor", f64::floor)
}

fn ceil(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "ceil", f64::ceil)
}

fn round(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "round", f64::round)
}

fn pow(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let base = float_arg(args, "pow")?;
    let exponent = match args.get(1) {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("pow: expected two numbers".to_string())),
    };
    Ok(NativeValue::Float(base.powf(exponent)))
}

fn sin(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "sin", f64::sin)
}

fn cos(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "cos", f64::cos)
}

fn exp(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "exp", f64::exp)
}

fn log(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    unary_float(args, "log", f64::ln)
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
        NativeFn { name: "floor", params: &[F64], ret: F64, imp: floor },
        NativeFn { name: "ceil", params: &[F64], ret: F64, imp: ceil },
        NativeFn { name: "round", params: &[F64], ret: F64, imp: round },
        NativeFn { name: "floorNumber", params: &[F64], ret: F64, imp: floor },
        NativeFn { name: "ceilNumber", params: &[F64], ret: F64, imp: ceil },
        NativeFn { name: "roundNumber", params: &[F64], ret: F64, imp: round },
        NativeFn { name: "receiverValue", params: &[Opaque, F64], ret: F64, imp: receiver_value },
        NativeFn { name: "powNumber", params: &[F64, F64], ret: F64, imp: pow },
        NativeFn { name: "sinNumber", params: &[F64], ret: F64, imp: sin },
        NativeFn { name: "cosNumber", params: &[F64], ret: F64, imp: cos },
        NativeFn { name: "expNumber", params: &[F64], ret: F64, imp: exp },
        NativeFn { name: "logNumber", params: &[F64], ret: F64, imp: log },
    ],
};
