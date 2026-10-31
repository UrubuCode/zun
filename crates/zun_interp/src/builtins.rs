//! Name resolution for native builtins implemented by managed runtime floors.

use super::native::{from_native_value, to_native_arg};
use super::stack::pop_n;
use super::{Interp, RunError, Value};

thread_local! {
    static MANAGED_BUILTIN_CACHE: std::cell::RefCell<std::collections::HashMap<u32, Option<&'static str>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Memoized classification of a builtin id as an interpreter-floor ("managed")
/// native. The underlying lookups scan name tables, so doing them per call
/// would dominate dynamic-heavy code.
fn managed_builtin_name(id: u32) -> Option<&'static str> {
    if let Some(hit) = MANAGED_BUILTIN_CACHE.with(|cache| cache.borrow().get(&id).copied()) {
        return hit;
    }
    let name = managed_collection_builtin(id)
        .or_else(|| managed_buffer_builtin(id))
        .or_else(|| managed_node_buffer_builtin(id))
        .or_else(|| managed_typeinfo_builtin(id))
        .or_else(|| managed_error_builtin(id))
        .or_else(|| managed_timers_builtin(id))
        .or_else(|| managed_array_ext_builtin(id))
        .or_else(|| managed_number_builtin(id));
    MANAGED_BUILTIN_CACHE.with(|cache| cache.borrow_mut().insert(id, name));
    name
}

pub(super) fn call_builtin(
    interp: &mut Interp<'_>,
    id: i32,
    stack: &mut Vec<Value>,
) -> Result<(), RunError> {
    let id = id as u32;
    let args = pop_n(stack, zun_natives::param_count(id))?;
    if let Some(name) = managed_builtin_name(id) {
        stack.push(interp.call_managed_collection(name, &args)?);
        return Ok(());
    }
    let native_args: Vec<zun_natives::NativeArg> = args.iter().map(to_native_arg).collect();
    let result = zun_natives::dispatch(id, &native_args, interp.output)
        .map_err(|e| native_error_to_run_error(interp, e.0))?;
    stack.push(from_native_value(result, id));
    Ok(())
}

/// A native failure whose message starts with `RangeError: ` or `TypeError: `
/// is thrown as a managed `Error` subclass instance (so `catch (e)` sees the
/// right `name` and `message`); any other message stays an interpreter error.
fn native_error_to_run_error(interp: &mut Interp<'_>, message: String) -> RunError {
    for class in ["RangeError", "TypeError", "ReferenceError"] {
        let Some(text) = message.strip_prefix(class).and_then(|rest| rest.strip_prefix(": ")) else {
            continue;
        };
        if let Some(object) = interp.new_error_object(class, text) {
            return RunError::Thrown(Value::Object(object));
        }
    }
    RunError::Native(message)
}

