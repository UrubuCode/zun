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
            params: &[Str, Str],
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
            name: "regexpExec",
            params: &[RegExp, Str],
            ret: Array,
            imp: unavailable,
        },
        NativeFn {
            name: "regexpSearch",
            params: &[RegExp, Str],
            ret: I32,
            imp: unavailable,
        },
        NativeFn {
            name: "regexpSplit",
            params: &[RegExp, Str],
            ret: Array,
            imp: unavailable,
        },
        NativeFn {
            name: "regexpReplace",
            params: &[RegExp, Str, Str],
            ret: Str,
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
        NativeFn { name: "mapSetF64", params: &[Map, F64, F64], ret: Bool, imp: unavailable },
        NativeFn { name: "mapGetF64Or", params: &[Map, F64, F64], ret: F64, imp: unavailable },
        NativeFn { name: "mapHasI64", params: &[Map, I64], ret: Bool, imp: unavailable },
        NativeFn { name: "mapHasI32", params: &[Map, I32], ret: Bool, imp: unavailable },
        NativeFn { name: "mapDeleteI64", params: &[Map, I64], ret: Bool, imp: unavailable },
        NativeFn { name: "setAddI64", params: &[Set, I64], ret: Bool, imp: unavailable },
        NativeFn { name: "setHasI64", params: &[Set, I64], ret: Bool, imp: unavailable },
        NativeFn { name: "setHasI32", params: &[Set, I32], ret: Bool, imp: unavailable },
        NativeFn { name: "setDeleteI64", params: &[Set, I64], ret: Bool, imp: unavailable },
        NativeFn { name: "mapSetI64", params: &[Map, I64, I64], ret: Bool, imp: unavailable },
        NativeFn { name: "mapSetI32", params: &[Map, I32, I32], ret: Bool, imp: unavailable },
        NativeFn { name: "setAddI32", params: &[Set, I32], ret: Bool, imp: unavailable },
        NativeFn { name: "mapDeleteI32", params: &[Map, I32], ret: Bool, imp: unavailable },
        NativeFn { name: "setDeleteI32", params: &[Set, I32], ret: Bool, imp: unavailable },
        NativeFn { name: "mapGetI64Or", params: &[Map, I64, I64], ret: I64, imp: unavailable },
        NativeFn { name: "mapGetI32Or", params: &[Map, I32, I32], ret: I32, imp: unavailable },
        NativeFn { name: "bigIntNew", params: &[Str], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntAdd", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntSub", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntMul", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntDiv", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntRem", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntNeg", params: &[BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntCompare", params: &[BigInt, BigInt], ret: I32, imp: unavailable },
        NativeFn { name: "bigIntEq", params: &[BigInt, BigInt], ret: Bool, imp: unavailable },
        NativeFn { name: "bigIntNe", params: &[BigInt, BigInt], ret: Bool, imp: unavailable },
        NativeFn { name: "bigIntLt", params: &[BigInt, BigInt], ret: Bool, imp: unavailable },
        NativeFn { name: "bigIntLe", params: &[BigInt, BigInt], ret: Bool, imp: unavailable },
        NativeFn { name: "bigIntGt", params: &[BigInt, BigInt], ret: Bool, imp: unavailable },
        NativeFn { name: "bigIntGe", params: &[BigInt, BigInt], ret: Bool, imp: unavailable },
        NativeFn { name: "bigIntToString", params: &[BigInt], ret: Str, imp: unavailable },
        // Dynamic member calls on `any` receivers (module namespaces from `import()`).
        NativeFn { name: "dynamicCall0", params: &[Opaque, Str], ret: Opaque, imp: unavailable },
        NativeFn { name: "dynamicCall1", params: &[Opaque, Str, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "dynamicCall2", params: &[Opaque, Str, Opaque, Opaque], ret: Opaque, imp: unavailable },
        // `Object.*` reflective statics. These touch the managed heap (dynamic
        // objects and typed field layouts), so the interpreter intercepts them
        // by name; the signatures here fix only the argument count the verifier
        // pops. `Opaque`/`Any` stand for "any managed value".
        NativeFn { name: "objectKeys", params: &[Opaque], ret: Array, imp: unavailable },
        NativeFn { name: "objectValues", params: &[Opaque], ret: Array, imp: unavailable },
        NativeFn { name: "objectEntries", params: &[Opaque], ret: Array, imp: unavailable },
        NativeFn { name: "objectAssign", params: &[DynamicObject, Opaque], ret: DynamicObject, imp: unavailable },
        NativeFn { name: "objectFreeze", params: &[Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "objectIsFrozen", params: &[Opaque], ret: Bool, imp: unavailable },
        NativeFn { name: "objectCreate", params: &[Opaque], ret: DynamicObject, imp: unavailable },
        NativeFn { name: "objectGetPrototypeOf", params: &[Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "objectSetPrototypeOf", params: &[DynamicObject, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "objectFromEntries", params: &[Array], ret: DynamicObject, imp: unavailable },
        NativeFn { name: "objectIs", params: &[Opaque, Opaque], ret: Bool, imp: unavailable },
        NativeFn { name: "objectHasOwn", params: &[Opaque, Str], ret: Bool, imp: unavailable },
        NativeFn { name: "objectIsPrototypeOf", params: &[Opaque, Opaque], ret: Bool, imp: unavailable },
        // `Function.prototype.apply`/`bind` reflective invokers. They re-enter
        // the interpreter (variadic over a managed Array of arguments), so the
        // interpreter intercepts them by name; the signatures here fix only the
        // argument count the verifier pops. `apply(function, thisArg, argsArray)`
        // invokes with `thisArg` as the leading positional argument spread with
        // the array; `bind(function, prependedArray)` allocates a bound function
        // value whose prepended arguments are `[thisArg, ...partialArgs]`.
        NativeFn { name: "functionApply", params: &[Function, Opaque, Array], ret: Opaque, imp: unavailable },
        NativeFn { name: "functionBind", params: &[Function, Array], ret: Function, imp: unavailable },
        // Realm-global registration (PLAN §3.1). Lean positional signature:
        // `global.set(name, value, readonly)`; registers `name -> value` on the
        // realm global singleton, non-writable when `readonly`.
        NativeFn { name: "globalSet", params: &[Str, Opaque, Bool], ret: Void, imp: unavailable },
        // Realm base-type class descriptors (PLAN §3.1, opaque representation).
        // `realmDescriptor(token)` boxes a base-type metadata token into an
        // opaque class-descriptor `Any`; `classDescriptorToken(value)` reads the
        // token back (or `-1` for a non-descriptor), used to observe descriptors
        // seeded onto the realm global.
        NativeFn { name: "realmDescriptor", params: &[F64], ret: Opaque, imp: unavailable },
        NativeFn { name: "classDescriptorToken", params: &[Opaque], ret: F64, imp: unavailable },
        // `Symbol` surface. Symbols are isolate-local, so the interpreter owns
        // the registry and intercepts these by name; the signatures fix only the
        // argument count the verifier pops. A symbol is an immediate `Any`
        // (`AnyTag::Symbol`), hence `Opaque` in/out.
        // `symbolNew(description)` mints a unique symbol (`Void` description →
        // `undefined`); `symbolFor(key)`/`symbolKeyFor(symbol)` are the
        // cross-realm registry; `symbolWellKnown(index)` returns the stable
        // well-known symbol value; `symbolDescription(symbol)` reads `.description`.
        NativeFn { name: "symbolNew", params: &[Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "symbolFor", params: &[Str], ret: Opaque, imp: unavailable },
        NativeFn { name: "symbolKeyFor", params: &[Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "symbolWellKnown", params: &[F64], ret: Opaque, imp: unavailable },
        NativeFn { name: "symbolDescription", params: &[Opaque], ret: Opaque, imp: unavailable },
    ],
};
