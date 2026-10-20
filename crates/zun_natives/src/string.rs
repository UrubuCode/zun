//! `zun:string` — machine-facing string primitives.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn length(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Int(value.encode_utf16().count() as i64)),
        _ => Err(NativeError("length: expected a string".to_string())),
    }
}

fn concat(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(left)), Some(NativeArg::Str(right))) => {
            Ok(NativeValue::Str(format!("{left}{right}")))
        }
        _ => Err(NativeError("concat: expected two strings".to_string())),
    }
}

fn contains(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match (args.first(), args.get(1)) {
        (Some(NativeArg::Str(value)), Some(NativeArg::Str(needle))) => {
            Ok(NativeValue::Bool(value.contains(needle)))
        }
        _ => Err(NativeError("contains: expected two strings".to_string())),
    }
}

fn char_code_at(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let value = match args.first() {
        Some(NativeArg::Str(value)) => value,
        _ => return Err(NativeError("charCodeAt: expected a string".to_string())),
    };
    let index = match args.get(1) {
        Some(NativeArg::Int(index)) if *index >= 0 => *index as usize,
        _ => return Err(NativeError("charCodeAt: expected a non-negative index".to_string())),
    };
    value
        .encode_utf16()
        .nth(index)
        .map(|unit| NativeValue::Int(unit as i64))
        .ok_or_else(|| NativeError(format!("charCodeAt: index {index} is out of bounds")))
}

/// The machine-facing string primitives used by `zun:core`.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:string",
    functions: &[
        NativeFn { name: "length", params: &[Str], ret: U64, imp: length },
        NativeFn { name: "concat", params: &[Str, Str], ret: Str, imp: concat },
        NativeFn { name: "contains", params: &[Str, Str], ret: Bool, imp: contains },
        NativeFn { name: "charCodeAt", params: &[Str, U64], ret: U32, imp: char_code_at },
    ],
};
