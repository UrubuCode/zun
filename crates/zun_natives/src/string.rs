//! `zun:string` — machine-facing string primitives.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn length(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    match args.first() {
        Some(NativeArg::Str(value)) => Ok(NativeValue::Int(value.chars().count() as i64)),
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

/// The machine-facing string primitives used by `zun:core`.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:string",
    functions: &[
        NativeFn { name: "length", params: &[Str], ret: U64, imp: length },
        NativeFn { name: "concat", params: &[Str, Str], ret: Str, imp: concat },
    ],
};
