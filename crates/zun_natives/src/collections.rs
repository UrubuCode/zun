use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError("managed collection intrinsic requires an interpreter heap".into()))
}

/// Signatures for collection operations. The interpreter supplies the managed
/// heap context; these declarations keep module resolution and type checking
/// independent of the GC crate.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:collections",
    functions: &[
        NativeFn { name: "mapNew", params: &[], ret: Opaque, imp: unavailable },
        NativeFn { name: "mapSet", params: &[Opaque, Opaque, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "mapGet", params: &[Opaque, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "mapHas", params: &[Opaque, Opaque], ret: Bool, imp: unavailable },
        NativeFn { name: "mapSize", params: &[Opaque], ret: U64, imp: unavailable },
        NativeFn { name: "setNew", params: &[], ret: Opaque, imp: unavailable },
        NativeFn { name: "setAdd", params: &[Opaque, Opaque], ret: Bool, imp: unavailable },
        NativeFn { name: "setHas", params: &[Opaque, Opaque], ret: Bool, imp: unavailable },
        NativeFn { name: "setSize", params: &[Opaque], ret: U64, imp: unavailable },
    ],
};
