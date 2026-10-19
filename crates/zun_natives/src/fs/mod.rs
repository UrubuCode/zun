//! `zun:fs` — filesystem access. Signature and logic together; split into more
//! files here (e.g. `dir.rs`) as it grows.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn read_text(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    std::fs::read_to_string(args[0].as_str())
        .map(NativeValue::Str)
        .map_err(|e| NativeError(format!("readText: {e}")))
}

fn write_text(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    std::fs::write(args[0].as_str(), args[1].as_str())
        .map(|()| NativeValue::Void)
        .map_err(|e| NativeError(format!("writeText: {e}")))
}

fn exists(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(std::path::Path::new(args[0].as_str()).exists()))
}

fn remove_file(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    std::fs::remove_file(args[0].as_str())
        .map(|()| NativeValue::Void)
        .map_err(|e| NativeError(format!("removeFile: {e}")))
}

/// The `zun:fs` module.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:fs",
    functions: &[
        NativeFn { name: "readText", params: &[Str], ret: Str, imp: read_text },
        NativeFn { name: "writeText", params: &[Str, Str], ret: Void, imp: write_text },
        NativeFn { name: "exists", params: &[Str], ret: Bool, imp: exists },
        NativeFn { name: "removeFile", params: &[Str], ret: Void, imp: remove_file },
    ],
};
