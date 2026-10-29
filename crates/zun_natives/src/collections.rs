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
    name: "collections",
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
            params: &[Function],
            ret: FinalizationRegistry,
            imp: unavailable,
        },
        NativeFn {
            // `(registry, target, heldValue, unregisterToken)`. The held value
            // is an arbitrary immediate `Any` (`Opaque`), GC-traced and
            // delivered to the cleanup callback. The unregister token is an
            // opaque object identity; a non-object value (the sentinel the
            // surface synthesizes when the caller omits the token) means the
            // registration can never be unregistered.
            name: "finalizationRegister",
            params: &[FinalizationRegistry, Opaque, Opaque, Opaque],
            ret: Void,
            imp: unavailable,
        },
        NativeFn {
            name: "finalizationUnregister",
            params: &[FinalizationRegistry, Opaque],
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
        NativeFn { name: "bigIntFromNumber", params: &[F64], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntPow", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntAnd", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntOr", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntXor", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntNot", params: &[BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntShl", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntShr", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntAsIntN", params: &[I64, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "bigIntAsUintN", params: &[I64, BigInt], ret: BigInt, imp: unavailable },
        // Dynamic member calls on `any` receivers (module namespaces from `import()`).
        NativeFn { name: "dynamicCall0", params: &[Opaque, Str], ret: Opaque, imp: unavailable },
        NativeFn { name: "dynamicCall1", params: &[Opaque, Str, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "dynamicCall2", params: &[Opaque, Str, Opaque, Opaque], ret: Opaque, imp: unavailable },
        // `new ns.C(...args)` on a dynamic import namespace: construct a class
        // export cross-module. `(namespace, className, argsArray)` — the array
        // form carries any constructor arity (see `proxyConstruct`).
        NativeFn { name: "dynamicConstruct", params: &[Opaque, Str, Array], ret: Opaque, imp: unavailable },
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
        // Standard `Array.prototype` callback methods. Each re-enters the
        // interpreter, invoking the callback with the ECMAScript signature
        // `(value, index, array)` (the comparator with `(a, b)`); excess
        // positional arguments are dropped to the callback's declared arity.
        // Signatures here fix only the argument count the verifier pops; the
        // interpreter intercepts them by name.
        NativeFn { name: "arrayForEach", params: &[Array, Function], ret: Void, imp: unavailable },
        NativeFn { name: "arrayMap", params: &[Array, Function], ret: Array, imp: unavailable },
        NativeFn { name: "arrayFilter", params: &[Array, Function], ret: Array, imp: unavailable },
        NativeFn { name: "arrayFind", params: &[Array, Function], ret: Opaque, imp: unavailable },
        NativeFn { name: "arrayFindIndex", params: &[Array, Function], ret: I32, imp: unavailable },
        NativeFn { name: "arraySome", params: &[Array, Function], ret: Bool, imp: unavailable },
        NativeFn { name: "arrayEvery", params: &[Array, Function], ret: Bool, imp: unavailable },
        NativeFn { name: "arrayReduce", params: &[Array, Function, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "arrayReduceRight", params: &[Array, Function, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "arraySortBy", params: &[Array, Function], ret: Array, imp: unavailable },
        // Standard `Array.prototype` structural methods that do not take a
        // callback but need host-side element coercion (join) or mutation.
        NativeFn { name: "arraySortDefault", params: &[Array], ret: Array, imp: unavailable },
        NativeFn { name: "arrayJoin", params: &[Array, Str], ret: Str, imp: unavailable },
        NativeFn { name: "arrayShift", params: &[Array], ret: Opaque, imp: unavailable },
        NativeFn { name: "arrayUnshift", params: &[Array, Opaque], ret: U64, imp: unavailable },
        NativeFn { name: "arraySplice", params: &[Array, U64, U64], ret: Array, imp: unavailable },
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
        // WeakSet: a set of weakly-held object keys over the WeakMap ephemeron
        // floor (value-less; the interpreter stores each key as its own value).
        NativeFn { name: "weakSetNew", params: &[], ret: WeakSet, imp: unavailable },
        NativeFn { name: "weakSetAdd", params: &[WeakSet, Opaque], ret: Bool, imp: unavailable },
        NativeFn { name: "weakSetHas", params: &[WeakSet, Opaque], ret: Bool, imp: unavailable },
        NativeFn { name: "weakSetDelete", params: &[WeakSet, Opaque], ret: Bool, imp: unavailable },
        // WeakRef: a single weakly-held reference over the GC weak-reference
        // floor. `deref` yields the target while live, `undefined` once collected.
        NativeFn { name: "weakRefNew", params: &[Opaque], ret: WeakRef, imp: unavailable },
        NativeFn { name: "weakRefDeref", params: &[WeakRef], ret: Opaque, imp: unavailable },
        // `String.prototype.split(separator: string, limit?)` — the
        // string-separator form (the RegExp form stays on `regexpSplit`). A
        // negative `limit` means unbounded. Builds a managed array, so the
        // interpreter owns the body; the signature fixes only the argument
        // count the verifier pops.
        NativeFn { name: "stringSplit", params: &[Str, Str, I64], ret: Array, imp: unavailable },
        // `Map.prototype.forEach(cb)` / `Set.prototype.forEach(cb)`. Each
        // re-enters the interpreter, invoking the callback with the ECMAScript
        // signatures `(value, key, map)` and `(value, value, set)` respectively;
        // excess arguments are dropped to the callback's declared arity. The
        // signatures fix only the argument count the verifier pops.
        NativeFn { name: "mapForEach", params: &[Map, Function], ret: Void, imp: unavailable },
        NativeFn { name: "setForEach", params: &[Set, Function], ret: Void, imp: unavailable },
        // Iterable constructors. `new Map(iterable)` consumes `[key, value]`
        // pairs; `new Set(iterable)` consumes elements. Currently an ARRAY
        // argument is accepted (the common case); arbitrary iterator-protocol
        // objects are deferred.
        NativeFn { name: "mapFrom", params: &[Array], ret: Map, imp: unavailable },
        NativeFn { name: "setFrom", params: &[Array], ret: Set, imp: unavailable },
        // General (non-array) iterable constructors: `new Map(iterable)` /
        // `new Set(iterable)` where the argument is a built-in iterable
        // (another Map/Set, a string, or an array). The raw value reaches the
        // native un-coerced (`Opaque`) so it can dispatch on runtime kind.
        NativeFn { name: "mapFromIterable", params: &[Opaque], ret: Map, imp: unavailable },
        NativeFn { name: "setFromIterable", params: &[Opaque], ret: Set, imp: unavailable },
        // Attach a function under a SYMBOL key on a dynamic object, mirroring
        // the string-keyed `dynamicSetFunction`. Backs behavioral dispatch of
        // well-known symbols whose member is a method (e.g.
        // `[Symbol.hasInstance]`): function values are not boxable into `Any`,
        // so they live in the object's dedicated symbol-function slot rather
        // than the ordinary symbol-value slot. The symbol argument arrives as
        // an immediate `Any` (`AnyTag::Symbol`), hence `Opaque`.
        NativeFn { name: "dynamicSetSymbolFunction", params: &[DynamicObject, Opaque, Function], ret: Void, imp: unavailable },
        // Property-descriptor model statics (NUCLEUS §6). Like the other
        // `object*` reflectors these touch the managed heap and are intercepted
        // by name; the signatures fix only the argument count the verifier
        // pops. Appended last so existing dispatch ids stay stable.
        NativeFn { name: "objectDefineProperty", params: &[DynamicObject, Str, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "objectDefineProperties", params: &[DynamicObject, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "objectGetOwnPropertyDescriptor", params: &[Opaque, Str], ret: Opaque, imp: unavailable },
        NativeFn { name: "objectGetOwnPropertyDescriptors", params: &[Opaque], ret: DynamicObject, imp: unavailable },
        NativeFn { name: "objectGetOwnPropertyNames", params: &[Opaque], ret: Array, imp: unavailable },
        NativeFn { name: "objectSeal", params: &[Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "objectIsSealed", params: &[Opaque], ret: Bool, imp: unavailable },
        NativeFn { name: "objectPreventExtensions", params: &[Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "objectIsExtensible", params: &[Opaque], ret: Bool, imp: unavailable },
        NativeFn { name: "objectPropertyIsEnumerable", params: &[Opaque, Str], ret: Bool, imp: unavailable },
        // `String.prototype.matchAll(regexp)` — array of match arrays (each
        // like `exec`'s result). Requires a global regexp (checked at runtime).
        // Appended last so existing dispatch ids stay stable.
        NativeFn { name: "stringMatchAll", params: &[RegExp, Str], ret: Array, imp: unavailable },
        // `functionBox(function)` wraps a callable in a GC-traced object so a
        // function-valued value can travel through an argument lane that boxes
        // into `Any` (which deliberately rejects bare functions, keeping their
        // captured environment visible to precise root handling). The bound-
        // function record's `__args` lane uses it for function-typed partial
        // arguments; `resolve_call_target` unwraps the box back to the callable.
        // Appended last so existing dispatch ids stay stable.
        NativeFn { name: "functionBox", params: &[Function], ret: Opaque, imp: unavailable },
        // `String(x)` coercion (ECMAScript ToString). The argument arrives as an
        // immediate `Any` (`Opaque`): a non-`any` operand is boxed at the call
        // site, so the one native handles every operand type. For an object it
        // performs OrdinaryToPrimitive(hint "string") via `[Symbol.toPrimitive]`,
        // then falls back to `[Symbol.toStringTag]` → `"[object <Tag>]"`, else
        // `"[object Object]"`. Appended last so existing dispatch ids stay stable.
        NativeFn { name: "stringCoerce", params: &[Opaque], ret: Str, imp: unavailable },
        // `applyDecorator(decoratorFn, value, context)` invokes a decorator
        // factory's result (a function value) with `(value, context)` at
        // class-definition time, discarding its return value. Backs decorator
        // factories `@dec(args)` (the factory call `dec(args)` is evaluated by
        // the caller; its result is this first argument). Appended last so
        // existing dispatch ids stay stable.
        NativeFn { name: "applyDecorator", params: &[Opaque, Opaque, Opaque], ret: Void, imp: unavailable },
        // `classSetSymbolFunction(classRid, symbol, fn)` attaches a method under
        // a Symbol key to a typed class's per-type side table (the typed-class
        // analogue of `dynamicSetSymbolFunction`). Backs class computed-symbol
        // members (`class C { [Symbol.hasInstance](v) {} }`): the method is a
        // function value and the class is identified by its metadata TypeDef
        // rid. The symbol arrives as an immediate `Any`, hence `Opaque`.
        NativeFn { name: "classSetSymbolFunction", params: &[U32, Opaque, Function], ret: Void, imp: unavailable },
        // NUCLEUS #10: Proxy `apply` trap + callable proxy targets. A proxy's
        // target is stored as a GC object; `proxyNewCallable` wraps a *function*
        // target (boxed via `functionBox` into a GC record carrying `__boxedFn`)
        // so a proxy can stand in for a callable. `proxyApply` routes
        // `proxy(...args)` and `Reflect.apply(proxy, thisArg, args)` through the
        // handler's `apply` trap (absent trap → the underlying function). The
        // thisArg arrives as an immediate `Any` (`Opaque`). Appended last so
        // existing dispatch ids stay stable.
        NativeFn { name: "proxyNewCallable", params: &[Opaque, DynamicObject], ret: Proxy, imp: unavailable },
        NativeFn { name: "proxyApply", params: &[Proxy, Opaque, Array], ret: Opaque, imp: unavailable },
        // NUCLEUS #6: `Object.getOwnPropertySymbols(obj)` — the object's own
        // symbol keys as an array of `Any` symbol values (empty for a typed
        // object). Touches the managed heap, intercepted by name. Appended last
        // so existing dispatch ids stay stable.
        NativeFn { name: "objectGetOwnPropertySymbols", params: &[Opaque], ret: Array, imp: unavailable },
        // NUCLEUS #5: `Number(x)` coercion (ECMAScript ToNumber). The argument
        // arrives as an immediate `Any` (`Opaque`); an object runs
        // OrdinaryToPrimitive(hint "number") via `[Symbol.toPrimitive]`.
        // Appended last so existing dispatch ids stay stable.
        NativeFn { name: "numberCoerce", params: &[Opaque], ret: F64, imp: unavailable },
        // NUCLEUS #5: `str.match/replace/search/split(arg)` where `arg` is a
        // dynamic value. `(receiver, arg, extra, kind)` with kind 0=match,
        // 1=replace, 2=search, 3=split; `extra` is the replacement / limit.
        // Invokes `arg[Symbol.<kind>](receiver, extra?)` when present, else falls
        // back to a string-path result. Appended last so ids stay stable.
        NativeFn { name: "stringSymbolDispatch", params: &[Str, Opaque, Opaque, I64], ret: Opaque, imp: unavailable },
        // NUCLEUS #5: ECMAScript `+` on two `Any` operands: ToPrimitive(default)
        // on each, then string concatenation if either is a string, else numeric
        // addition. Appended last so ids stay stable.
        NativeFn { name: "addAny", params: &[Opaque, Opaque], ret: Opaque, imp: unavailable },
        // `decoratorAddInitializer(fn)` queues a class-decorator `addInitializer`
        // callback; `decoratorRunInitializers()` drains and runs the queue in
        // registration order (each callback receives `null` as its target).
        // Backs the typed `DecoratorContext` (builtin/decorators.zun). Appended
        // last so existing dispatch ids stay stable.
        NativeFn { name: "decoratorAddInitializer", params: &[Function], ret: Void, imp: unavailable },
        NativeFn { name: "decoratorRunInitializers", params: &[], ret: Void, imp: unavailable },
        // `Reflect.construct` / Proxy `construct` trap, CLASS targets only.
        // A user class used as a value lowers to a class descriptor `Any`
        // (token = `USER_CLASS_FLAG | typedef rid`). `classBox(descriptor)` wraps
        // it in a GC record (key `__classDesc`) so it can be a proxy target;
        // `reflectConstruct(descriptor, argsArray)` allocates an instance with
        // the array elements as the positional field values (the same shape
        // `new C(...)` lowers to); `proxyConstruct(proxy, argsArray)` routes
        // `new proxy(...)` through the handler's `construct` trap, defaulting to
        // `reflectConstruct` on the wrapped class. LIMITATION: arbitrary-function
        // constructors are out of scope (Zun has no function.prototype/`this`
        // construction model), and `newTarget` is approximated by the target.
        NativeFn { name: "classBox", params: &[Opaque], ret: DynamicObject, imp: unavailable },
        NativeFn { name: "reflectConstruct", params: &[Opaque, Array], ret: Opaque, imp: unavailable },
        NativeFn { name: "proxyConstruct", params: &[Proxy, Array], ret: Opaque, imp: unavailable },
        // `classSetMethod(classRid, name, fn)` registers a typed class's
        // `valueOf`/`toString` instance method by name so the interpreter floor
        // can invoke it (with `this`) from OrdinaryToPrimitive.
        // `classSetFieldDefault(classRid, slot, value)` records a literal field
        // initializer so runtime construction (`Reflect.construct`, proxy
        // `construct`) can fill trailing absent args.
        NativeFn { name: "classSetMethod", params: &[U32, Str, Function], ret: Void, imp: unavailable },
        NativeFn { name: "classSetFieldDefault", params: &[U32, U32, Opaque], ret: Void, imp: unavailable },
        // `functionConstruct(argsArray)`: `new Function(p1, ..., body)` / `Function(...)`.
        // All elements are strings (last = body). Handled by the interpreter via an
        // injected `EvalHost`; with no host it throws "code generation from strings
        // disallowed". Appended last so existing dispatch ids stay stable.
        NativeFn { name: "functionConstruct", params: &[Array], ret: Function, imp: unavailable },
        // NUCLEUS #25 decorator residuals. `classReplaceMethod(classRid, name, fn)`
        // overwrites a decorated method's `class_named_methods` entry with the
        // function a method decorator returned; `classBoundMethod(receiver,
        // declClassRid, name)` resolves the receiver's runtime-class entry
        // (walking to the declaring class) and binds the receiver as `this`, so a
        // decorated-method call site is an indirect call; `classMethodBox(classRid,
        // name)` hands the current entry to a decorator as its `value` (a
        // `functionBox` record). `classSetFieldTransform` stores a field
        // decorator's returned `(initial) => value`; `classInstanceInit(obj)` runs
        // field transforms and instance `addInitializer` callbacks on a new
        // instance and returns it; `decoratorAddInstanceInitializer`,
        // `decoratorAccessGet` and `decoratorAccessSet` back the typed
        // `DecoratorContext` (`builtin/decorators.zun`). Appended last so existing
        // dispatch ids stay stable.
        NativeFn { name: "classReplaceMethod", params: &[U32, Str, Function], ret: Void, imp: unavailable },
        NativeFn { name: "classBoundMethod", params: &[Opaque, U32, Str], ret: Function, imp: unavailable },
        NativeFn { name: "classMethodBox", params: &[U32, Str], ret: Opaque, imp: unavailable },
        NativeFn { name: "classSetFieldTransform", params: &[U32, U32, Function], ret: Void, imp: unavailable },
        NativeFn { name: "classInstanceInit", params: &[Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "decoratorAddInstanceInitializer", params: &[F64, Function], ret: Void, imp: unavailable },
        NativeFn { name: "decoratorAccessGet", params: &[Opaque, F64, F64, Str], ret: Opaque, imp: unavailable },
        NativeFn { name: "decoratorAccessSet", params: &[Opaque, F64, F64, Str, Opaque], ret: Void, imp: unavailable },
        // `functionBoxIs(box, fn)` — whether a `functionBox` record wraps exactly
        // the function `fn` (handle identity). Function values have no `==`/`is`
        // of their own, so this is how a stored boxed listener is matched against
        // a function value (`EventTarget.removeEventListener`). Appended last so
        // existing dispatch ids stay stable.
        NativeFn { name: "functionBoxIs", params: &[Opaque, Function], ret: Bool, imp: unavailable },
        // `globalGet(name)` — read a module-level binding previously registered
        // with `globalSet` from the realm global singleton; `undefined` when the
        // name is absent. Backs top-level `let`/`const` visibility inside
        // functions. Appended last so existing dispatch ids stay stable.
        NativeFn { name: "globalGet", params: &[Str], ret: Opaque, imp: unavailable },
        // Dynamic member calls with three and four arguments (the 0/1/2 forms
        // are defined earlier). Appended last so existing dispatch ids stay
        // stable.
        NativeFn { name: "dynamicCall3", params: &[Opaque, Str, Opaque, Opaque, Opaque], ret: Opaque, imp: unavailable },
        NativeFn { name: "dynamicCall4", params: &[Opaque, Str, Opaque, Opaque, Opaque, Opaque], ret: Opaque, imp: unavailable },
        // `stringMethod(receiver, name, a0, a1)` — a `String.prototype` method with
        // full JS argument coercion (optional / `undefined` / fractional /
        // non-number arguments). Appended last so existing dispatch ids stay stable.
        NativeFn { name: "stringMethod", params: &[Str, Str, Opaque, Opaque], ret: Opaque, imp: unavailable },
        // `replaceFn(search, input, fn, all)` — replace-with-function for a RegExp
        // or string search.
        NativeFn { name: "replaceFn", params: &[Opaque, Str, Function, Bool], ret: Str, imp: unavailable },
        // ECMAScript IsLooselyEqual (`==`) on two `any` operands.
        NativeFn { name: "looseEquals", params: &[Opaque, Opaque], ret: Bool, imp: unavailable },
        // JS-chaining forms: `Map.prototype.set` / `Set.prototype.add` return the
        // receiver. Appended last so existing dispatch ids stay stable.
        NativeFn { name: "mapSetChain", params: &[Map, Opaque, Opaque], ret: Map, imp: unavailable },
        NativeFn { name: "setAddChain", params: &[Set, Opaque], ret: Set, imp: unavailable },
        // `Reflect.set(target, key, value)` — stores respecting writability and
        // extensibility, returning a boolean success (never the value, unlike
        // `target[key] = value`). Appended last so existing dispatch ids stay stable.
        NativeFn { name: "reflectSet", params: &[Opaque, Opaque, Opaque], ret: Bool, imp: unavailable },
        // ToNumber that throws a TypeError for BigInt and Symbol (the `Math.*`
        // argument coercion), unlike `numberCoerce` (`Number(x)`).
        NativeFn { name: "numberCoerceStrict", params: &[Opaque], ret: F64, imp: unavailable },
        // `Math.<name>.apply(null, array)` — `(name, array)`; ToNumber (strict) on
        // each element, then the variadic/fixed-arity Math function.
        NativeFn { name: "bigIntUshr", params: &[BigInt, BigInt], ret: BigInt, imp: unavailable },
        NativeFn { name: "mathApply", params: &[Str, Opaque], ret: F64, imp: unavailable },
    ],
};

/// High bit of a class-descriptor token marking a *user* class (the low bits
/// are the TypeDef rid); realm base-type tokens never set it.
pub const USER_CLASS_FLAG: u32 = 0x8000_0000;
