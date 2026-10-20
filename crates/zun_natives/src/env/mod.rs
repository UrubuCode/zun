//! `zun:env` — process environment.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn get(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Str(
        std::env::var(args[0].as_str()).unwrap_or_default(),
    ))
}

/// The `zun:env` module.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:env",
    functions: &[NativeFn {
        name: "get",
        params: &[Str],
        ret: Str,
        imp: get,
    }],
};
