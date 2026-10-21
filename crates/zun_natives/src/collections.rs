use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError(
        "managed collection intrinsic requires an interpreter heap".into(),
    ))
}

/// Signatures for collection operations. The interpreter supplies the managed
/// heap context; these declarations keep module resolution and type checking
/// independent of the GC crate.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:collections",
    functions: &[
        NativeFn {
            name: "mapNew",
            params: &[],
            ret: Map,
            imp: unavailable,
        },
        NativeFn {
            name: "mapSet",
            params: &[Map, Opaque, Opaque],
            ret: Void,
            imp: unavailable,
        },
        NativeFn {
            name: "mapGet",
            params: &[Map, Opaque],
            ret: Opaque,
            imp: unavailable,
        },
        NativeFn {
            name: "mapHas",
            params: &[Map, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "mapSize",
            params: &[Map],
            ret: U64,
            imp: unavailable,
        },
        NativeFn {
            name: "mapDelete",
            params: &[Map, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "mapClear",
            params: &[Map],
            ret: Void,
            imp: unavailable,
        },
        NativeFn {
            name: "mapValues",
            params: &[Map],
            ret: Array,
            imp: unavailable,
        },
        NativeFn { name: "mapKeys", params: &[Map], ret: Array, imp: unavailable },
        NativeFn { name: "mapEntries", params: &[Map], ret: Array, imp: unavailable },
        NativeFn {
            name: "setNew",
            params: &[],
            ret: Set,
            imp: unavailable,
        },
        NativeFn {
            name: "setAdd",
            params: &[Set, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "setHas",
            params: &[Set, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "setSize",
            params: &[Set],
            ret: U64,
            imp: unavailable,
        },
        NativeFn {
            name: "setDelete",
            params: &[Set, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "setClear",
            params: &[Set],
            ret: Void,
            imp: unavailable,
        },
        NativeFn {
            name: "setValues",
            params: &[Set],
            ret: Array,
            imp: unavailable,
        },
        NativeFn { name: "setEntries", params: &[Set], ret: Array, imp: unavailable },
        NativeFn {
            name: "weakMapNew",
            params: &[],
            ret: WeakMap,
            imp: unavailable,
        },
        NativeFn {
            name: "weakMapSet",
            params: &[WeakMap, Opaque, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "weakMapGet",
            params: &[WeakMap, Opaque],
            ret: Opaque,
            imp: unavailable,
        },
        NativeFn {
            name: "weakMapHas",
            params: &[WeakMap, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "weakMapDelete",
            params: &[WeakMap, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "regexpNew",
            params: &[Str],
            ret: RegExp,
            imp: unavailable,
        },
        NativeFn {
            name: "regexpIsMatch",
            params: &[RegExp, Str],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "regexpSource",
            params: &[RegExp],
            ret: Str,
            imp: unavailable,
        },
        NativeFn {
            name: "finalizationRegistryNew",
            params: &[],
            ret: FinalizationRegistry,
            imp: unavailable,
        },
        NativeFn {
            name: "finalizationRegister",
            params: &[FinalizationRegistry, Opaque, Function],
            ret: U64,
            imp: unavailable,
        },
        NativeFn {
            name: "finalizationUnregister",
            params: &[FinalizationRegistry, U64],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "reflectHas",
            params: &[DynamicObject, Str],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "reflectDelete",
            params: &[DynamicObject, Str],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn {
            name: "proxyNew",
            params: &[DynamicObject, DynamicObject],
            ret: Proxy,
            imp: unavailable,
        },
        NativeFn {
            name: "proxyGet",
            params: &[Proxy, Str],
            ret: Opaque,
            imp: unavailable,
        },
        NativeFn {
            name: "proxySet",
            params: &[Proxy, Str, Opaque],
            ret: Bool,
            imp: unavailable,
        },
        NativeFn { name: "proxyHas", params: &[Proxy, Str], ret: Bool, imp: unavailable },
        NativeFn { name: "proxyDelete", params: &[Proxy, Str], ret: Bool, imp: unavailable },
        NativeFn {
            name: "dynamicSetFunction",
            params: &[DynamicObject, Str, Function],
            ret: Void,
            imp: unavailable,
        },
        // Append new intrinsics so existing positional native IDs remain stable.
        NativeFn { name: "mapHasF64", params: &[Map, F64], ret: Bool, imp: unavailable },
        NativeFn { name: "mapDeleteF64", params: &[Map, F64], ret: Bool, imp: unavailable },
        NativeFn { name: "setAddF64", params: &[Set, F64], ret: Bool, imp: unavailable },
        NativeFn { name: "setHasF64", params: &[Set, F64], ret: Bool, imp: unavailable },
        NativeFn { name: "setDeleteF64", params: &[Set, F64], ret: Bool, imp: unavailable },
    ],
};
