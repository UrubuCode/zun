//! `zun:io` — console output. Signature and logic together.
//!
//! `print` is listed first so its registry id is 0 (the implicit builtin).

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn print_line(args: &[NativeArg], sink: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    sink.println(&args[0].display());
    Ok(NativeValue::Void)
}

/// The `zun:io` module. `print` accepts any primitive (checked by the compiler).
pub const MODULE: crate::Module = crate::Module {
    name: "io",
    functions: &[
        NativeFn {
            name: "print",
            params: &[Str],
            ret: Void,
            imp: print_line,
        },
        NativeFn {
            name: "println",
            params: &[Str],
            ret: Void,
            imp: print_line,
        },
    ],
};
