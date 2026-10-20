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
        NativeFn { name: "bufferReadU32", params: &[Opaque, U64], ret: U32, imp: unavailable },
        NativeFn { name: "bufferWriteU32", params: &[Opaque, U64, U32], ret: Bool, imp: unavailable },
        NativeFn { name: "bufferReadF64", params: &[Opaque, U64], ret: F64, imp: unavailable },
        NativeFn { name: "bufferWriteF64", params: &[Opaque, U64, F64], ret: Bool, imp: unavailable },
        NativeFn { name: "dataViewNew", params: &[Opaque, U64, U64], ret: Opaque, imp: unavailable },
        NativeFn { name: "dataViewGetU32", params: &[Opaque, U64, Bool], ret: U32, imp: unavailable },
        NativeFn { name: "dataViewSetU32", params: &[Opaque, U64, U32, Bool], ret: Bool, imp: unavailable },
        NativeFn { name: "dataViewGetU16", params: &[Opaque, U64, Bool], ret: U16, imp: unavailable },
        NativeFn { name: "dataViewSetU16", params: &[Opaque, U64, U16, Bool], ret: Bool, imp: unavailable },
        NativeFn { name: "dataViewGetF64", params: &[Opaque, U64, Bool], ret: F64, imp: unavailable },
        NativeFn { name: "dataViewSetF64", params: &[Opaque, U64, F64, Bool], ret: Bool, imp: unavailable },
    ],
};