impl Interp<'_> {
    /// Allocate an instance of the Error-family class `class` (`message`,
    /// `stackFrames`, `cause` layout) without running its constructor.
    pub(crate) fn new_error_object(&mut self, class: &str, message: &str) -> Option<zun_gc::Obj> {
        let rid = self
            .module
            .tables
            .type_defs
            .iter()
            .position(|ty| self.module.strings.get(ty.name) == class)?
            + 1;
        let token = zun_metadata::Token::new(zun_metadata::TableKind::TypeDef, rid as u32);
        let (field_start, field_count) = self.type_field_range(token).ok()?;
        let kinds: Vec<u8> = (0..field_count as u32)
            .map(|i| self.field_kind_at(field_start + i))
            .collect::<Result<_, _>>()
            .ok()?;
        let values = [
            Value::Str(message.into()),
            Value::Str("".into()),
            Value::Void,
            Value::Str("".into()),
        ];
        if field_count != values.len() {
            return None;
        }
        Some(zun_gc::with_current_heap_mut(|heap| {
            let obj = heap.alloc(rid as u32, field_count);
            for (index, value) in values.iter().enumerate() {
                super::value::store_field(kinds[index], obj, index, value, heap);
            }
            obj
        }))
    }
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
        "regexpFlags",
        "regexpFlag",
        "finalizationRegistryNew",
        "finalizationRegister",
        "finalizationUnregister",
        "reflectHas",
        "forInHas",
        "generatorSent",
        "generatorReturnValue",
        "setCallArgc",
        "argumentsObject",
        "arrayMakeHole",
        "arrayPushHole",
        "arrayHas",
        "reflectDelete",
        "reflectSet",
        "reflectDefineProperty",
        "deleteStrict",
        "objectPrototype",
        "objectJsonStringify",
        "proxyNew",
        "proxyGet",
        "proxySet",
        "proxyHas",
        "proxyDelete",
        "dynamicSetFunction",
        "dynamicCall0",
        "dynamicCall1",
        "dynamicCall2",
        "dynamicCall3",
        "dynamicCall4",
        "dynamicCall5",
        "dynamicCall6",
        "stringMethod",
        "replaceFn",
        "dynamicConstruct",
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
        "objectForInKeys",
        "objectValues",
        "objectEntries",
        "objectAssign",
        "objectFreeze",
        "objectIsFrozen",
        "objectCreate",
        "objectGetPrototypeOf",
        "objectSetPrototypeOf",
        "objectFromEntries",
        "objectGroupBy",
        "templateObject",
        "objectIs",
        "objectHasOwn",
        "objectIsPrototypeOf",
        "functionApply",
        "functionCallSpread",
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
        "globalGet",
        "globalRead",
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
        "functionBoxIs",
        "mapSetChain",
        "setAddChain",
        "stringCoerce",
        "stringCoerceStrict",
        "applyDecorator",
        "classSetSymbolFunction",
        "serialize",
        "deserialize",
        "proxyNewCallable",
        "proxyApply",
        "objectGetOwnPropertySymbols",
        "reflectOwnKeys",
        "numberCoerce",
        "addAny",
        "decoratorAddInitializer",
        "decoratorRunInitializers",
        "classBox",
        "reflectConstruct",
        "proxyConstruct",
        "classSetMethod",
        "classSetFieldDefault",
        "functionConstruct",
        "classReplaceMethod",
        "classBoundMethod",
        "classMethodBox",
        "classSetFieldTransform",
        "classInstanceInit",
        "decoratorAddInstanceInitializer",
        "decoratorAccessGet",
        "decoratorAccessSet",
        "looseEquals",
        "numberCoerceStrict",
        "mathApply",
        "bigIntUshr",
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

/// Recognize the `Number.prototype` formatters whose argument needs ToNumber
/// coercion (which can run user `valueOf` hooks) and whose range errors must
/// be thrown as managed `RangeError` instances.
pub(crate) fn managed_number_builtin(id: u32) -> Option<&'static str> {
    for name in [
        "numberToFixedJs",
        "numberToExponentialJs",
        "numberToPrecisionJs",
        "numberToStringRadixJs",
    ] {
        if zun_natives::resolve("number", name).map(|(candidate, _)| candidate) == Some(id) {
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
        "arrayDestructure",
        "arrayFromMapped",
        "arraySpreadAppend",
        "arraySlice",
        "coalesceUndefined",
        "anyIsUndefined",
        "objectRest",
        "arrayToSorted",
        "arrayToSortedBy",
        "arrayToReversed",
        "arrayWith",
        "arrayToSpliced",
        "arrayGroup",
        "arrayGroupToMap",
        "arrayConcat",
        "arraySliceRange",
        "arrayIndexOf",
        "arrayLastIndexOf",
        "arrayIncludes",
        "arrayReverse",
    ] {
        if zun_natives::resolve("array", name).map(|(candidate, _)| candidate) == Some(id) {
            return Some(name);
        }
    }
    None
}

pub(crate) fn managed_typeinfo_builtin(id: u32) -> Option<&'static str> {
    for name in ["typeofAny", "objectToStringTag", "instanceofClass", "instanceofDynamic"] {
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
        "generatorIteratorClose",
        "symbolIteratorCall",
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
