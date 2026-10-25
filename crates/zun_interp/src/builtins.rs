//! Name resolution for native builtins implemented by managed runtime floors.

use super::native::{from_native_value, to_native_arg};
use super::stack::pop_n;
use super::{Interp, RunError, Value};

pub(super) fn call_builtin(
    interp: &mut Interp<'_>,
    id: i32,
    stack: &mut Vec<Value>,
) -> Result<(), RunError> {
    let id = id as u32;
    let args = pop_n(stack, zun_natives::param_count(id))?;
    if let Some(name) = managed_collection_builtin(id)
        .or_else(|| managed_buffer_builtin(id))
        .or_else(|| managed_node_buffer_builtin(id))
        .or_else(|| managed_typeinfo_builtin(id))
        .or_else(|| managed_error_builtin(id))
        .or_else(|| managed_timers_builtin(id))
        .or_else(|| managed_array_ext_builtin(id))
    {
        stack.push(interp.call_managed_collection(name, &args)?);
        return Ok(());
    }
    let native_args: Vec<zun_natives::NativeArg> = args.iter().map(to_native_arg).collect();
    let result = zun_natives::dispatch(id, &native_args, interp.output)
        .map_err(|e| RunError::Native(e.0))?;
    stack.push(from_native_value(result, id));
    Ok(())
}

pub(crate) fn managed_collection_builtin(id: u32) -> Option<&'static str> {
    for name in [
        "bufferNew",
        "bufferLength",
        "bufferSlice",
        "bufferGetU8",
        "bufferSetU8",
        "mapNew",
        "mapSet",
        "mapGet",
        "mapHas",
        "mapSize",
        "mapDelete",
        "mapClear",
        "mapValues",
        "mapKeys",
        "mapEntries",
        "mapForEach",
        "mapFrom",
        "mapFromIterable",
        "setFromIterable",
        "setForEach",
        "setFrom",
        "mapHasF64",
        "mapGetF64Or",
        "mapSetF64",
        "mapDeleteF64",
        "mapHasI64",
        "mapGetI64Or",
        "mapSetI64",
        "mapDeleteI64",
        "mapHasI32",
        "mapGetI32Or",
        "mapSetI32",
        "mapDeleteI32",
        "setNew",
        "setAdd",
        "setHas",
        "setSize",
        "setDelete",
        "setClear",
        "setValues",
        "setEntries",
        "setHasF64",
        "setAddF64",
        "setDeleteF64",
        "setHasI64",
        "setAddI64",
        "setDeleteI64",
        "setHasI32",
        "setAddI32",
        "setDeleteI32",
        "weakMapNew",
        "weakMapSet",
        "weakMapGet",
        "weakMapHas",
        "weakMapDelete",
        "weakSetNew",
        "weakSetAdd",
        "weakSetHas",
        "weakSetDelete",
        "weakRefNew",
        "weakRefDeref",
        "regexpNew",
        "bigIntNew",
        "bigIntAdd",
        "bigIntSub",
        "bigIntMul",
        "bigIntDiv",
        "bigIntRem",
        "bigIntNeg",
        "bigIntCompare",
        "bigIntEq",
        "bigIntNe",
        "bigIntLt",
        "bigIntLe",
        "bigIntGt",
        "bigIntGe",
        "bigIntToString",
        "bigIntFromNumber",
        "bigIntPow",
        "bigIntAnd",
        "bigIntOr",
        "bigIntXor",
        "bigIntNot",
        "bigIntShl",
        "bigIntShr",
        "bigIntAsIntN",
        "bigIntAsUintN",
        "regexpIsMatch",
        "regexpExec",
        "regexpSearch",
        "regexpSplit",
        "regexpReplace",
        "regexpSource",
        "finalizationRegistryNew",
        "finalizationRegister",
        "finalizationUnregister",
        "reflectHas",
        "reflectDelete",
        "proxyNew",
        "proxyGet",
        "proxySet",
        "proxyHas",
        "proxyDelete",
        "dynamicSetFunction",
        "dynamicCall0",
        "dynamicCall1",
        "dynamicCall2",
        "promiseNew",
        "promiseWithResolvers",
        "promiseResolve",
        "promiseReject",
        "promiseState",
        "promiseResult",
        "promiseIsReady",
        "promiseThen",
        "promiseCatch",
        "promiseFinally",
        "promiseResolveStatic",
        "promiseRejectStatic",
        "promiseAll",
        "promiseAllSettled",
        "promiseRace",
        "promiseAny",
        "promiseNewExecutor",
        "objectKeys",
        "objectValues",
        "objectEntries",
        "objectAssign",
        "objectFreeze",
        "objectIsFrozen",
        "objectCreate",
        "objectGetPrototypeOf",
        "objectSetPrototypeOf",
        "objectFromEntries",
        "objectIs",
        "objectHasOwn",
        "objectIsPrototypeOf",
        "functionApply",
        "functionBind",
        "arrayForEach",
        "arrayMap",
        "arrayFilter",
        "arrayFind",
        "arrayFindIndex",
        "arraySome",
        "arrayEvery",
        "arrayReduce",
        "arrayReduceRight",
        "arraySortBy",
        "arraySortDefault",
        "arrayJoin",
        "arrayShift",
        "arrayUnshift",
        "arraySplice",
        "globalSet",
        "realmDescriptor",
        "classDescriptorToken",
        "symbolNew",
        "symbolFor",
        "symbolKeyFor",
        "symbolWellKnown",
        "symbolDescription",
        "stringSplit",
        "dynamicSetSymbolFunction",
        "stringSymbolDispatch",
        "objectDefineProperty",
        "objectDefineProperties",
        "objectGetOwnPropertyDescriptor",
        "objectGetOwnPropertyDescriptors",
        "objectGetOwnPropertyNames",
        "objectSeal",
        "objectIsSealed",
        "objectPreventExtensions",
        "objectIsExtensible",
        "objectPropertyIsEnumerable",
        "stringMatchAll",
        "functionBox",
        "stringCoerce",
        "applyDecorator",
        "classSetSymbolFunction",
        "serialize",
        "deserialize",
        "proxyNewCallable",
        "proxyApply",
        "objectGetOwnPropertySymbols",
        "numberCoerce",
        "addAny",
    ] {
        if zun_natives::resolve("collections", name)
            .or_else(|| zun_natives::resolve("promise", name))
            .or_else(|| zun_natives::resolve("serde", name))
            .map(|(candidate, _)| candidate)
            == Some(id)
        {
            return Some(name);
        }
    }
    None
}

