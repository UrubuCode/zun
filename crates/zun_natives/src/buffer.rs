use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError("managed buffer intrinsic requires an interpreter heap".into()))
}

/// Byte-buffer intrinsics backed by the GC-owned contiguous buffer.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:buffer",
    functions: &[
        NativeFn { name: "bufferNew", params: &[U64], ret: Opaque, imp: unavailable },
        NativeFn { name: "bufferLength", params: &[Opaque], ret: U64, imp: unavailable },
        NativeFn { name: "bufferGetU8", params: &[Opaque, U64], ret: U8, imp: unavailable },
        NativeFn { name: "bufferSetU8", params: &[Opaque, U64, U8], ret: Bool, imp: unavailable },
    ],
};
