//! `zun:io` — console output. Signature and logic together.
//!
//! `print` is listed first so its registry id is 0 (the implicit builtin).

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn print_line(args: &[NativeArg], sink: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    sink.println(&args[0].display());
    Ok(NativeValue::Void)
}

/// Write to the error stream (`stderr`). Backs `console.error`/`console.warn`.
fn eprint_line(args: &[NativeArg], sink: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    sink.eprintln(&args[0].display());
    Ok(NativeValue::Void)
}

/// The `zun:io` module. `print` accepts any primitive (checked by the compiler).
///
/// `print`/`println` stay first so `print` keeps intrinsic id 0 (the implicit
/// builtin). New functions are appended at the end of the module (see the
/// native-id convention): `eprint`/`eprintln` write to the error stream.
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
        NativeFn {
            name: "eprint",
            params: &[Str],
            ret: Void,
            imp: eprint_line,
        },
        NativeFn {
            name: "eprintln",
            params: &[Str],
            ret: Void,
            imp: eprint_line,
        },
    ],
};