/// Recognize the Node `Buffer` intrinsics (the `nodeBuffer` namespace),
/// serviced on the interpreter's managed byte-view floor.
pub(crate) fn managed_node_buffer_builtin(id: u32) -> Option<&'static str> {
    for name in [
        "nodeBufferFromString",
        "nodeBufferFromArray",
        "nodeBufferFromArrayBuffer",
        "nodeBufferAlloc",
        "nodeBufferAllocUnsafe",
        "nodeBufferConcat",
        "nodeBufferIsBuffer",
        "nodeBufferByteLength",
        "nodeBufferToString",
        "nodeBufferWrite",
        "nodeBufferSlice",
        "nodeBufferFill",
        "nodeBufferEquals",
        "nodeBufferCopy",
        "nodeBufferReadUInt8",
        "nodeBufferWriteUInt8",
        "nodeBufferReadInt8",
        "nodeBufferWriteInt8",
        "nodeBufferReadUInt16LE",
        "nodeBufferWriteUInt16LE",
        "nodeBufferReadUInt16BE",
        "nodeBufferWriteUInt16BE",
        "nodeBufferReadInt16LE",
        "nodeBufferWriteInt16LE",
        "nodeBufferReadInt16BE",
        "nodeBufferWriteInt16BE",
        "nodeBufferReadUInt32LE",
        "nodeBufferWriteUInt32LE",
        "nodeBufferReadUInt32BE",
        "nodeBufferWriteUInt32BE",
        "nodeBufferReadInt32LE",
        "nodeBufferWriteInt32LE",
        "nodeBufferReadInt32BE",
        "nodeBufferWriteInt32BE",
        "nodeBufferReadBigInt64LE",
        "nodeBufferWriteBigInt64LE",
        "nodeBufferReadBigInt64BE",
        "nodeBufferWriteBigInt64BE",
        "nodeBufferReadBigUInt64LE",
        "nodeBufferWriteBigUInt64LE",
        "nodeBufferReadBigUInt64BE",
        "nodeBufferWriteBigUInt64BE",
        // NUCLEUS #21 gap closure.
        "nodeBufferReadFloatLE",
        "nodeBufferReadFloatBE",
        "nodeBufferWriteFloatLE",
        "nodeBufferWriteFloatBE",
        "nodeBufferReadDoubleLE",
        "nodeBufferReadDoubleBE",
        "nodeBufferWriteDoubleLE",
        "nodeBufferWriteDoubleBE",
        "nodeBufferReadUIntLE",
        "nodeBufferReadUIntBE",
        "nodeBufferReadIntLE",
        "nodeBufferReadIntBE",
        "nodeBufferWriteUIntLE",
        "nodeBufferWriteUIntBE",
        "nodeBufferWriteIntLE",
        "nodeBufferWriteIntBE",
        "nodeBufferSwap16",
        "nodeBufferSwap32",
        "nodeBufferSwap64",
        "nodeBufferIndexOf",
        "nodeBufferLastIndexOf",
        "nodeBufferIncludes",
    ] {
        if zun_natives::resolve("nodeBuffer", name).map(|(candidate, _)| candidate) == Some(id) {
            return Some(name);
        }
    }
    None
}

/// Recognize the `timers` macrotask intrinsics (ASYNC slice 4), serviced on the
/// interpreter floor because each needs the scheduler, timer heap and heap.
pub(crate) fn managed_timers_builtin(id: u32) -> Option<&'static str> {
    for name in [
        "setTimeout",
        "setInterval",
        "clearTimeout",
        "clearInterval",
        "queueMicrotask",
        "setImmediate",
    ] {
        if zun_natives::resolve("timers", name).map(|(candidate, _)| candidate) == Some(id) {
            return Some(name);
        }
    }
    None
}

