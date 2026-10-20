use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError(
        "Promise intrinsic requires an interpreter heap".into(),
    ))
}

pub const MODULE: crate::Module = crate::Module {
    name: "zun:promise",
    functions: &[
        NativeFn { name: "promiseNew", params: &[], ret: Promise, imp: unavailable },
        NativeFn {
            name: "promiseResolve",
            params: &[Promise, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "promiseReject",
            params: &[Promise, Str],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn { name: "promiseState", params: &[Promise], ret: U8, imp: unavailable },
    ],
};
