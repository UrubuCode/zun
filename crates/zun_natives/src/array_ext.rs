use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError(
        "managed array intrinsic requires an interpreter heap".into(),
    ))
}

/// Extended `Array.prototype` / `Array.*` surface (NUCLEUS #7). Appended as a
/// dedicated module placed LAST in the registry so every existing dispatch id
/// stays stable. Each function re-enters the interpreter or touches the managed
/// heap, so the implementations live in `zun_interp` (intercepted by name); the
/// signatures here fix only the argument count the verifier pops. The module
/// name is `array`, matching the `zun:core` namespace the `collections` module's
/// `array*` natives already resolve under.
pub const MODULE: crate::Module = crate::Module {
    name: "array",
    functions: &[
        // `at(index)` with ECMAScript relative indexing: a negative index counts
        // from the end, and an out-of-range index yields `undefined`.
        NativeFn { name: "arrayAt", params: &[Array, F64], ret: Opaque, imp: unavailable },
        // `findLast(cb)` / `findLastIndex(cb)` — reverse iteration, callback
        // `(value, index, array)`.
        NativeFn { name: "arrayFindLast", params: &[Array, Function], ret: Opaque, imp: unavailable },
        NativeFn { name: "arrayFindLastIndex", params: &[Array, Function], ret: I32, imp: unavailable },
        // `flat(depth=1)` — recursively flattens nested arrays up to `depth`
        // levels; non-array elements pass through unchanged.
        NativeFn { name: "arrayFlat", params: &[Array, F64], ret: Array, imp: unavailable },
        // `flatMap(cb)` — `map(cb)` then a single flatten level.
        NativeFn { name: "arrayFlatMap", params: &[Array, Function], ret: Array, imp: unavailable },
        // Iterator helpers. For this floor each returns a fresh managed Array
        // (which composes with `for...of` and indexing); `entries` yields
        // `[index, value]` pairs.
        NativeFn { name: "arrayKeys", params: &[Array], ret: Array, imp: unavailable },
        NativeFn { name: "arrayValues", params: &[Array], ret: Array, imp: unavailable },
        NativeFn { name: "arrayEntries", params: &[Array], ret: Array, imp: unavailable },
        // `splice(start, deleteCount, ...items)` — deletes and inserts the items
        // (gathered into a managed Array by the lowering) at `start`, returning
        // the removed elements.
        NativeFn { name: "arraySpliceInsert", params: &[Array, U64, U64, Array], ret: Array, imp: unavailable },
        // No-initial-value `reduce`/`reduceRight`: seed from the first/last
        // element and throw a TypeError on an empty array (spec).
        NativeFn { name: "arrayReduceNoInit", params: &[Array, Function], ret: Opaque, imp: unavailable },
        NativeFn { name: "arrayReduceRightNoInit", params: &[Array, Function], ret: Opaque, imp: unavailable },
        // `Array.isArray(x)`.
        NativeFn { name: "arrayIsArray", params: &[Opaque], ret: Bool, imp: unavailable },
        // `Array.from(arrayLike_or_iterable, mapFn?)`. v1 accepts a managed Array
        // or a string; other iterables are deferred. The mapped form invokes the
        // callback with `(value, index)`.
        NativeFn { name: "arrayFrom", params: &[Opaque], ret: Array, imp: unavailable },
        NativeFn { name: "arrayFromMapped", params: &[Opaque, Function], ret: Array, imp: unavailable },
        // `arraySpreadAppend(target, source)` appends every element of the
        // iterable `source` (a managed Array or a string, matching `arrayFrom`'s
        // supported sources) to `target`, returning `target`. Backs array-literal
        // spread (`[a, ...xs]`). Appended last so existing dispatch ids stay
        // stable.
        NativeFn { name: "arraySpreadAppend", params: &[Array, Opaque], ret: Array, imp: unavailable },
        // `arraySlice(source, start)` returns a fresh managed Array holding the
        // elements of `source` from index `start` to the end. Backs the
        // `...rest` element of array destructuring. Appended last so existing
        // dispatch ids stay stable.
        NativeFn { name: "arraySlice", params: &[Array, U64], ret: Array, imp: unavailable },
        // `coalesceUndefined(value, fallback)` returns `fallback` when `value`
        // reads as `undefined`, else `value`. Backs destructuring defaults
        // (`[a = 1]`, `{ x = 1 }`); both operands are evaluated (eager default).
        NativeFn { name: "coalesceUndefined", params: &[Opaque, Opaque], ret: Opaque, imp: unavailable },
        // `objectRest(source, excludedKeys)` returns a fresh dynamic object
        // holding every own-enumerable string key of `source` except those
        // named in the `excludedKeys` Array. Backs object-pattern `...rest`.
        NativeFn { name: "objectRest", params: &[Opaque, Array], ret: DynamicObject, imp: unavailable },
        // ES2023 change-array-by-copy methods (NUCLEUS #7 remaining). Each
        // returns a FRESH managed Array and leaves the receiver untouched.
        // Appended last so existing dispatch ids stay stable.
        //
        // `toSorted(cmp?)` — a sorted copy (default string order, or the
        // comparator); `toReversed()` — a reversed copy. The comparator
        // re-enters the interpreter like `sort`.
        NativeFn { name: "arrayToSorted", params: &[Array], ret: Array, imp: unavailable },
        NativeFn { name: "arrayToSortedBy", params: &[Array, Function], ret: Array, imp: unavailable },
        NativeFn { name: "arrayToReversed", params: &[Array], ret: Array, imp: unavailable },
        // `with(index, value)` — a copy with `index` replaced; a negative
        // index counts from the end, and an out-of-range index throws a
        // RangeError-equivalent.
        NativeFn { name: "arrayWith", params: &[Array, I64, Opaque], ret: Array, imp: unavailable },
        // `toSpliced(start, deleteCount, ...items)` — a copy with the splice
        // applied; the variadic inserts are gathered into a managed Array.
        NativeFn { name: "arrayToSpliced", params: &[Array, U64, U64, Array], ret: Array, imp: unavailable },
        // `group(cb)` / `groupToMap(cb)` (array grouping proposal). `group`
        // returns a dynamic object keyed by `String(cb(v, i, arr))`;
        // `groupToMap` returns a Map keyed by the callback result. Values are
        // arrays of the grouped elements.
        NativeFn { name: "arrayGroup", params: &[Array, Function], ret: DynamicObject, imp: unavailable },
        NativeFn { name: "arrayGroupToMap", params: &[Array, Function], ret: Map, imp: unavailable },
        // `concat(...items)` - the receiver followed by each gathered item;
        // arrays (and objects whose `[Symbol.isConcatSpreadable]` is `true`)
        // are spread, anything else is appended as one element.
        NativeFn { name: "arrayConcat", params: &[Array, Array], ret: Array, imp: unavailable },
    ],
};