/// Recognize the extended Array surface (NUCLEUS #7), serviced on the
/// interpreter floor because each re-enters the interpreter or touches the
/// managed heap.
pub(crate) fn managed_array_ext_builtin(id: u32) -> Option<&'static str> {
    for name in [
        "arrayAt",
        "arrayFindLast",
        "arrayFindLastIndex",
        "arrayFlat",
        "arrayFlatMap",
        "arrayKeys",
        "arrayValues",
        "arrayEntries",
        "arraySpliceInsert",
        "arrayReduceNoInit",
        "arrayReduceRightNoInit",
        "arrayIsArray",
        "arrayFrom",
        "arrayFromMapped",
        "arraySpreadAppend",
        "arraySlice",
        "coalesceUndefined",
        "objectRest",
        "arrayToSorted",
        "arrayToSortedBy",
        "arrayToReversed",
        "arrayWith",
        "arrayToSpliced",
        "arrayGroup",
        "arrayGroupToMap",
        "arrayConcat",
    ] {
        if zun_natives::resolve("array", name).map(|(candidate, _)| candidate) == Some(id) {
            return Some(name);
        }
    }
    None
}

pub(crate) fn managed_typeinfo_builtin(id: u32) -> Option<&'static str> {
    for name in ["typeofAny", "instanceofClass", "instanceofDynamic"] {
        if zun_natives::resolve("typeinfo", name).map(|(candidate, _)| candidate) == Some(id) {
            return Some(name);
        }
    }
    None
}

/// Recognize the `zun:error` intrinsics the interpreter services directly on
/// its managed floor (the live frame-stack capture behind `Error.stack`).
pub(crate) fn managed_error_builtin(id: u32) -> Option<&'static str> {
    for name in ["captureStackTrace"] {
        if zun_natives::resolve("error", name).map(|(candidate, _)| candidate) == Some(id) {
            return Some(name);
        }
    }
    None
}

pub(crate) fn managed_buffer_builtin(id: u32) -> Option<&'static str> {
    for name in [
        "bufferNew",
        "bufferLength",
        "bufferSlice",
        "bufferGetU8",
        "bufferSetU8",
        "bufferReadU32",
        "bufferWriteU32",
        "bufferReadF64",
        "bufferWriteF64",
        "dataViewNew",
        "dataViewLength",
        "dataViewByteLength",
        "dataViewByteOffset",
        "dataViewBuffer",
        "isView",
        "dataViewGetU32",
        "dataViewSetU32",
        "dataViewGetInt8",
        "dataViewSetInt8",
        "dataViewGetUint8",
        "dataViewSetUint8",
        "dataViewGetInt16",
        "dataViewSetInt16",
        "dataViewGetInt32",
        "dataViewSetInt32",
        "dataViewGetFloat32",
        "dataViewSetFloat32",
        "dataViewGetInt64",
        "dataViewSetInt64",
        "dataViewGetUint64",
        "dataViewSetUint64",
        "dataViewGetU16",
        "dataViewSetU16",
        "dataViewGetF64",
        "dataViewSetF64",
        "arrayIteratorNew",
        "arrayIteratorNext",
        "arrayIteratorDone",
        "setIteratorNew",
        "setIteratorNext",
        "setIteratorDone",
        "mapIteratorNew",
        "mapIteratorNext",
        "mapIteratorDone",
        "generatorIteratorNew",
        "generatorIteratorNewArgs",
        "generatorIteratorNext",
        "generatorIteratorDone",
        "stringIteratorNew",
        // Kind-agnostic second-generation typed-array surface (bulk set,
        // slice/copyWithin, functional methods, from/of). One native each;
        // the element kind is a runtime property or a discriminant argument.
        "typedArraySetFrom",
        "typedArraySlice",
        "typedArrayCopyWithin",
        "typedArrayIndexOf",
        "typedArrayIncludes",
        "typedArrayJoin",
        "typedArrayForEach",
        "typedArrayMap",
        "typedArrayFilter",
        "typedArrayReduce",
        "typedArrayFind",
        "typedArrayFindIndex",
        "typedArraySome",
        "typedArrayEvery",
        "typedArrayFrom",
        "typedArrayFromMapped",
        "typedArrayOf",
        // ES2024 resizable ArrayBuffer.
        "bufferResizableNew",
        "bufferResize",
        "bufferMaxByteLength",
        "bufferResizable",
        "typedArrayTrackingNew",
        "typedArrayReduceNoInit",
        "typedArrayReduceRight",
        "typedArrayReduceRightNoInit",
        "bufferTransfer",
        "bufferTransferFixed",
        "bufferDetached",
        "dataViewTrackingNew",
    ]
    .into_iter()
    // The typed-array intrinsics come from the master table, so a kind
    // cannot be callable from compiled code but unknown here.
    .chain(crate::typed_arrays::NATIVES.iter().copied())
    {
        if zun_natives::resolve("buffer", name).map(|(candidate, _)| candidate) == Some(id) {
            return Some(name);
        }
    }
    None
}
