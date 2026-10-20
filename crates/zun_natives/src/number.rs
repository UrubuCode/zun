//! `zun:number` — numeric conversion primitives.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn parse_float(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let text = match args.first() {
        Some(NativeArg::Str(value)) => value.trim(),
        _ => return Err(NativeError("parseFloat: expected a string".to_string())),
    };
    text.parse::<f64>()
        .map(NativeValue::Float)
        .map_err(|_| NativeError(format!("parseFloat: invalid number `{text}`")))
}

fn to_fixed(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("toFixed: expected a number".to_string())),
    };
    let digits = match args.get(1) {
        Some(NativeArg::Int(value)) if (0..=100).contains(value) => *value as usize,
        _ => return Err(NativeError("toFixed: expected digits from 0 through 100".to_string())),
    };
    Ok(NativeValue::Str(format!("{value:.digits$}")))
}

fn to_string(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Float(value)) => Ok(NativeValue::Str(value.to_string())),
        Some(NativeArg::Int(value)) => Ok(NativeValue::Str(value.to_string())),
        _ => Err(NativeError("toString: expected a number".to_string())),
    }
}

fn is_finite(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("isFinite: expected a number".to_string())),
    };
    Ok(NativeValue::Bool(value.is_finite()))
}

fn is_nan(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Float(value)) => *value,
        Some(NativeArg::Int(value)) => *value as f64,
        _ => return Err(NativeError("isNaN: expected a number".to_string())),
    };
    Ok(NativeValue::Bool(value.is_nan()))
}

/// Runtime-backed numeric conversion functions.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:number",
    functions: &[
        NativeFn { name: "parseFloat", params: &[Str], ret: F64, imp: parse_float },
        NativeFn { name: "toFixed", params: &[F64, I32], ret: Str, imp: to_fixed },
        NativeFn { name: "toString", params: &[F64], ret: Str, imp: to_string },
        NativeFn { name: "isFinite", params: &[F64], ret: Bool, imp: is_finite },
        NativeFn { name: "isNaN", params: &[F64], ret: Bool, imp: is_nan },
    ],
};
