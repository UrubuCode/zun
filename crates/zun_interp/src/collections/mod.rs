//! Managed collection dispatch extracted from the interpreter core.

pub(super) use super::{Interp, RunError};
pub(super) use super::value::Value;
pub(super) use super::state::{Aggregate, CombinatorKind, ExecutionFrame, ExecutionOutcome, Reaction, ReactionKind, TimerEntry};
pub(super) use zun_sched::{Continuation, ContinuationId, Settlement, TaskId};
pub(super) use zun_module::read_body;
pub(super) use zun_il::{decode_one, Opcode};

mod iterators;
mod mapset;
mod misc_ops;
mod dynamic_call;
mod array_iterate;
mod array_mutate;
mod array_construct;

mod object_core;
mod object_descriptor;

mod regexp;
mod reflect;
mod proxy;
mod string_num;
mod function;
mod promise;

mod buffer;
mod generator;
mod weak;
mod bigint;
mod class_decorator;

/// `Promise.withResolvers` settle modes, stored in the settler record's
/// `__settleMode` slot (as a float `Any`).
pub(super) const SETTLE_RESOLVE: f64 = 0.0;
pub(super) const SETTLE_REJECT: f64 = 1.0;

/// How a reflective `Object`/`Reflect` operation on a possible proxy resolves.
/// The descriptor-dependent statics (`getOwnPropertyDescriptor`,
/// `defineProperty`, `getPrototypeOf`, `setPrototypeOf`, `isExtensible`,
/// `preventExtensions`, `ownKeys`) consult this before touching the
/// property-descriptor machinery (NUCLEUS #6): a proxy with the matching trap
/// invokes it; a proxy without it, or any non-proxy object, operates on the
/// underlying target directly.
pub(super) enum ProxyRoute {
    /// The handler defines the trap; invoke it with the target as the leading
    /// positional argument (closure environment threaded first, as with the
    /// `get`/`set`/`has`/`deleteProperty` traps).
    Trap(zun_core::FunctionHandle, zun_gc::Obj),
    /// A proxy without the trap: the operation falls back to its target.
    Target(zun_gc::Obj),
    /// Not a proxy: the operation runs on the object itself.
    NotProxy,
}

/// Resolve a value to a heap object, following an `Any` payload. Promoted from
/// the preamble closure of `call_managed_collection_inner` so extracted helper
/// submodules can share it.
pub(super) fn object(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Object(object) => Some(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
        _ => None,
    }
}

/// Box a value into a heap `Any` cell. Promoted from the preamble closure.
pub(super) fn any(value: &Value) -> Option<zun_core::Any> {
    zun_gc::with_current_heap_mut(|heap| value.box_any_cell(heap))
}

/// A valid weak key: an object (or a function, via its cell). Promoted from the
/// preamble closure.
pub(super) fn weak_key(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Function(handle) => Some(zun_gc::with_current_heap_mut(|heap| heap.function_cell(*handle))),
        // An unregistered symbol (user-created or well-known) is a legal weak key.
        Value::Any(any) if any.as_symbol().is_some() => {
            let id = any.as_symbol()?;
            if zun_gc::with_current_heap(|heap| heap.symbol_key_for(id).is_some()) {
                None
            } else {
                Some(zun_gc::with_current_heap_mut(|heap| heap.symbol_cell(id)))
            }
        }
        other => object(other).filter(|candidate| {
            zun_gc::with_current_heap(|heap| heap.bigint_value(*candidate).is_none())
        }),
    }
}

/// A Promise callback argument: a function value, or absent (the lowering
/// passes `null`/`undefined` for an omitted `then`/`catch`/`finally` slot).
/// Promoted from the preamble closure.
pub(super) fn callback_arg(value: Option<&Value>) -> Option<zun_core::FunctionHandle> {
    match value {
        Some(Value::Function(handle)) => Some(*handle),
        _ => None,
    }
}

impl<'a> Interp<'a> {
    /// Run `callback` over a Map / Set with a live cursor: entries appended
    /// while iterating are visited, deleted entries are skipped.
    fn collection_for_each(
        &mut self,
        object: zun_gc::Obj,
        callback: zun_core::FunctionHandle,
        is_set: bool,
    ) -> Result<Value, RunError> {
        let roots = [object];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        zun_gc::with_current_heap_mut(|heap| heap.collection_pin(roots[0], true));
        let mut outcome = Ok(Value::Void);
        let mut index = 0usize;
        loop {
            let Some(slot) = zun_gc::with_current_heap(|heap| heap.collection_cursor(roots[0], index)) else {
                break;
            };
            index += 1;
            let Some((key, value)) = slot else { continue };
            let receiver = Value::Any(zun_core::Any::object(roots[0].addr() as usize));
            let callback_args = if is_set {
                vec![Value::Any(key), Value::Any(key), receiver]
            } else {
                vec![Value::Any(value), Value::Any(key), receiver]
            };
            if let Err(error) = self.invoke_callback(callback, callback_args) {
                outcome = Err(error);
                break;
            }
        }
        zun_gc::with_current_heap_mut(|heap| heap.collection_pin(roots[0], false));
        outcome
    }

    pub(crate) fn call_managed_collection(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        // `recv.m?.(..)` probe: a `?`-prefixed member is called if it resolves and
        // otherwise answers `undefined`.
        if name.starts_with("dynamicCall") {
            if let Some(Value::Str(member)) = args.get(1) {
                if let Some(real) = member.strip_prefix('?') {
                    let mut stripped = args.to_vec();
                    stripped[1] = Value::Str(real.into());
                    OPTIONAL_PROBE.with(|flag| flag.set(true));
                    let outcome = self.call_managed_collection_inner(name, &stripped);
                    OPTIONAL_PROBE.with(|flag| flag.set(false));
                    return match outcome {
                        Err(RunError::BadToken) => Ok(Value::Any(zun_core::Any::void())),
                        other => other,
                    };
                }
            }
        }
        let result = self.call_managed_collection_inner(name, args);
        // A dynamic member call that resolved to nothing is a JS TypeError.
        if matches!(result, Err(RunError::BadToken)) && name.starts_with("dynamicCall") {
            if let Some(Value::Str(member)) = args.get(1) {
                let member = member.to_string();
                if let Some(object) = self.new_error_object("TypeError", &format!("{member} is not a function")) {
                    return Err(RunError::Thrown(Value::Object(object)));
                }
            }
        }
        result
    }

    fn call_managed_collection_inner(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        // The typed-array intrinsics are generated from the master table in
        // `zun_core`, so every kind the checker accepts runs on Tier 0.
        if let Some(result) = crate::typed_arrays::call(name, args) {
            return result;
        }
        if let Some(result) = crate::node_buffer::call(name, args) {
            return result;
        }
        if let Some(result) = self.call_typed_array_ext(name, args) {
            return result;
        }
        if let Some(result) = self.call_decorator_native(name, args) {
            return result;
        }
        self.guard_array_native(name, args)?;
        // Reflection over a regexp sees its `lastIndex` as a real own property.
        if matches!(
            name,
            "reflectSet" | "reflectHas" | "reflectDelete" | "reflectOwnKeys" | "reflectDefineProperty"
                | "objectDefineProperty" | "objectDefineProperties" | "objectGetOwnPropertyDescriptor"
                | "objectGetOwnPropertyDescriptors" | "objectGetOwnPropertyNames" | "objectKeys"
                | "objectValues" | "objectEntries" | "objectFreeze" | "objectIsFrozen" | "objectSeal"
                | "objectIsSealed" | "objectPreventExtensions" | "objectIsExtensible" | "objectHasOwn"
                | "deleteStrict"
        ) {
            let regex_target = match args.first() {
                Some(Value::Object(object)) => Some(*object),
                Some(Value::Any(any)) => any
                    .as_object()
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
                _ => None,
            };
            if let Some(target) = regex_target {
                zun_gc::with_current_heap_mut(|heap| {
                    if heap.regex_source(target).is_some() {
                        heap.regex_materialize_last_index(target);
                    }
                });
            }
        }
        match name {
            "bufferNew" => self.col_buffer(name, args),
            "bufferLength" => self.col_buffer(name, args),
            "bufferSlice" => self.col_buffer(name, args),
            "bufferResizableNew" => self.col_buffer(name, args),
            "bufferResize" => self.col_buffer(name, args),
            "bufferTransfer" | "bufferTransferFixed" => self.col_buffer(name, args),
            "bufferDetached" => self.col_buffer(name, args),
            "bufferMaxByteLength" => self.col_buffer(name, args),
            "bufferResizable" => self.col_buffer(name, args),
            "bufferGetU8" => self.col_buffer(name, args),
            "bufferSetU8" => self.col_buffer(name, args),
            "bufferReadU32" => self.col_buffer(name, args),
            "bufferWriteU32" => self.col_buffer(name, args),
            "bufferReadF64" => self.col_buffer(name, args),
            "bufferWriteF64" => self.col_buffer(name, args),
            "dataViewNew" => self.col_buffer(name, args),
            "dataViewTrackingNew" => self.col_buffer(name, args),
            "dataViewLength" => self.col_buffer(name, args),
            "dataViewByteLength" => self.col_buffer(name, args),
            "dataViewByteOffset" => self.col_buffer(name, args),
            "dataViewBuffer" => self.col_buffer(name, args),
            "isView" => self.col_buffer(name, args),
            "dataViewGetU32" => self.col_buffer(name, args),
            "dataViewSetU32" => self.col_buffer(name, args),
            "dataViewGetInt8" => self.col_buffer(name, args),
            "dataViewSetInt8" => self.col_buffer(name, args),
            "dataViewGetUint8" => self.col_buffer(name, args),
            "dataViewSetUint8" => self.col_buffer(name, args),
            "dataViewGetInt16" => self.col_buffer(name, args),
            "dataViewSetInt16" => self.col_buffer(name, args),
            "dataViewGetInt32" => self.col_buffer(name, args),
            "dataViewSetInt32" => self.col_buffer(name, args),
            "dataViewGetFloat32" => self.col_buffer(name, args),
            "dataViewSetFloat32" => self.col_buffer(name, args),
            "dataViewGetInt64" => self.col_buffer(name, args),
            "dataViewSetInt64" => self.col_buffer(name, args),
            "dataViewGetUint64" => self.col_buffer(name, args),
            "dataViewSetUint64" => self.col_buffer(name, args),
            "dataViewGetU16" => self.col_buffer(name, args),
            "dataViewSetU16" => self.col_buffer(name, args),
            "dataViewGetF64" => self.col_buffer(name, args),
            "dataViewSetF64" => self.col_buffer(name, args),
            "arrayIteratorNew" => self.col_iterator(name, args),
            "stringSplit" => self.col_iterator(name, args),
            "stringIteratorNew" => self.col_iterator(name, args),
            "arrayIteratorNext" => self.col_iterator(name, args),
            "arrayIteratorDone" => self.col_iterator(name, args),
            "setIteratorNew" => self.col_iterator(name, args),
            "setIteratorNext" => self.col_iterator(name, args),
            "setIteratorDone" => self.col_iterator(name, args),
            "mapIteratorNew" => self.col_iterator(name, args),
            "mapIteratorNext" => self.col_iterator(name, args),
            "mapIteratorDone" => self.col_iterator(name, args),
            "generatorIteratorNew" => self.col_generator(name, args),
            "generatorIteratorNewArgs" => self.col_generator(name, args),
            "generatorIteratorDone" => self.col_generator(name, args),
            "symbolIteratorCall" => self.col_generator(name, args),
            "generatorIteratorClose" => self.col_generator(name, args),
            "generatorIteratorNext" => self.col_generator(name, args),
            "mapNew" => self.col_mapset(name, args),
            "mapFrom" => self.col_mapset(name, args),
            "mapSet" => self.col_mapset(name, args),
            "mapSetChain" => self.col_mapset(name, args),
            "setAddChain" => self.col_mapset(name, args),
            "mapSetI32" => self.col_mapset(name, args),
            "mapSetI64" => self.col_mapset(name, args),
            "mapSetF64" => self.col_mapset(name, args),
            "mapHasI32" => self.col_mapset(name, args),
            "mapHasI64" => self.col_mapset(name, args),
            "mapHasF64" => self.col_mapset(name, args),
            "mapGetI32Or" => self.col_mapset(name, args),
            "mapGetI64Or" => self.col_mapset(name, args),
            "mapGetF64Or" => self.col_mapset(name, args),
            "mapDeleteI32" => self.col_mapset(name, args),
            "mapDeleteI64" => self.col_mapset(name, args),
            "mapDeleteF64" => self.col_mapset(name, args),
            "mapGet" => self.col_mapset(name, args),
            "optionalProbeOk" => self.col_mapset(name, args),
            "mapHas" => self.col_mapset(name, args),
            "mapSize" => self.col_mapset(name, args),
            "mapDelete" => self.col_mapset(name, args),
            "mapClear" => self.col_mapset(name, args),
            "mapValues" => self.col_mapset(name, args),
            "mapKeys" | "mapEntries" => self.col_mapset(name, args),
            "mapForEach" => self.col_mapset(name, args),
            "setNew" => self.col_mapset(name, args),
            "setFrom" => self.col_mapset(name, args),
            "setFromIterable" => self.col_mapset(name, args),
            "mapFromIterable" => self.col_mapset(name, args),
            "setAdd" => self.col_mapset(name, args),
            "setAddI32" => self.col_mapset(name, args),
            "setAddI64" => self.col_mapset(name, args),
            "setAddF64" => self.col_mapset(name, args),
            "setHasI32" => self.col_mapset(name, args),
            "setHasI64" => self.col_mapset(name, args),
            "setHasF64" => self.col_mapset(name, args),
            "setDeleteI32" => self.col_mapset(name, args),
            "setDeleteI64" => self.col_mapset(name, args),
            "setDeleteF64" => self.col_mapset(name, args),
            "setHas" => self.col_mapset(name, args),
            "setSize" => self.col_mapset(name, args),
            "setDelete" => self.col_mapset(name, args),
            "setClear" => self.col_mapset(name, args),
            "setValues" => self.col_mapset(name, args),
            "setEntries" => self.col_mapset(name, args),
            "setForEach" => self.col_mapset(name, args),
            "weakMapNew" => self.col_weak(name, args),
            "weakMapSet" => self.col_weak(name, args),
            "weakMapGet" => self.col_weak(name, args),
            "weakMapHas" => self.col_weak(name, args),
            "weakMapDelete" => self.col_weak(name, args),
            "weakSetNew" => self.col_weak(name, args),
            "weakSetAdd" => self.col_weak(name, args),
            "weakSetHas" => self.col_weak(name, args),
            "weakSetDelete" => self.col_weak(name, args),
            "weakRefNew" => self.col_weak(name, args),
            "weakRefDeref" => self.col_weak(name, args),
            "regexpNew" => self.col_regexp(name, args),
            "regexpExec" => self.col_regexp(name, args),
            "regexpTest" => self.col_regexp(name, args),
            "regexpLastIndex" => self.col_regexp(name, args),
            "regexpSetLastIndex" => self.col_regexp(name, args),
            "regexpReplaceAllGuard" => self.col_regexp(name, args),
            "regexpMatch" => self.col_regexp(name, args),
            "stringMatchAll" => self.col_regexp(name, args),
            "bigIntNew" => self.col_bigint(name, args),
            "bigIntFromNumber" => self.col_bigint(name, args),
            "bigIntUshr" => self.col_bigint(name, args),
            "bigIntPow" | "bigIntAnd" | "bigIntOr" | "bigIntXor" | "bigIntShl" | "bigIntShr" => self.col_bigint(name, args),
            "bigIntNot" => self.col_bigint(name, args),
            "bigIntAsIntN" | "bigIntAsUintN" => self.col_bigint(name, args),
            "bigIntAdd" | "bigIntSub" | "bigIntMul" | "bigIntDiv" | "bigIntRem" | "bigIntCompare"
            | "bigIntEq" | "bigIntNe" | "bigIntLt" | "bigIntLe" | "bigIntGt" | "bigIntGe" => self.col_bigint(name, args),
            "bigIntNeg" => self.col_bigint(name, args),
            "bigIntToString" => self.col_bigint(name, args),
            "regexpIsMatch" => self.col_regexp(name, args),
            "regexpSearch" | "regexpSplit" | "regexpSplitLimit" | "regexpReplace" => self.col_regexp(name, args),
            "regexpSource" => self.col_regexp(name, args),
            "regexpFlags" | "regexpFlag" => self.col_regexp(name, args),
            "finalizationRegistryNew" => self.col_weak(name, args),
            "finalizationRegister" => self.col_weak(name, args),
            "finalizationUnregister" => self.col_weak(name, args),
            "collectionHas" => self.col_reflect(name, args),
            "symbolHas" => self.col_reflect(name, args),
            "reflectHas" => self.col_reflect(name, args),
            "arrayDeleteIndex" => self.col_misc(name, args),
            "arrayMakeHole" => self.col_misc(name, args),
            "arrayPushHole" => self.col_misc(name, args),
            "setCallArgc" => self.col_misc(name, args),
            "argumentsObject" => self.col_misc(name, args),
            "generatorReturnValue" => self.col_misc(name, args),
            "asyncAwaitMark" => self.col_misc(name, args),
            "asyncGenResume" => self.col_misc(name, args),
            "syncIterOpen" => self.col_misc(name, args),
            "asyncIterOpen" => self.col_misc(name, args),
            "generatorSent" => self.col_misc(name, args),
            "forInHas" => self.col_misc(name, args),
            "arrayHas" => self.col_misc(name, args),
            "reflectDelete" => self.col_reflect(name, args),
            "deleteStrict" => self.col_reflect(name, args),
            "reflectSet" => self.col_reflect(name, args),
            "proxyNew" => self.col_proxy(name, args),
            "proxyGet" => self.col_proxy(name, args),
            "proxySet" => self.col_proxy(name, args),
            "proxyHas" | "proxyDelete" => self.col_proxy(name, args),
            "proxyNewCallable" => self.col_proxy(name, args),
            "proxyApply" => self.col_proxy(name, args),
            "replaceFn" => self.col_string_num(name, args),
            "stringMethod" => self.col_string_num(name, args),
            "dynamicCall0" | "dynamicCall1" | "dynamicCall2" | "dynamicCall3" | "dynamicCall4" | "dynamicCall5" | "dynamicCall6" => self.col_dynamic_call(name, args),
            "dynamicConstruct" => self.col_class_decorator(name, args),
            "dynamicSetFunction" => self.col_class_decorator(name, args),
            "decoratorAddInitializer" => self.col_class_decorator(name, args),
            "decoratorRunInitializers" => self.col_class_decorator(name, args),
            "classBox" => self.col_class_decorator(name, args),
            "reflectConstruct" => self.col_reflect(name, args),
            "proxyConstruct" => self.col_proxy(name, args),
            "applyDecorator" => self.col_class_decorator(name, args),
            "classSetSymbolFunction" => self.col_class_decorator(name, args),
            "classSetMethod" => self.col_class_decorator(name, args),
            "classSetFieldDefault" => self.col_class_decorator(name, args),
            "dynamicSetSymbolFunction" => self.col_class_decorator(name, args),
            "instanceofBuiltin" => self.col_class_decorator(name, args),
            "instanceofDynamic" => self.col_class_decorator(name, args),
            "stringCoerce" | "stringCoerceStrict" => self.col_string_num(name, args),
            "splitLimit" => self.col_string_num(name, args),
            "numberCoerce" => self.col_string_num(name, args),
            "mathApply" => self.col_string_num(name, args),
            "numberCoerceStrict" => self.col_string_num(name, args),
            "looseEquals" => self.col_string_num(name, args),
            "numberToFixedJs" | "numberToExponentialJs" | "numberToPrecisionJs"
            | "numberToStringRadixJs" => self.col_string_num(name, args),
            "stringSymbolDispatch" => self.col_string_num(name, args),
            "addAny" => {
                // ECMAScript `+`: ToPrimitive(default) on each operand, then
                // string concat if either is a string, else numeric add.
                // BigInt operands: two BigInts add; a BigInt mixed with a non-string
                // primitive is a TypeError; with a string it concatenates.
                {
                    let big = |v: Option<&Value>| -> bool {
                        match v.and_then(|v| unbox_arg(v).ok()) {
                            Some(Value::Object(obj)) => zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some()),
                            _ => false,
                        }
                    };
                    let (a_big, b_big) = (big(args.first()), big(args.get(1)));
                    if a_big && b_big {
                        return self.call_managed_collection("bigIntAdd", &[unbox_arg(&args[0])?, unbox_arg(&args[1])?]);
                    }
                    if a_big || b_big {
                        let other = if a_big { args.get(1) } else { args.first() };
                        let is_str_or_obj = matches!(
                            other.and_then(|v| unbox_arg(v).ok()),
                            Some(Value::Str(_)) | Some(Value::Object(_))
                        );
                        if !is_str_or_obj {
                            return Err(RunError::Native(
                                "TypeError: Cannot mix BigInt and other types, use explicit conversions".into(),
                            ));
                        }
                    }
                }
                let mut prims = Vec::with_capacity(2);
                for index in 0..2 {
                    if let Some(Value::Any(any)) = args.get(index) {
                        if any.tag() == zun_core::AnyTag::Symbol {
                            let other_is_string = matches!(
                                args.get(1 - index).map(|v| unbox_arg(v)),
                                Some(Ok(Value::Str(_)))
                            );
                            let kind = if other_is_string { "string" } else { "number" };
                            return Err(RunError::Native(format!(
                                "TypeError: Cannot convert a Symbol value to a {kind}"
                            )));
                        }
                    }
                    let value = unbox_arg(args.get(index).ok_or(RunError::BadToken)?)?;
                    let prim = match value {
                        Value::Object(obj) => {
                            if let Some(text) = self.array_join_dyn(obj, ",")? {
                                Value::Str(text.into())
                            } else {
                                match self.ordinary_to_primitive(obj, "default")? {
                                    Some(prim) => prim,
                                    None => Value::Str("[object Object]".into()),
                                }
                            }
                        }
                        other => other,
                    };
                    prims.push(prim);
                }
                let result = if prims.iter().any(|p| matches!(p, Value::Str(_))) {
                    let text: String = prims
                        .iter()
                        .map(|p| match p {
                            Value::Str(text) => text.to_string(),
                            other => other.display(),
                        })
                        .collect();
                    Value::Str(text.into())
                } else {
                    let left = self.to_number_value(prims[0].clone())?;
                    let right = self.to_number_value(prims[1].clone())?;
                    Value::Float(left + right, zun_il::NumKind::F64)
                };
                let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                    .unwrap_or_else(zun_core::Any::void);
                Ok(Value::Any(boxed))
            }
            "functionApply" => self.col_function(name, args),
            "functionCallSpread" => self.col_function(name, args),
            "functionBind" => self.col_function(name, args),
            "functionConstruct" => self.col_function(name, args),
            "functionBoxIs" => self.col_function(name, args),
            "functionBox" => self.col_function(name, args),
            "arrayForEach" | "arrayMap" | "arrayFilter" | "arrayFind" | "arrayFindIndex"
            | "arraySome" | "arrayEvery" => self.col_array_iterate(name, args),
            "arrayReduce" | "arrayReduceRight" => self.col_array_iterate(name, args),
            "arraySortBy" | "arraySortDefault" => self.col_array_iterate(name, args),
            "arrayJoin" => self.col_array_iterate(name, args),
            "arrayShift" => self.col_array_iterate(name, args),
            "arrayUnshift" => self.col_array_iterate(name, args),
            "arraySplice" => self.col_array_iterate(name, args),
            "arrayAt" => self.col_array_iterate(name, args),
            "arrayFindLast" | "arrayFindLastIndex" => self.col_array_iterate(name, args),
            "arrayFlat" => self.col_array_iterate(name, args),
            "arrayFlatMap" => self.col_array_iterate(name, args),
            "arrayGroup" | "arrayGroupToMap" => self.col_array_iterate(name, args),
            "arrayKeys" | "arrayValues" | "arrayEntries" => self.col_array_iterate(name, args),
            "arrayFill" | "arrayCopyWithin" => self.col_array_iterate(name, args),
            "methodCallSpread" => self.col_array_iterate(name, args),
            "arraySliceArgs" => self.col_array_mutate(name, args),
            "arraySpliceArgs" => self.col_array_mutate(name, args),
            "arraySpliceInsert" => self.col_array_mutate(name, args),
            "arraySlice" => self.col_array_mutate(name, args),
            "arraySliceRange" => self.col_array_mutate(name, args),
            "arrayReverse" => self.col_array_mutate(name, args),
            "arrayIndexOf" | "arrayLastIndexOf" | "arrayIncludes" => self.col_array_mutate(name, args),
            "anyIsUndefined" => self.col_array_mutate(name, args),
            "coalesceUndefined" => self.col_array_mutate(name, args),
            "objectRest" => self.col_array_mutate(name, args),
            "arrayReduceNoInit" | "arrayReduceRightNoInit" => self.col_array_mutate(name, args),
            "arrayToSorted" | "arrayToSortedBy" => self.col_array_mutate(name, args),
            "arrayToReversed" => self.col_array_mutate(name, args),
            "arrayWith" => self.col_array_mutate(name, args),
            "arrayToSpliced" => self.col_array_mutate(name, args),
            "arrayConcat" => self.col_array_mutate(name, args),
            "arrayIsArray" => self.col_array_construct(name, args),
            "arrayDestructure" => self.col_array_construct(name, args),
            "arrayLikeFill" => self.col_array_construct(name, args),
            "arrayLikeReverse" => self.col_array_construct(name, args),
            "arrayLikePush" | "arrayLikePop" => self.col_array_construct(name, args),
            "arrayFromArrayLike" => self.col_array_construct(name, args),
            "destructOpen" => self.col_array_construct(name, args),
            "destructStep" => self.col_array_construct(name, args),
            "destructRest" => self.col_array_construct(name, args),
            "destructFinish" => self.col_array_construct(name, args),
            "arrayFrom" | "arrayFromMapped" => self.col_array_construct(name, args),
            "arraySpreadAppend" => self.col_array_construct(name, args),
            "promiseNew" => self.col_promise(name, args),
            "promiseWithResolvers" => self.col_promise(name, args),
            "promiseNewExecutor" => self.col_promise(name, args),
            "promiseResolve" => self.col_promise(name, args),
            "promiseReject" => self.col_promise(name, args),
            "promiseState" => self.col_promise(name, args),
            "promiseResult" => self.col_promise(name, args),
            "promiseIsReady" => self.col_promise(name, args),
            "promiseThen" => self.col_promise(name, args),
            "promiseCatch" => self.col_promise(name, args),
            "promiseFinally" => self.col_promise(name, args),
            "promiseResolveStatic" => self.col_promise(name, args),
            "promiseRejectStatic" => self.col_promise(name, args),
            "promiseAll" | "promiseAllSettled" | "promiseRace" | "promiseAny" => self.col_promise(name, args),
            "setTimeout" | "setInterval" | "setImmediate" => self.col_promise(name, args),
            "setTimeoutArgs" | "setIntervalArgs" => self.col_promise(name, args),
            "clearTimeout" | "clearInterval" => self.col_promise(name, args),
            "queueMicrotask" => self.col_promise(name, args),
            "objectForInKeys" => self.col_object_core(name, args),
            "objectKeys" => self.col_object_core(name, args),
            "objectValues" => self.col_object_core(name, args),
            "objectEntries" => self.col_object_core(name, args),
            "objectAssign" => self.col_object_core(name, args),
            "globalSet" => self.col_object_core(name, args),
            "globalGet" => self.col_object_core(name, args),
            "globalRead" => self.col_object_core(name, args),
            "realmDescriptor" => self.col_object_core(name, args),
            "classDescriptorToken" => self.col_object_core(name, args),
            "symbolNew" => self.col_object_core(name, args),
            "symbolFor" => self.col_object_core(name, args),
            "symbolKeyFor" => self.col_object_core(name, args),
            "symbolWellKnown" => self.col_object_core(name, args),
            "symbolDescription" => self.col_object_core(name, args),
            "objectFreeze" => self.col_object_core(name, args),
            "objectIsFrozen" => self.col_object_core(name, args),
            "objectCreate" => self.col_object_core(name, args),
            "objectGetPrototypeOf" => self.col_object_core(name, args),
            "objectJsonStringify" => self.col_object_core(name, args),
            "objectPrototype" => self.col_object_core(name, args),
            "regexpPrototype" => self.col_object_core(name, args),
            "objectSetPrototypeOf" => self.col_object_core(name, args),
            "templateObject" => self.col_object_descriptor(name, args),
            "objectGroupBy" => self.col_object_descriptor(name, args),
            "objectFromEntries" => self.col_object_descriptor(name, args),
            "objectIs" => self.col_object_descriptor(name, args),
            "objectHasOwn" => self.col_object_descriptor(name, args),
            "objectIsPrototypeOf" => self.col_object_descriptor(name, args),
            "objectDefineProperty" | "reflectDefineProperty" => self.col_object_descriptor(name, args),
            "objectDefineProperties" => self.col_object_descriptor(name, args),
            "objectGetOwnPropertyDescriptor" => self.col_object_descriptor(name, args),
            "objectGetOwnPropertyDescriptors" => self.col_object_descriptor(name, args),
            "objectGetOwnPropertyNames" => self.col_object_descriptor(name, args),
            "reflectOwnKeys" => self.col_object_descriptor(name, args),
            "objectGetOwnPropertySymbols" => self.col_object_descriptor(name, args),
            "objectSeal" => self.col_object_descriptor(name, args),
            "objectIsSealed" => self.col_object_descriptor(name, args),
            "objectPreventExtensions" => self.col_object_descriptor(name, args),
            "objectIsExtensible" => self.col_object_descriptor(name, args),
            "objectPropertyIsEnumerable" => self.col_object_descriptor(name, args),
            "captureStackTrace" => self.col_object_descriptor(name, args),
            "typeofAny" => self.col_object_descriptor(name, args),
            "objectToStringTag" => self.col_object_descriptor(name, args),
            "instanceofClass" => self.col_object_descriptor(name, args),
            "serialize" => self.col_object_descriptor(name, args),
            "deserialize" => self.col_object_descriptor(name, args),
            _ => Err(RunError::BadToken),
        }
    }

    /// Render the live activation chain as the frame lines of an `Error.stack`
    /// string: innermost frame first, one `    at <fn> (<span>)` per frame. See
    /// the `captureStackTrace` arm for the fidelity notes.
    pub(crate) fn render_stack_trace(&self) -> String {
        let mut lines = String::new();
        for (index, frame) in self.call_stack.iter().rev().enumerate() {
            if index > 0 {
                lines.push('\n');
            }
            let (name, span) = match self.module.debug_frame_at(frame.method_rid, frame.pc) {
                Some(debug) => (debug.method_name, debug.source_span),
                None => ("<anonymous>", None),
            };
            let name = if name.is_empty() { "<anonymous>" } else { name };
            match span {
                Some((start, end)) => {
                    lines.push_str(&format!("    at {name} ({start}..{end})"));
                }
                None => lines.push_str(&format!("    at {name}")),
            }
        }
        lines
    }

    /// Enumerate the own enumerable string keys and their values for either a
    /// dynamic object (insertion-ordered dynamic keys) or a fixed-layout typed
    /// object (declared instance fields, base classes first, static and private
    /// fields excluded).
    pub(crate) fn object_own_entries(
        &mut self,
        obj: zun_gc::Obj,
    ) -> Result<Vec<(String, Value)>, RunError> {
        self.object_own_entries_impl(obj, true)
    }

    /// Enumerate own enumerable keys; `read_values` false skips getters (values
    /// come back as `undefined`), as `Object.keys` never reads property values.
    pub(crate) fn object_own_entries_impl(
        &mut self,
        obj: zun_gc::Obj,
        read_values: bool,
    ) -> Result<Vec<(String, Value)>, RunError> {
        // A primitive wrapper object (`new String("hi")`, `new Number(1)`):
        // a string wrapper exposes its characters; number/boolean wrappers
        // have no own enumerable properties.
        if !zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj)) && obj.type_rid() >= 1 {
            let wrapper = self
                .module
                .tables
                .type_defs
                .get(obj.type_rid() as usize - 1)
                .map(|def| self.module.strings.get(def.name).to_string());
            match wrapper.as_deref() {
                Some("StringObject") => {
                    let text = match self.instance_member_get(obj, "value")? {
                        Some(Value::Str(text)) => text.to_string(),
                        _ => String::new(),
                    };
                    return Ok(text
                        .chars()
                        .enumerate()
                        .map(|(index, ch)| (index.to_string(), Value::Str(ch.to_string().into())))
                        .collect());
                }
                Some("NumberObject") | Some("BooleanObject") => return Ok(Vec::new()),
                _ => {}
            }
        }
        // A typed array: its elements by index.
        if let Some(len) = crate::typed_arrays::element_len(obj) {
            let mut out: Vec<(String, Value)> = Vec::with_capacity(len);
            for index in 0..len {
                if let Some(element) = crate::typed_arrays::read_element(obj, index) {
                    out.push((index.to_string(), element));
                }
            }
            return Ok(out);
        }
        // An array: its present indices (as strings), then enumerable expando
        // properties.
        if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(obj)) {
            let mut out: Vec<(String, Value)> = Vec::new();
            for index in 0..len {
                let element = zun_gc::with_current_heap(|heap| {
                    heap.array_get(obj, index)
                        .filter(|any| !any.is_hole())
                        .and_then(|any| Value::unbox_any(any, heap))
                });
                if let Some(element) = element {
                    out.push((index.to_string(), element));
                }
            }
            let expandos = zun_gc::with_current_heap(|heap| {
                heap.dynamic_keys(obj)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|key| heap.dynamic_is_enumerable(obj, key))
                    .filter_map(|key| {
                        heap.dynamic_get(obj, &key)
                            .and_then(|any| Value::unbox_any(any, heap))
                            .map(|value| (key, value))
                    })
                    .collect::<Vec<_>>()
            });
            out.extend(expandos);
            return Ok(out);
        }
        if zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj) && !heap.is_instance_bag(obj)) {
            let keys = zun_gc::with_current_heap(|heap| heap.dynamic_keys(obj))
                .ok_or(RunError::BadToken)?;
            let mut out = Vec::with_capacity(keys.len());
            for key in keys {
                let (enumerable, getter, data) = zun_gc::with_current_heap(|heap| {
                    (
                        heap.dynamic_is_enumerable(obj, &key),
                        heap.dynamic_accessor_getter(obj, &key),
                        heap.dynamic_get(obj, &key),
                    )
                });
                if !enumerable {
                    continue;
                }
                if !read_values {
                    out.push((key, Value::Void));
                    continue;
                }
                let value = if let Some(getter) = getter {
                    self.invoke_function_value(getter, self.with_this_arg(getter, obj, Vec::new()))?
                } else if zun_gc::with_current_heap(|heap| heap.dynamic_is_accessor(obj, &key)) {
                    Value::Void
                } else if let Some(data) = data {
                    zun_gc::with_current_heap(|heap| Value::unbox_any(data, heap))
                        .ok_or(RunError::BadToken)?
                } else {
                    continue;
                };
                out.push((key, value));
            }
            return Ok(out);
        }
        // Typed object: walk the declared-field layout from the base-most type
        // down to the concrete type so keys read in definition order.
        let type_rid = obj.type_rid();
        let mut chain = Vec::new();
        let mut current = type_rid;
        loop {
            // A zero rid is not a valid TypeDef (1-based); stop walking rather
            // than underflow on `current - 1`.
            if current == 0 {
                break;
            }
            chain.push(current);
            let ty = self
                .module
                .tables
                .type_defs
                .get(current as usize - 1)
                .ok_or(RunError::BadToken)?;
            if ty.base.is_null() || ty.base.table() != Some(zun_metadata::TableKind::TypeDef) {
                break;
            }
            current = ty.base.rid();
        }
        // Every TypeDef's field range already includes the inherited fields
        // (base-first), so only the concrete type is walked. The leading fields
        // that belong to a runtime-internal base (the `Error` record slots,
        // `AggregateError.errors`, a `__Builtin*` wrapper's native value) are
        // not own enumerable properties and are skipped.
        let mut hidden_prefix = 0usize;
        for &rid in &chain {
            let ty = self.module.tables.type_defs.get(rid as usize - 1).ok_or(RunError::BadToken)?;
            let name = self.module.strings.get(ty.name);
            if name == "Error" || name == "AggregateError" || name.starts_with("__Builtin") {
                hidden_prefix = hidden_prefix.max((ty.field_end - ty.field_start) as usize);
            }
        }
        chain.reverse();
        let mut out = Vec::new();
        for type_rid in chain.into_iter().rev().take(1) {
            let (field_start, field_end) = {
                let ty = self
                    .module
                    .tables
                    .type_defs
                    .get(type_rid as usize - 1)
                    .ok_or(RunError::BadToken)?;
                (ty.field_start, ty.field_end)
            };
            for field_rid in field_start..field_end {
                if ((field_rid - field_start) as usize) < hidden_prefix {
                    continue;
                }
                let field = self
                    .module
                    .tables
                    .field_defs
                    .get(field_rid as usize - 1)
                    .ok_or(RunError::BadToken)?;
                if field.is_static || field.is_private {
                    continue;
                }
                let name = self.module.strings.get(field.name).to_string();
                let layout = self.types.get_field(field_rid).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| {
                    super::value::load_field(layout.kind, obj, layout.slot, heap)
                });
                out.push((name, value));
            }
        }
        // Expando properties of a typed instance follow its declared fields;
        // canonical integer keys sort first, as for any ordinary object.
        if zun_gc::with_current_heap(|heap| heap.is_instance_bag(obj)) {
            let extras: Vec<(String, Value)> = zun_gc::with_current_heap(|heap| {
                heap.dynamic_keys(obj)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|key| heap.dynamic_is_enumerable(obj, key))
                    .filter_map(|key| {
                        heap.dynamic_get(obj, &key)
                            .and_then(|any| Value::unbox_any(any, heap))
                            .map(|value| (key, value))
                    })
                    .collect()
            });
            out.extend(extras);
            let is_index = |key: &str| key.parse::<u32>().is_ok_and(|n| n.to_string() == key && n != u32::MAX);
            let (mut indexed, rest): (Vec<_>, Vec<_>) = out.into_iter().partition(|(key, _)| is_index(key));
            indexed.sort_by_key(|(key, _)| key.parse::<u32>().unwrap_or(0));
            indexed.extend(rest);
            out = indexed;
        }
        Ok(out)
    }

    /// Own string property names of `obj` (dynamic or typed), including
    /// non-enumerable keys (`getOwnPropertyNames`).
    fn object_own_names(&mut self, obj: zun_gc::Obj) -> Result<Vec<String>, RunError> {
        if !zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj)) && obj.type_rid() >= 1 {
            let is_string_wrapper = self
                .module
                .tables
                .type_defs
                .get(obj.type_rid() as usize - 1)
                .map(|def| self.module.strings.get(def.name) == "StringObject")
                .unwrap_or(false);
            if is_string_wrapper {
                let mut names: Vec<String> =
                    self.object_own_entries(obj)?.into_iter().map(|(name, _)| name).collect();
                names.push("length".to_owned());
                return Ok(names);
            }
        }
        if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(obj)) {
            let mut names: Vec<String> = zun_gc::with_current_heap(|heap| {
                (0..len)
                    .filter(|&index| !heap.array_get(obj, index).is_some_and(|any| any.is_hole()))
                    .map(|index| index.to_string())
                    .collect()
            });
            names.push("length".to_owned());
            let expandos = zun_gc::with_current_heap(|heap| heap.dynamic_keys(obj)).unwrap_or_default();
            names.extend(expandos);
            return Ok(names);
        }
        if zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj)) {
            zun_gc::with_current_heap(|heap| heap.dynamic_keys(obj)).ok_or(RunError::BadToken)
        } else {
            Ok(self.object_own_entries(obj)?.into_iter().map(|(name, _)| name).collect())
        }
    }

    /// Build a descriptor object `{ value, writable, enumerable, configurable }`
    /// or `{ get, set, enumerable, configurable }` for `obj[key]`, or `None` if
    /// there is no own property.
    /// Data descriptor of an own symbol-keyed property (`None` when absent).
    fn build_symbol_descriptor(&mut self, obj: zun_gc::Obj, symbol: zun_core::SymbolId) -> Option<zun_gc::Obj> {
        zun_gc::with_current_heap_mut(|heap| {
            let value = heap.dynamic_get_symbol(obj, symbol)?;
            let enumerable = heap.dynamic_is_symbol_enumerable(obj, symbol);
            let frozen = !heap.dynamic_can_set(obj, " __symbol_probe");
            let out = heap.alloc_dynamic();
            heap.dynamic_set(out, "value".to_owned(), value);
            heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(!frozen));
            heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(enumerable));
            heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(!frozen));
            Some(out)
        })
    }

    fn build_descriptor(&mut self, obj: zun_gc::Obj, key: &str) -> Result<Option<zun_gc::Obj>, RunError> {
        // Arrays: indexed elements and the `length` data property.
        if let Some((len, level)) = zun_gc::with_current_heap(|heap| {
            heap.array_len(obj).map(|len| (len, heap.array_integrity(obj)))
        }) {
            let entry = if key == "length" {
                Some((zun_core::Any::int(len as i64), level < 3, false, false))
            } else {
                match key.parse::<usize>() {
                    Ok(index) if index < len => zun_gc::with_current_heap(|heap| heap.array_get(obj, index))
                        .map(|value| (value, level < 3, true, level < 2)),
                    _ => None,
                }
            };
            // A non-index expando on an array (e.g. a template object's `raw`)
            // reports through the dynamic-property path below.
            let expando = entry.is_none()
                && key.parse::<usize>().is_err()
                && zun_gc::with_current_heap(|heap| heap.dynamic_descriptor(obj, key).is_some());
            if !expando {
            let Some((value, writable, enumerable, configurable)) = entry else {
                return Ok(None);
            };
            let out = zun_gc::with_current_heap_mut(|heap| {
                let out = heap.alloc_dynamic();
                heap.dynamic_set(out, "value".to_owned(), value);
                heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(writable));
                heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(enumerable));
                heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(configurable));
                out
            });
            return Ok(Some(out));
            }
        }
        let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
        if is_dynamic || zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()) {
            let Some(descriptor) = zun_gc::with_current_heap(|heap| heap.dynamic_descriptor(obj, key)) else {
                return Ok(None);
            };
            let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
            let set_bool = |name: &str, flag: bool| {
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set(out, name.to_owned(), zun_core::Any::bool(flag));
                });
            };
            match descriptor {
                zun_core::PropertyDescriptor::Data { value, writable, enumerable, configurable } => {
                    zun_gc::with_current_heap_mut(|heap| {
                        // A function-slot property reports its function as the value.
                        match heap.dynamic_get_function(obj, key) {
                            Some(handle) => {
                                heap.dynamic_set_function(out, "value", handle);
                            }
                            None => {
                                heap.dynamic_set(out, "value".to_owned(), value);
                            }
                        }
                    });
                    set_bool("writable", writable);
                    set_bool("enumerable", enumerable);
                    set_bool("configurable", configurable);
                }
                zun_core::PropertyDescriptor::Accessor { get, set, enumerable, configurable } => {
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(handle) = get {
                            heap.dynamic_set_function(out, "get", handle);
                        } else {
                            heap.dynamic_set(out, "get".to_owned(), zun_core::Any::void());
                        }
                        if let Some(handle) = set {
                            heap.dynamic_set_function(out, "set", handle);
                        } else {
                            heap.dynamic_set(out, "set".to_owned(), zun_core::Any::void());
                        }
                    });
                    set_bool("enumerable", enumerable);
                    set_bool("configurable", configurable);
                }
            }
            Ok(Some(out))
        } else {
            // Typed object: a declared field is a writable, enumerable data
            // property (the fixed layout is non-configurable).
            let entries = self.object_own_entries(obj)?;
            let Some((_, value)) = entries.into_iter().find(|(name, _)| name == key) else {
                return Ok(None);
            };
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
            zun_gc::with_current_heap_mut(|heap| {
                heap.dynamic_set(out, "value".to_owned(), boxed);
                heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(true));
                heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(true));
                heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(false));
            });
            Ok(Some(out))
        }
    }

    /// Classify how a reflective operation on `obj` resolves for a given trap
    /// name (see [`ProxyRoute`]). A proxy with the trap routes through it; a
    /// proxy without it falls back to its target; a plain object operates on
    /// itself.
    fn proxy_route(&self, obj: zun_gc::Obj, trap: &str) -> ProxyRoute {
        match zun_gc::with_current_heap(|heap| heap.proxy_target(obj)) {
            Some(target) => match zun_gc::with_current_heap(|heap| heap.proxy_trap(obj, trap)) {
                Some(callback) => ProxyRoute::Trap(callback, target),
                None => ProxyRoute::Target(target),
            },
            None => ProxyRoute::NotProxy,
        }
    }

    /// Invoke a proxy handler trap, threading its closure environment (if any)
    /// as the leading positional slot and appending `leading` (the target and
    /// the trap-specific operands). Mirrors the dispatch convention of the
    /// `get`/`set`/`has`/`deleteProperty` traps.
    /// Construct an instance of a user class from a runtime class descriptor
    /// token (`USER_CLASS_FLAG | TypeDef rid`) and positional field values: the
    /// runtime twin of `NewObj` (which `new C(...)` lowers to). The value count
    /// may be shorter than the class's field count: trailing slots take the
    /// literal initializer default recorded by `classSetFieldDefault` (non-literal
    /// initializers are not reachable here and make the call fail).
    /// Design boundary: only typed classes are constructible (no
    /// `function.prototype` + `this` model for arbitrary function constructors),
    /// and `newTarget` is approximated by the target class.
    /// Returns the instance boxed as an `Any`.
    pub(crate) fn construct_class_from_descriptor(
        &mut self,
        token: u32,
        mut values: Vec<Value>,
    ) -> Result<Value, RunError> {
        if token & zun_natives::USER_CLASS_FLAG == 0 {
            return Err(RunError::BadToken);
        }
        let token = zun_metadata::Token::new(
            zun_metadata::TableKind::TypeDef,
            token & !zun_natives::USER_CLASS_FLAG,
        );
        let (field_start, field_count) = self.type_field_range(token)?;
        // A class with a `constructor` (own or inherited): allocate the instance
        // with its recorded defaults and run the constructor on it.
        if let Some(ctor) = self.class_constructor(token.rid()) {
            let kinds: Vec<u8> = (0..field_count as u32)
                .map(|i| self.field_kind_at(field_start + i))
                .collect::<Result<_, _>>()?;
            // Slots without a recorded default stay zeroed (0, false, null, undefined).
            let defaults: Vec<Option<Value>> = (0..field_count)
                .map(|slot| self.class_field_defaults.get(&(token.rid(), slot as u32)).cloned())
                .collect();
            let obj = zun_gc::with_current_heap_mut(|h| {
                let obj = h.alloc(token.rid(), field_count);
                for (i, value) in defaults.iter().enumerate() {
                    if let Some(value) = value.as_ref().filter(|v| !matches!(v, Value::Void) || kinds[i] == crate::value::FIELD_ANY) {
                        crate::value::store_field(kinds[i], obj, i, value, h);
                    }
                }
                obj
            });
            let mut call_args = Vec::with_capacity(values.len() + 1);
            call_args.push(Value::Object(obj));
            call_args.extend(values);
            // Omitted trailing parameters carry the null sentinel that the
            // callee's default-parameter prologue consumes.
            let declared = self
                .module
                .tables
                .method_defs
                .get(ctor as usize - 1)
                .and_then(|method| read_body(&self.module.code, method.body_offset).ok())
                .map_or(call_args.len(), |body| body.param_count as usize);
            while call_args.len() < declared {
                call_args.push(Value::Null);
            }
            self.call(ctor, call_args, None)?;
            let obj = self.run_instance_hooks(obj)?;
            let boxed = zun_gc::with_current_heap_mut(|heap| Value::Object(obj).box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            return Ok(Value::Any(boxed));
        }
        if values.len() > field_count {
            return Err(RunError::BadToken);
        }
        // Trailing absent args take the class's literal field-initializer
        // default (registered by `classSetFieldDefault`); an absent arg with no
        // recorded default is an error.
        for slot in values.len()..field_count {
            let default = self
                .class_field_defaults
                .get(&(token.rid(), slot as u32))
                .cloned()
                .ok_or(RunError::BadToken)?;
            values.push(default);
        }
        let kinds: Vec<u8> = (0..field_count as u32)
            .map(|i| self.field_kind_at(field_start + i))
            .collect::<Result<_, _>>()?;
        let obj = zun_gc::with_current_heap_mut(|h| {
            let obj = h.alloc(token.rid(), field_count);
            for (i, value) in values.iter().enumerate() {
                crate::value::store_field(kinds[i], obj, i, value, h);
            }
            obj
        });
        let obj = self.run_instance_hooks(obj)?;
        let boxed = zun_gc::with_current_heap_mut(|heap| Value::Object(obj).box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        Ok(Value::Any(boxed))
    }

    fn invoke_proxy_trap(
        &mut self,
        callback: zun_core::FunctionHandle,
        leading: Vec<Value>,
    ) -> Result<Value, RunError> {
        let mut call_args = Vec::with_capacity(leading.len() + 1);
        if let Some(address) = callback.environment() {
            call_args.push(Value::Object(unsafe {
                zun_gc::Obj::from_addr(address as *mut u64)
            }));
        }
        call_args.extend(leading);
        self.call(callback.method_rid(), call_args, None)
    }

    /// Read every element of a managed array, unboxing each `Any` slot to its
    /// concrete interpreter value.
    pub(super) fn read_array_values(&self, array: zun_gc::Obj) -> Result<Vec<Value>, RunError> {
        zun_gc::with_current_heap(|heap| {
            let length = heap.array_len(array).ok_or(RunError::BadToken)?;
            let mut values = Vec::with_capacity(length);
            for index in 0..length {
                let any = heap.array_get(array, index).ok_or(RunError::BadToken)?;
                values.push(Value::unbox_any(any, heap).ok_or(RunError::BadToken)?);
            }
            Ok(values)
        })
    }

    /// Resolve a function value and positional arguments to the concrete target
    /// method RID and the final argument vector, peeling a `bind` chain (each
    /// bound record prepends its recorded `[thisArg, ...partial]`) and threading
    /// the target closure environment (if any) as the leading slot. Shared by
    /// `apply`, `bind`-chain calls, and the bound-function path of `CallIndirect`.
    /// Build a `Promise.withResolvers` settler: a bound-function handle over a
    /// record naming the promise and the settle mode. It is callable like any
    /// function value, but is dispatched natively (see [`Self::settler_of`] /
    /// the interception in [`Self::invoke_function_value`] and `CallIndirect`),
    /// not through a method body.
    pub(super) fn make_promise_settler(
        &mut self,
        promise: zun_gc::Obj,
        mode: f64,
    ) -> zun_core::FunctionHandle {
        let roots = [promise];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let record = zun_gc::with_current_heap_mut(|heap| {
            let record = heap.alloc_dynamic();
            heap.dynamic_set(record, "__settleMode", zun_core::Any::float(mode));
            heap.dynamic_set(
                record,
                "__settlePromise",
                zun_core::Any::object(roots[0].addr() as usize),
            );
            record
        });
        zun_core::FunctionHandle::new_bound(record.addr() as usize)
    }

    /// If `handle` is a `withResolvers` settler, return the promise it settles
    /// and its mode (`SETTLE_RESOLVE` / `SETTLE_REJECT`). A settler is a bound
    /// handle whose record carries `__settleMode`; an ordinary `bind` record
    /// carries `__target` instead, so the two never collide.
    pub(super) fn settler_of(
        &self,
        handle: zun_core::FunctionHandle,
    ) -> Option<(zun_gc::Obj, f64)> {
        if !handle.is_bound() {
            return None;
        }
        let record = unsafe { zun_gc::Obj::from_addr(handle.environment()? as *mut u64) };
        zun_gc::with_current_heap(|heap| {
            let mode = heap.dynamic_get(record, "__settleMode")?.as_float()?;
            let promise = heap.dynamic_get(record, "__settlePromise")?.as_object()?;
            let promise = unsafe { zun_gc::Obj::from_addr(promise as *mut u64) };
            Some((promise, mode))
        })
    }

    /// Run a `withResolvers` settler: `resolve(v)` fulfils the promise with `v`;
    /// `reject(e)` rejects it with `e` (a first-class reason when `e` is an
    /// object/boxed value, else its string message). Returns `undefined`, like
    /// the JS resolver functions.
    pub(super) fn invoke_promise_settler(
        &mut self,
        promise: zun_gc::Obj,
        mode: f64,
        mut args: Vec<Value>,
    ) -> Result<Value, RunError> {
        if mode >= 100.0 {
            // An `Object.prototype` method value: `valueOf` answers the receiver
            // object, the predicates answer false, the string forms the tag.
            return Ok(match mode as u32 - 100 {
                2 => Value::Object(promise),
                3..=5 => Value::Bool(false),
                _ => Value::Str("[object Object]".into()),
            });
        }
        let value = if args.is_empty() { Value::Void } else { args.remove(0) };
        if mode == SETTLE_REJECT {
            match &value {
                // An object/boxed reason is carried first-class; a plain string
                // keeps the string surface.
                Value::Str(message) => {
                    self.settle_promise_reject(promise, message.to_string());
                }
                other => {
                    let reason = zun_gc::with_current_heap_mut(|heap| other.clone().box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    let message = value_to_error_string(other);
                    self.settle_promise_reject_value(promise, reason, message);
                }
            }
        } else {
            // Resolving with a Promise adopts it (chaining, not wrapping): the
            // settled promise tracks the adopted one. Any other value fulfils
            // directly. Generic (non-Promise) thenables are not yet detected —
            // only managed Promise adoption is handled here.
            self.settle_target_with(promise, value);
        }
        Ok(Value::Void)
    }

    pub(crate) fn resolve_call_target(
        &self,
        mut handle: zun_core::FunctionHandle,
        mut call_args: Vec<Value>,
    ) -> Result<(u32, Vec<Value>), RunError> {
        while handle.is_bound() {
            let record = unsafe {
                zun_gc::Obj::from_addr(handle.environment().ok_or(RunError::BadToken)? as *mut u64)
            };
            let (target, prepended) = zun_gc::with_current_heap(|heap| {
                let target = heap
                    .dynamic_get_function(record, "__target")
                    .ok_or(RunError::BadToken)?;
                let args_any = heap.dynamic_get(record, "__args").ok_or(RunError::BadToken)?;
                let array = args_any.as_object().ok_or(RunError::BadToken)?;
                let array = unsafe { zun_gc::Obj::from_addr(array as *mut u64) };
                let length = heap.array_len(array).ok_or(RunError::BadToken)?;
                let mut prepended = Vec::with_capacity(length);
                for index in 0..length {
                    let any = heap.array_get(array, index).ok_or(RunError::BadToken)?;
                    let value = Value::unbox_any(any, heap).ok_or(RunError::BadToken)?;
                    // A function-typed partial argument was stored as a
                    // `functionBox` wrapper (functions do not box into `Any`);
                    // restore the callable before it is threaded as an argument.
                    prepended.push(unbox_function_box(value, heap));
                }
                Ok::<_, RunError>((target, prepended))
            })?;
            let mut combined = prepended;
            combined.append(&mut call_args);
            call_args = combined;
            handle = target;
        }
        let final_args = if let Some(address) = handle.environment() {
            let environment = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
            let mut args = Vec::with_capacity(call_args.len() + 1);
            args.push(Value::Object(environment));
            args.extend(call_args);
            args
        } else {
            call_args
        };
        Ok((handle.method_rid(), final_args))
    }

    /// The managed object an interpreter value refers to, unboxing an `Any`.
    fn value_object(value: &Value) -> Option<zun_gc::Obj> {
        match value {
            Value::Object(object) => Some(*object),
            Value::Any(any) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
            _ => None,
        }
    }

    /// ECMAScript `GetIterator(source)`: resolve the iterator object that drives
    /// a for-of / spread / collection-constructor over `source`.
    ///
    /// A built-in iterator object drives itself; a raw array/set/map is wrapped
    /// in the matching built-in iterator; a user value implementing
    /// `[Symbol.iterator]` (an object literal's symbol-function slot or a typed
    /// class's computed-symbol method) has it invoked to produce the iterator.
    fn resolve_iterator(&mut self, source: zun_gc::Obj) -> Result<zun_gc::Obj, RunError> {
        let iterator_id = zun_core::WellKnownSymbol::Iterator.id();
        let is_builtin_iter = zun_gc::with_current_heap(|heap| {
            heap.array_iterator_done(source).is_some()
                || heap.set_iterator_done(source).is_some()
                || heap.map_iterator_done(source).is_some()
        });
        if is_builtin_iter || zun_gc::with_current_heap(|heap| heap.generator_state(source).is_some()) {
            return Ok(source);
        }
        if zun_gc::with_current_heap(|heap| heap.array_len(source).is_some() && heap.dynamic_get_symbol_function(source, iterator_id).is_none()) {
            return zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(source))
                .ok_or(RunError::BadToken);
        }
        if zun_gc::with_current_heap(|heap| heap.set_values(source)).is_some() {
            return zun_gc::with_current_heap_mut(|heap| heap.alloc_set_iterator(source))
                .ok_or(RunError::BadToken);
        }
        if zun_gc::with_current_heap(|heap| heap.map_entries(source)).is_some() {
            return zun_gc::with_current_heap_mut(|heap| heap.alloc_map_iterator(source))
                .ok_or(RunError::BadToken);
        }
        let handle = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get_symbol_function(source, iterator_id)
        })
        .or_else(|| {
            self.class_symbol_methods
                .get(&(source.type_rid(), iterator_id.get()))
                .copied()
        });
        let Some(handle) = handle else {
            return Err(RunError::Native("TypeError: value is not iterable".into()));
        };
        let call_args = self.with_this_arg(handle, source, Vec::new());
        let result = self.invoke_function_value(handle, call_args)?;
        Self::value_object(&result).ok_or_else(|| {
            RunError::Native("TypeError: [Symbol.iterator]() did not return an object".into())
        })
    }

    /// Advance a resolved iterator object once, returning `(value, done)`. A
    /// built-in iterator advances through its native cursor; a user iterator has
    /// its `next()` method called and the `{ value, done }` result read back.
    fn protocol_step(&mut self, it: zun_gc::Obj) -> Result<(Value, bool), RunError> {
        if zun_gc::with_current_heap(|heap| heap.generator_state(it).is_some()) {
            let finished = self.call_managed_collection("generatorIteratorDone", &[Value::Object(it)])?;
            if matches!(finished, Value::Bool(true)) {
                return Ok((Value::Void, true));
            }
            let value = self.call_managed_collection("generatorIteratorNext", &[Value::Object(it)])?;
            return Ok((value, false));
        }
        if let Some(done) = zun_gc::with_current_heap(|heap| heap.array_iterator_done(it)) {
            if done {
                return Ok((Value::Void, true));
            }
            let value = zun_gc::with_current_heap_mut(|heap| heap.array_iterator_next(it))
                .and_then(|any| zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)))
                .unwrap_or(Value::Void);
            return Ok((value, false));
        }
        if let Some(done) = zun_gc::with_current_heap(|heap| heap.set_iterator_done(it)) {
            if done {
                return Ok((Value::Void, true));
            }
            let value = zun_gc::with_current_heap_mut(|heap| heap.set_iterator_next(it))
                .and_then(|any| zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)))
                .unwrap_or(Value::Void);
            return Ok((value, false));
        }
        if let Some(done) = zun_gc::with_current_heap(|heap| heap.map_iterator_done(it)) {
            if done {
                return Ok((Value::Void, true));
            }
            let value = zun_gc::with_current_heap_mut(|heap| heap.map_iterator_next(it))
                .map(Value::Object)
                .unwrap_or(Value::Void);
            return Ok((value, false));
        }
        let next = zun_gc::with_current_heap(|heap| heap.dynamic_get_function(it, "next"));
        let Some(next) = next else {
            return Err(RunError::Native("TypeError: iterator has no next() method".into()));
        };
        let roots = [it];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let next_args = self.with_this_arg(next, it, Vec::new());
        let result = self.invoke_function_value(next, next_args)?;
        let Some(result_obj) = Self::value_object(&result) else {
            return Err(RunError::Native("TypeError: iterator.next() did not return an object".into()));
        };
        let done = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(result_obj, "done")
                .and_then(|any| Value::unbox_any(any, heap))
                .map(|value| is_truthy(&value, heap))
                .unwrap_or(false)
        });
        if done {
            return Ok((Value::Void, true));
        }
        let value = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(result_obj, "value")
                .and_then(|any| Value::unbox_any(any, heap))
        })
        .unwrap_or(Value::Void);
        Ok((value, false))
    }

    /// Build the look-ahead wrapper that backs the generic for-of protocol path
    /// (`generatorIterator*` over a non-generator). It carries the resolved
    /// iterator (`__zun_it`) plus a one-slot cache (`__zun_cur` / `__zun_done` /
    /// `__zun_has`) so the desugared `done`/`next` pair reads one lookahead.
    pub(crate) fn protocol_iterator_new(
        &mut self,
        source: zun_gc::Obj,
    ) -> Result<Value, RunError> {
        let it = self.resolve_iterator(source)?;
        let roots = [it];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let wrapper = zun_gc::with_current_heap_mut(|heap| {
            let wrapper = heap.alloc_dynamic();
            heap.dynamic_set(
                wrapper,
                "__zun_it",
                zun_core::Any::object(roots[0].addr() as usize),
            );
            wrapper
        });
        Ok(Value::Object(wrapper))
    }

    /// IteratorClose for a real generator: resume the suspended frame as a
    /// `return()` completion so its `finally` clauses run, then mark it done.
    fn close_generator(
        &mut self,
        generator: zun_gc::Obj,
        throwing: bool,
    ) -> Result<Value, RunError> {
        let mut state = zun_gc::with_current_heap_mut(|heap| heap.generator_state_take(generator))
            .ok_or(RunError::BadToken)?;
        let saved = if state.done { None } else { state.frame.take() };
        state.has_current = false;
        state.current = zun_core::Any::void();
        let Some(saved) = saved else {
            state.done = true;
            zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
            return Ok(Value::Bool(true));
        };
        let locals = zun_gc::with_current_heap(|heap| {
            saved.locals.iter().map(|value| gen_unbox(*value, heap)).collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let stack = zun_gc::with_current_heap(|heap| {
            saved.eval_stack.iter().map(|value| gen_unbox(*value, heap)).collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let mut frame = ExecutionFrame::new(state.method_rid, locals);
        frame.pc = saved.pc;
        frame.stack = stack;
        // Mark done first so re-entrant calls see a finished generator.
        state.done = true;
        zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
        let roots = [generator];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        crate::unwind::GENERATOR_RETURN_PENDING.with(|flag| flag.set(true));
        let outcome = self.resume_suspended(frame);
        crate::unwind::GENERATOR_RETURN_PENDING.with(|flag| flag.set(false));
        match outcome {
            ExecutionOutcome::Thrown(value) if crate::unwind::is_generator_return(&value) => {
                Ok(Value::Bool(true))
            }
            ExecutionOutcome::Thrown(value) if !throwing => Err(RunError::Thrown(value)),
            ExecutionOutcome::Fault(error) => Err(error),
            _ => Ok(Value::Bool(true)),
        }
    }

    /// The `done` side of the protocol for-of path: prefetch the next value into
    /// the wrapper's cache (unless one is already pending or the iterator is
    /// exhausted) and report whether iteration is complete.
    fn protocol_done(&mut self, wrapper: zun_gc::Obj) -> Result<Value, RunError> {
        let roots = [wrapper];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let has = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(roots[0], "__zun_has").and_then(|any| any.as_bool())
        })
        .unwrap_or(false);
        let done_cached = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(roots[0], "__zun_done").and_then(|any| any.as_bool())
        })
        .unwrap_or(false);
        if has || done_cached {
            return Ok(Value::Bool(done_cached));
        }
        let it = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(roots[0], "__zun_it").and_then(|any| any.as_object())
        })
        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
        .ok_or(RunError::BadToken)?;
        let (value, done) = self.protocol_step(it)?;
        let boxed = if done {
            zun_core::Any::void()
        } else {
            zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::null)
        };
        zun_gc::with_current_heap_mut(|heap| {
            heap.dynamic_set(roots[0], "__zun_cur", boxed);
            heap.dynamic_set(roots[0], "__zun_done", zun_core::Any::bool(done));
            heap.dynamic_set(roots[0], "__zun_has", zun_core::Any::bool(true));
        });
        Ok(Value::Bool(done))
    }

    /// The `next` side of the protocol for-of path: hand back the value the
    /// matching `protocol_done` prefetched and clear the cache slot.
    fn protocol_next(&mut self, wrapper: zun_gc::Obj) -> Result<Value, RunError> {
        let current = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(wrapper, "__zun_cur")
                .and_then(|any| Value::unbox_any(any, heap))
        })
        .unwrap_or(Value::Void);
        zun_gc::with_current_heap_mut(|heap| {
            heap.dynamic_set(wrapper, "__zun_has", zun_core::Any::bool(false));
        });
        Ok(current)
    }

    /// Invoke a generator method value (`gen.next` / `gen.return` / `gen.throw`)
    /// obtained by reading the property off a generator object.
    /// Allocate a fresh `{ value, done }` iterator-result object.
    fn make_iter_result(&mut self, value: Value, done: bool) -> Value {
        let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
            .unwrap_or_else(zun_core::Any::void);
        let value_root: Vec<zun_gc::Obj> = boxed
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
            .into_iter()
            .collect();
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&value_root) };
        let result = zun_gc::with_current_heap_mut(|heap| {
            let result = heap.alloc_dynamic();
            let value_any = match value_root.first() {
                Some(object) => zun_core::Any::object(object.addr() as usize),
                None => boxed,
            };
            heap.dynamic_set(result, "value", value_any);
            heap.dynamic_set(result, "done", zun_core::Any::bool(done));
            result
        });
        Value::Any(zun_core::Any::object(result.addr() as usize))
    }

    /// `generator.return(value)`: resume a suspended generator as a return
    /// completion so its `finally` clauses run. A `finally` may override the
    /// result with its own `return` or suspend the completion with a `yield`.
    fn generator_return(&mut self, generator: zun_gc::Obj, value: Value) -> Result<Value, RunError> {
        let mut state = zun_gc::with_current_heap_mut(|heap| heap.generator_state_take(generator))
            .ok_or(RunError::BadToken)?;
        let saved = if state.done { None } else { state.frame.take() };
        state.has_current = false;
        state.current = zun_core::Any::void();
        let Some(saved) = saved else {
            state.done = true;
            zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
            return Ok(self.make_iter_result(value, true));
        };
        let locals = zun_gc::with_current_heap(|heap| {
            saved.locals.iter().map(|v| gen_unbox(*v, heap)).collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let stack = zun_gc::with_current_heap(|heap| {
            saved.eval_stack.iter().map(|v| gen_unbox(*v, heap)).collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let mut frame = ExecutionFrame::new(state.method_rid, locals);
        frame.pc = saved.pc;
        frame.stack = stack;
        // The state stays taken while the body runs, so a re-entrant call
        // finds no live generator.
        let roots = [generator];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        crate::unwind::GENERATOR_RETURN_PENDING.with(|flag| flag.set(true));
        let previous_value = crate::unwind::GENERATOR_RETURN_VALUE
            .with(|slot| slot.borrow_mut().replace(value.clone()));
        let outcome = self.resume_suspended(frame);
        crate::unwind::GENERATOR_RETURN_VALUE.with(|slot| *slot.borrow_mut() = previous_value);
        crate::unwind::GENERATOR_RETURN_PENDING.with(|flag| flag.set(false));
        let generator = roots[0];
        let result = match outcome {
            ExecutionOutcome::Thrown(thrown) if crate::unwind::is_generator_return(&thrown) => {
                state.done = true;
                self.make_iter_result(value, true)
            }
            ExecutionOutcome::Complete(returned) => {
                state.done = true;
                self.make_iter_result(returned, true)
            }
            ExecutionOutcome::Suspended(frame) => {
                let yielded = self.yielded.take().ok_or(RunError::Malformed(
                    "generator suspended without yielding a value".into(),
                ))?;
                let locals = zun_gc::with_current_heap_mut(|heap| {
                    frame.locals.iter().map(|v| gen_box(v, heap)).collect::<Option<Vec<_>>>()
                })
                .ok_or(RunError::BadToken)?;
                let eval_stack = zun_gc::with_current_heap_mut(|heap| {
                    frame.stack.iter().map(|v| gen_box(v, heap)).collect::<Option<Vec<_>>>()
                })
                .ok_or(RunError::BadToken)?;
                state.frame = Some(zun_gc::GeneratorFrame {
                    method_rid: frame.method_rid,
                    pc: frame.pc,
                    locals,
                    eval_stack,
                });
                state.pending_return = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                    .ok_or(RunError::BadToken)?;
                state.has_pending_return = true;
                zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
                return Ok(self.make_iter_result(yielded, false));
            }
            ExecutionOutcome::Thrown(thrown) => {
                state.done = true;
                zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
                return Err(RunError::Thrown(thrown));
            }
            ExecutionOutcome::Fault(error) => return Err(error),
        };
        zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
        Ok(result)
    }

    pub(crate) fn call_generator_method(
        &mut self,
        kind: u32,
        generator: zun_gc::Obj,
        args: &[Value],
    ) -> Result<Value, RunError> {
        let first = match args.first() {
            Some(Value::Any(any)) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                .unwrap_or(Value::Void),
            Some(other) => other.clone(),
            None => Value::Void,
        };
        match kind {
            0 => {
                crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = args.first().map(|_| first.clone()));
                let result = self.builtin_iterator_result(generator);
                crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = None);
                Ok(result?.unwrap_or(Value::Void))
            }
            1 => self.generator_return(generator, first),
            // `iterable[Symbol.iterator]` read off a built-in iterable, called.
            3 => self.call_managed_collection("symbolIteratorCall", &[Value::Object(generator)]),
            _ => {
                let suspended = zun_gc::with_current_heap(|heap| {
                    heap.generator_state(generator)
                        .map(|state| !state.done && !state.has_current && state.frame.is_some())
                })
                .unwrap_or(false);
                if suspended {
                    // Raise the exception at the suspended `yield`; a surrounding
                    // try/catch in the generator may handle it and keep going.
                    crate::unwind::GENERATOR_THROW_PENDING.with(|slot| *slot.borrow_mut() = Some(first));
                    let result = self.builtin_iterator_result(generator);
                    crate::unwind::GENERATOR_THROW_PENDING.with(|slot| *slot.borrow_mut() = None);
                    return Ok(result?.unwrap_or(Value::Void));
                }
                self.close_generator(generator, true)?;
                Err(RunError::Thrown(first))
            }
        }
    }

    /// `iterator.next()` for a built-in iterator object reached through a dynamic
    /// member call (`[10, 20].values().next()`): advance it and return a fresh
    /// `{ value, done }` result object. Returns `None` for a non-iterator so the
    /// dynamic-call dispatch carries on.
    fn builtin_iterator_result(
        &mut self,
        it: zun_gc::Obj,
    ) -> Result<Option<Value>, RunError> {
        let is_builtin = zun_gc::with_current_heap(|heap| {
            heap.array_iterator_done(it).is_some()
                || heap.set_iterator_done(it).is_some()
                || heap.map_iterator_done(it).is_some()
        });
        let is_generator = zun_gc::with_current_heap(|heap| heap.generator_state(it).is_some());
        if !is_builtin && !is_generator {
            return Ok(None);
        }
        let (value, done) = if is_generator {
            let finished = self.call_managed_collection("generatorIteratorDone", &[Value::Object(it)])?;
            if matches!(finished, Value::Bool(true)) {
                // The first completion reports the generator's return value
                // (what `yield*` evaluates to); later ones report undefined.
                let returned = zun_gc::with_current_heap_mut(|heap| {
                    heap.generator_state_mut(it).map(|state| std::mem::replace(&mut state.current, zun_core::Any::void()))
                });
                let value = returned
                    .and_then(|any| zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)))
                    .unwrap_or(Value::Void);
                (value, true)
            } else {
                (self.call_managed_collection("generatorIteratorNext", &[Value::Object(it)])?, false)
            }
        } else {
            self.protocol_step(it)?
        };
        let boxed = if done && !is_generator {
            zun_core::Any::void()
        } else {
            zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::null)
        };
        let value_root: Vec<zun_gc::Obj> = boxed
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
            .into_iter()
            .collect();
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&value_root) };
        let result = zun_gc::with_current_heap_mut(|heap| {
            let result = heap.alloc_dynamic();
            let value_any = match value_root.first() {
                Some(object) => zun_core::Any::object(object.addr() as usize),
                None => boxed,
            };
            heap.dynamic_set(result, "value", value_any);
            heap.dynamic_set(result, "done", zun_core::Any::bool(done));
            result
        });
        Ok(Some(Value::Any(zun_core::Any::object(result.addr() as usize))))
    }

    /// Drive `source` fully through the iteration protocol, handing each boxed
    /// value to `sink` to deposit into `target`. `target` (and the look-ahead
    /// wrapper) stay rooted across the drive, and the possibly-relocated target
    /// is returned. Backs the `Set`/`Map` iterable constructors and array spread
    /// for arbitrary user iterables.
    pub(crate) fn drive_iterable<F>(
        &mut self,
        source: zun_gc::Obj,
        target: zun_gc::Obj,
        mut sink: F,
    ) -> Result<zun_gc::Obj, RunError>
    where
        F: FnMut(&mut zun_gc::Heap, zun_gc::Obj, zun_core::Any),
    {
        let setup = [target];
        let wrapper = {
            let _scope = unsafe { zun_gc::NativeRootScope::enter(&setup) };
            Self::value_object(&self.protocol_iterator_new(source)?)
                .ok_or(RunError::BadToken)?
        };
        let roots = [wrapper, setup[0]];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        loop {
            if matches!(self.protocol_done(roots[0])?, Value::Bool(true)) {
                break;
            }
            let value = self.protocol_next(roots[0])?;
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::null);
            zun_gc::with_current_heap_mut(|heap| sink(heap, roots[1], boxed));
        }
        Ok(roots[1])
    }

    /// Read `name` from the explicit prototype chain of `receiver` (not the
    /// receiver itself): data slots, function slots, and accessors (invoked
    /// with `receiver` as `this`).
    pub(crate) fn prototype_chain_get(
        &mut self,
        receiver: zun_gc::Obj,
        name: &str,
    ) -> Result<Option<Value>, RunError> {
        let mut current = receiver;
        for hop in 0..256 {
            // A typed class instance inherits from its class's prototype object.
            let class_proto = if hop == 0 { self.instance_class_prototype(receiver) } else { None };
            let Some(next) = class_proto.or_else(|| zun_gc::with_current_heap(|heap| {
                heap.dynamic_prototype(current)
                    .and_then(|any| any.as_object())
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
            })) else {
                return Ok(None);
            };
            current = next;
            let getter =
                zun_gc::with_current_heap(|heap| heap.dynamic_accessor_getter(current, name));
            if let Some(getter) = getter {
                let args = self.with_this_arg(getter, receiver, Vec::new());
                return self.invoke_function_value(getter, args).map(Some);
            }
            let found = zun_gc::with_current_heap(|heap| {
                if let Some(any) = heap.dynamic_get(current, name) {
                    Some(Value::Any(any))
                } else {
                    heap.dynamic_get_function(current, name).map(Value::Function)
                }
            });
            if found.is_some() {
                return Ok(found);
            }
        }
        Ok(None)
    }

    /// Read member `name` of a dynamic object for `o[name]`: own accessor,
    /// own data slot, own function slot, then the explicit prototype chain.
    /// The `obj.constructor` record (`{ name, __classDesc }`) of a typed class
    /// instance, or one naming `Array`/`Object` for arrays and plain objects
    /// without an own `constructor`.
    /// `{ name, __classDesc }` for a built-in constructor, with the descriptor
    /// read off the realm global so `x.constructor === Array` holds.
    pub(crate) fn builtin_constructor_record(class_name: &str) -> zun_gc::Obj {
        zun_gc::with_current_heap_mut(|heap| {
            let record = heap.alloc_dynamic();
            let name_any = Value::Str(class_name.into()).box_any(heap).unwrap_or_else(zun_core::Any::null);
            heap.dynamic_set(record, "name", name_any);
            let global = heap.global_object();
            if let Some(desc) = heap.dynamic_get(global, class_name) {
                if desc.as_class_descriptor().is_some() {
                    heap.dynamic_set(record, "__classDesc", desc);
                }
            }
            record
        })
    }

    pub(crate) fn constructor_record(&mut self, obj: zun_gc::Obj) -> Option<zun_gc::Obj> {
        let builtin_name = zun_gc::with_current_heap(|heap| builtin_constructor_name(heap, obj));
        if let Some(class_name) = builtin_name {
            return Some(Self::builtin_constructor_record(class_name));
        }
        let typed = zun_gc::with_current_heap(|heap| {
            (!heap.is_dynamic_object(obj) || heap.is_instance_bag(obj)) && heap.array_len(obj).is_none()
        });
        let type_rid = obj.type_rid();
        if !typed || type_rid < 1 {
            return None;
        }
        let def = self.module.tables.type_defs.get(type_rid as usize - 1)?;
        let class_name = self.module.strings.get(def.name).to_string();
        let token = type_rid | zun_natives::USER_CLASS_FLAG;
        Some(zun_gc::with_current_heap_mut(|heap| {
            let record = heap.alloc_dynamic();
            let name_any = Value::Str(class_name.into()).box_any(heap).unwrap_or_else(zun_core::Any::null);
            heap.dynamic_set(record, "name", name_any);
            heap.dynamic_set(record, "__classDesc", zun_core::Any::class_descriptor(token));
            record
        }))
    }

    pub(crate) fn dynamic_member_get(
        &mut self,
        object: zun_gc::Obj,
        name: &str,
    ) -> Result<Value, RunError> {
        if name == "constructor" {
            if let Some(record) = self.constructor_record(object) {
                return Ok(Value::Object(record));
            }
        }
        if name == "size" {
            let size = zun_gc::with_current_heap(|heap| heap.map_len(object).or_else(|| heap.set_len(object)));
            if let Some(size) = size {
                return Ok(Value::Float(size as f64, zun_il::NumKind::F64));
            }
        }
        let getter = zun_gc::with_current_heap(|heap| heap.dynamic_accessor_getter(object, name));
        if let Some(getter) = getter {
            let args = self.with_this_arg(getter, object, Vec::new());
            return self.invoke_function_value(getter, args);
        }
        let own = zun_gc::with_current_heap(|heap| {
            if let Some(any) = heap.dynamic_get(object, name) {
                Some(Value::Any(any))
            } else {
                heap.dynamic_get_function(object, name).map(Value::Function)
            }
        });
        if let Some(value) = own {
            return Ok(value);
        }
        // A typed class instance exposes its declared fields and getters.
        let typed = zun_gc::with_current_heap(|heap| {
            (!heap.is_dynamic_object(object) || heap.is_instance_bag(object))
                && heap.array_len(object).is_none()
                && object.type_rid() >= 1
        });
        if typed {
            if let Some(value) = self.instance_member_get(object, name)? {
                return Ok(value);
            }
        }
        Ok(self.prototype_chain_get(object, name)?.unwrap_or(Value::Void))
    }

    /// Arguments for invoking an object-literal accessor/method closure: a
    /// closure that uses `this` (marked `@this`) takes the receiver first.
    /// `proxy.key` read through an `any` receiver: the handler's `get` trap
    /// (called as `(target, key, receiver)` with the handler as `this`), or the
    /// target's own property when there is none.
    pub(crate) fn proxy_dynamic_get(&mut self, proxy: zun_gc::Obj, key: &str) -> Result<Value, RunError> {
        let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy)) else {
            return Err(RunError::BadToken);
        };
        let trap = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "get"));
        let result = match trap {
            Some(callback) => {
                let handler = zun_gc::with_current_heap(|heap| heap.proxy_handler(proxy)).unwrap_or(target);
                let args = vec![Value::Object(target), Value::Str(key.into()), Value::Object(proxy)];
                self.invoke_function_value(callback, self.with_this_arg(callback, handler, args))?
            }
            None => zun_gc::with_current_heap(|heap| {
                heap.dynamic_get(target, key)
                    .and_then(|any| Value::unbox_any(any, heap))
                    .unwrap_or(Value::Void)
            }),
        };
        Ok(match result {
            Value::Function(_) | Value::Any(_) => result,
            other => zun_gc::with_current_heap_mut(|heap| other.box_any(heap)).map(Value::Any).unwrap_or(Value::Void),
        })
    }

    /// `proxy.key = value` through an `any` receiver: the `set` trap
    /// (`(target, key, value, receiver)`) or a plain store on the target.
    pub(crate) fn proxy_dynamic_set(&mut self, proxy: zun_gc::Obj, key: &str, value: Value) -> Result<(), RunError> {
        let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy)) else {
            return Err(RunError::BadToken);
        };
        let trap = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "set"));
        match trap {
            Some(callback) => {
                let handler = zun_gc::with_current_heap(|heap| heap.proxy_handler(proxy)).unwrap_or(target);
                let boxed = match &value {
                    Value::Function(_) | Value::Any(_) => value.clone(),
                    other => zun_gc::with_current_heap_mut(|heap| other.box_any(heap)).map(Value::Any).unwrap_or(Value::Void),
                };
                let args = vec![Value::Object(target), Value::Str(key.into()), boxed, Value::Object(proxy)];
                self.invoke_function_value(callback, self.with_this_arg(callback, handler, args))?;
            }
            None => {
                if let Some(any) = zun_gc::with_current_heap_mut(|heap| value.box_any(heap)) {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(target, key.to_owned(), any);
                    });
                }
            }
        }
        Ok(())
    }

    /// `length` / `name` of a class symbol-keyed method value (for example
    /// `Date.prototype[Symbol.toPrimitive]`), read off the method metadata.
    /// Other function values keep their existing expando-bag behavior.
    pub(crate) fn function_meta(&self, handle: zun_core::FunctionHandle, name: &str) -> Option<Value> {
        // A promise resolving function (`resolve` / `reject`): length 1, empty name.
        if self.settler_of(handle).is_some() {
            return Some(if name == "name" {
                Value::Str("".into())
            } else {
                Value::Float(1.0, zun_il::NumKind::F64)
            });
        }
        if handle.is_bound() || handle.generator_method().is_some() {
            return None;
        }
        let rid = handle.method_rid();
        let method = self.module.tables.method_defs.get((rid as usize).wrapping_sub(1))?;
        let method_name = self.module.strings.get(method.name);
        if !method_name.starts_with("[symbol]") {
            return None;
        }
        if name == "name" {
            let symbol = self
                .class_symbol_methods
                .iter()
                .find(|(_, candidate)| candidate.method_rid() == rid)
                .map(|(key, _)| key.1)?;
            let well_known = zun_core::WellKnownSymbol::ALL.get(symbol as usize)?;
            return Some(Value::Str(format!("[{}]", well_known.description()).into()));
        }
        let body = read_body(&self.module.code, method.body_offset).ok()?;
        let receiver = usize::from(method_name.contains("@this"));
        let length = (body.param_count as usize).saturating_sub(receiver);
        Some(Value::Float(length as f64, zun_il::NumKind::F64))
    }

    pub(crate) fn with_this_arg(
        &self,
        handle: zun_core::FunctionHandle,
        receiver: zun_gc::Obj,
        mut args: Vec<Value>,
    ) -> Vec<Value> {
        if !handle.is_bound() {
            let takes_this = self
                .module
                .tables
                .method_defs
                .get((handle.method_rid() as usize).wrapping_sub(1))
                .map(|method| self.module.strings.get(method.name).contains("@this"))
                .unwrap_or(false);
            if takes_this {
                args.insert(0, Value::Object(receiver));
            }
        }
        args
    }

    /// Invoke a function value with the given positional arguments. The engine
    /// behind `apply`; exceptions propagate as `RunError::Thrown` (matching the
    /// proxy-trap invocation path).
    pub(crate) fn invoke_function_value(
        &mut self,
        handle: zun_core::FunctionHandle,
        call_args: Vec<Value>,
    ) -> Result<Value, RunError> {
        if handle.builtin_method().is_some() {
            return Err(RunError::Native(
                "TypeError: RegExp.prototype method called on an incompatible receiver".into(),
            ));
        }
        if let Some(kind) = handle.generator_method() {
            let generator = handle
                .environment()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                .ok_or(RunError::BadToken)?;
            return self.call_generator_method(kind, generator, &call_args);
        }
        if let Some((promise, mode)) = self.settler_of(handle) {
            return self.invoke_promise_settler(promise, mode, call_args);
        }
        let (method_rid, final_args) = self.resolve_call_target(handle, call_args)?;
        // Calling a generator function value creates the generator object.
        if self.method_contains_yield(method_rid) {
            return self.create_generator(method_rid, &final_args);
        }
        self.call(method_rid, final_args, None)
    }

    /// Invoke a dynamic callback with ECMAScript "extra arguments ignored"
    /// semantics: after the `bind`/closure environment is threaded, the
    /// positional argument vector is truncated to the callback's declared
    /// parameter count before dispatch. A callback declaring fewer parameters
    /// than the host supplies (`(value, index, array)`) therefore never has its
    /// own local slots clobbered, and one declaring more sees `undefined` in the
    /// surplus slots (padded by [`Interp::call`]).
    /// The kind-agnostic second-generation typed-array surface: bulk `set`,
    /// `slice`/`copyWithin`, the functional methods, and the `from`/`of`
    /// statics. Returns `None` when `name` is not one of these, so the caller
    /// carries on with the rest of its dispatch.
    ///
    /// The element kind is read from the view at run time (for instance
    /// methods) or decoded from a table-order discriminant argument (for the
    /// statics), so there is one handler per method rather than one per kind.
    /// Callback methods re-enter the interpreter and root their view/result
    /// across the call, mirroring the `Array` functional methods.
    fn call_typed_array_ext(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        let obj = |value: &Value| match value {
            Value::Object(object) => Some(*object),
            Value::Any(any) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
            _ => None,
        };
        // Every value a source (array or typed array) holds, as interpreter
        // values — the input shape for `set`/`from`/`of`.
        let source_elements = |src: zun_gc::Obj| -> Option<Vec<Value>> {
            if let Some(len) = crate::typed_arrays::element_len(src) {
                let mut out = Vec::with_capacity(len);
                for index in 0..len {
                    out.push(crate::typed_arrays::read_element(src, index)?);
                }
                return Some(out);
            }
            let len = zun_gc::with_current_heap(|heap| heap.array_len(src))?;
            let mut out = Vec::with_capacity(len);
            for index in 0..len {
                let any = zun_gc::with_current_heap(|heap| heap.array_get(src, index))?;
                let value = zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))?;
                out.push(value);
            }
            Some(out)
        };
        // A relative index (JS `slice`/`copyWithin` style): negative counts
        // from the end; the result is clamped into `0..=length`.
        let relative = |value: i64, length: usize| -> usize {
            if value < 0 {
                (length as i64 + value).max(0) as usize
            } else {
                (value as usize).min(length)
            }
        };
        match name {
            "typedArraySetFrom" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let src = obj(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let elements = source_elements(src).ok_or(RunError::BadToken)?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                // Out of range reports `false` rather than throwing a
                // RangeError (the whole set surface here yields a bool).
                if offset.checked_add(elements.len()).is_none_or(|end| end > length) {
                    return Ok(Value::Bool(false));
                }
                for (index, value) in elements.iter().enumerate() {
                    if crate::typed_arrays::write_element(view, offset + index, value)
                        != Some(true)
                    {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            })()),
            "typedArraySlice" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let (start, end) = (int_arg(args.get(1))?, int_arg(args.get(2))?);
                let element = crate::typed_arrays::element_of(view).ok_or(RunError::BadToken)?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let begin = relative(start, length);
                let finish = relative(end, length).max(begin);
                // Read the source range before allocating the result so the
                // view is never read across the allocation that could move it.
                let mut values = Vec::with_capacity(finish - begin);
                for index in begin..finish {
                    values.push(crate::typed_arrays::read_element(view, index).ok_or(RunError::BadToken)?);
                }
                let result = crate::typed_arrays::alloc_of(element, values.len())
                    .ok_or(RunError::BadToken)?;
                for (index, value) in values.iter().enumerate() {
                    crate::typed_arrays::write_element(result, index, value);
                }
                Ok(Value::Object(result))
            })()),
            "typedArrayCopyWithin" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target = int_arg(args.get(1))?;
                let start = int_arg(args.get(2))?;
                let end = int_arg(args.get(3))?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let dest = relative(target, length);
                let from = relative(start, length);
                let to = relative(end, length).max(from);
                // The copied count is bounded by what fits at the destination.
                let count = (to - from).min(length - dest);
                // Snapshot the source range first: source and destination may
                // overlap, so a straight element-by-element copy would clobber.
                let mut values = Vec::with_capacity(count);
                for index in 0..count {
                    values.push(crate::typed_arrays::read_element(view, from + index).ok_or(RunError::BadToken)?);
                }
                for (index, value) in values.iter().enumerate() {
                    crate::typed_arrays::write_element(view, dest + index, value);
                }
                Ok(Value::Object(view))
            })()),
            "typedArrayIndexOf" | "typedArrayIncludes" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // The search value arrives boxed to `Any`; unbox it to the
                // concrete numeric value the elements compare against.
                let needle = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .ok_or(RunError::BadToken)?,
                    other => other.clone(),
                };
                let from = int_arg(args.get(2))?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let start = relative(from, length);
                let includes = name == "typedArrayIncludes";
                for index in start..length {
                    let element = crate::typed_arrays::read_element(view, index).ok_or(RunError::BadToken)?;
                    if numeric_same(&element, &needle, includes) {
                        return Ok(if includes {
                            Value::Bool(true)
                        } else {
                            Value::Int(index as i64, zun_il::NumKind::I64)
                        });
                    }
                }
                Ok(if includes {
                    Value::Bool(false)
                } else {
                    Value::Int(-1, zun_il::NumKind::I64)
                })
            })()),
            "typedArrayJoin" => Some((|| {
                let view = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let separator = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(text) => text.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let mut parts = Vec::with_capacity(length);
                for index in 0..length {
                    let element = crate::typed_arrays::read_element(view, index).ok_or(RunError::BadToken)?;
                    let text = zun_gc::with_current_heap_mut(|heap| {
                        element.box_any(heap).and_then(|any| stringify_any(any, heap))
                    })
                    .unwrap_or_default();
                    parts.push(text);
                }
                Ok(Value::Str(parts.join(&separator).into()))
            })()),
            "typedArrayForEach" | "typedArrayMap" | "typedArrayFilter" | "typedArrayFind"
            | "typedArrayFindIndex" | "typedArraySome" | "typedArrayEvery" => {
                Some(self.typed_array_iterating(name, args))
            }
            "typedArrayReduce" => Some(self.typed_array_reduce(args, true, true)),
            "typedArrayReduceNoInit" => Some(self.typed_array_reduce(args, true, false)),
            "typedArrayReduceRight" => Some(self.typed_array_reduce(args, false, true)),
            "typedArrayReduceRightNoInit" => Some(self.typed_array_reduce(args, false, false)),
            "typedArrayFrom" | "typedArrayOf" => Some((|| {
                let src = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let kind = kind_arg(args.get(1))?;
                let elements = source_elements(src).ok_or(RunError::BadToken)?;
                let result = crate::typed_arrays::alloc_of(kind, elements.len())
                    .ok_or(RunError::BadToken)?;
                for (index, value) in elements.iter().enumerate() {
                    crate::typed_arrays::write_element(result, index, value);
                }
                Ok(Value::Object(result))
            })()),
            "typedArrayFromMapped" => Some(self.typed_array_from_mapped(args)),
            "typedArrayTrackingNew" => Some((|| {
                let buffer = obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let kind = kind_arg(args.get(2))?;
                let view = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_typed_array_tracking(buffer, offset, kind)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(view))
            })()),
            _ => None,
        }
    }

    /// The typed-array functional methods whose shape matches the `Array`
    /// ones: `forEach`/`map`/`filter`/`find`/`findIndex`/`some`/`every`. The
    /// element kind is the view's; `map`/`filter` build a new view of that
    /// kind.
    fn typed_array_iterating(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        let view = match args.first() {
            Some(Value::Object(object)) => *object,
            Some(Value::Any(any)) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                .ok_or(RunError::BadToken)?,
            _ => return Err(RunError::BadToken),
        };
        let callback = match args.get(1).ok_or(RunError::BadToken)? {
            Value::Function(handle) => *handle,
            _ => return Err(RunError::BadToken),
        };
        let element = crate::typed_arrays::element_of(view).ok_or(RunError::BadToken)?;
        let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
        // `map` writes into a fresh same-kind view of equal length; `filter`
        // collects kept elements (plain numeric values, GC-safe) and sizes its
        // result after the loop. Both root the source view across callbacks.
        let result = if name == "typedArrayMap" {
            crate::typed_arrays::alloc_of(element, length).ok_or(RunError::BadToken)?
        } else {
            view
        };
        let roots = [view, result];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let mut kept: Vec<Value> = Vec::new();
        for index in 0..length {
            let value = crate::typed_arrays::read_element(roots[0], index).ok_or(RunError::BadToken)?;
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let callback_args = vec![
                Value::Any(boxed),
                Value::Any(zun_core::Any::float(index as f64)),
                Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
            ];
            let produced = self.invoke_callback(callback, callback_args)?;
            match name {
                "typedArrayForEach" => {}
                "typedArrayMap" => {
                    crate::typed_arrays::write_element(roots[1], index, &produced);
                }
                "typedArrayFilter" => {
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        kept.push(value);
                    }
                }
                "typedArrayFind" => {
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        return Ok(value);
                    }
                }
                "typedArrayFindIndex" => {
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        return Ok(Value::Int(index as i64, zun_il::NumKind::I64));
                    }
                }
                "typedArraySome" => {
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        return Ok(Value::Bool(true));
                    }
                }
                "typedArrayEvery" => {
                    if !zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        return Ok(Value::Bool(false));
                    }
                }
                _ => unreachable!(),
            }
        }
        match name {
            "typedArrayForEach" => Ok(Value::Void),
            "typedArrayMap" => Ok(Value::Object(roots[1])),
            "typedArrayFilter" => {
                let result = crate::typed_arrays::alloc_of(element, kept.len())
                    .ok_or(RunError::BadToken)?;
                for (index, value) in kept.iter().enumerate() {
                    crate::typed_arrays::write_element(result, index, value);
                }
                Ok(Value::Object(result))
            }
            "typedArrayFind" => Ok(Value::Void),
            "typedArrayFindIndex" => Ok(Value::Int(-1, zun_il::NumKind::I64)),
            "typedArraySome" => Ok(Value::Bool(false)),
            "typedArrayEvery" => Ok(Value::Bool(true)),
            _ => unreachable!(),
        }
    }

    /// `typedArray.reduce(callback, initialValue)`. The initial value is
    /// required here; the no-initial form (which seeds from the first element)
    /// is deferred. The accumulator is held in a one-slot GC array so it
    /// survives callback re-entry whether or not it is an object.
    /// `reduce`/`reduceRight` over a typed array. `forward` folds left-to-right
    /// (`reduce`) or right-to-left (`reduceRight`); when `has_init` is false the
    /// accumulator seeds from the first/last element and an empty typed array
    /// throws a TypeError-equivalent (NUCLEUS #19).
    fn typed_array_reduce(
        &mut self,
        args: &[Value],
        forward: bool,
        has_init: bool,
    ) -> Result<Value, RunError> {
        let view = match args.first() {
            Some(Value::Object(object)) => *object,
            Some(Value::Any(any)) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                .ok_or(RunError::BadToken)?,
            _ => return Err(RunError::BadToken),
        };
        let callback = match args.get(1).ok_or(RunError::BadToken)? {
            Value::Function(handle) => *handle,
            _ => return Err(RunError::BadToken),
        };
        let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
        // Index of the step `step` visits (0..length iterations), walking from
        // the appropriate end.
        let at = |step: usize| if forward { step } else { length - 1 - step };
        // Seed the accumulator: the initial value, or the first/last element.
        let (seed, start_step) = if has_init {
            (args.get(2).ok_or(RunError::BadToken)?.clone(), 0usize)
        } else {
            if length == 0 {
                return match self.new_error_object("TypeError", "Reduce of empty array with no initial value") {
                        Some(error) => Err(RunError::Thrown(Value::Object(error))),
                        None => Err(RunError::BadToken),
                    };
            }
            let seed = crate::typed_arrays::read_element(view, at(0)).ok_or(RunError::BadToken)?;
            (seed, 1usize)
        };
        let accumulator = zun_gc::with_current_heap_mut(|heap| {
            let holder = heap.alloc_array(1);
            if let Some(boxed) = seed.box_any(heap) {
                heap.array_set(holder, 0, boxed);
            }
            holder
        });
        let roots = [view, accumulator];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        for step in start_step..length {
            let index = at(step);
            let element = crate::typed_arrays::read_element(roots[0], index).ok_or(RunError::BadToken)?;
            let boxed_element = zun_gc::with_current_heap_mut(|heap| element.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let acc = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
                .ok_or(RunError::BadToken)?;
            let callback_args = vec![
                Value::Any(acc),
                Value::Any(boxed_element),
                Value::Any(zun_core::Any::float(index as f64)),
                Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
            ];
            let next = self.invoke_callback(callback, callback_args)?;
            let boxed = zun_gc::with_current_heap_mut(|heap| next.box_any(heap))
                .ok_or(RunError::BadToken)?;
            zun_gc::with_current_heap_mut(|heap| heap.array_set(roots[1], 0, boxed));
        }
        let result = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
            .ok_or(RunError::BadToken)?;
        zun_gc::with_current_heap(|heap| Value::unbox_any(result, heap)).ok_or(RunError::BadToken)
    }

    /// `TypedArray.from(source, mapFn)`: the mapped static. Reads the source
    /// into values, runs each through the callback, and stores the result into
    /// a fresh view of the discriminant-named kind.
    fn typed_array_from_mapped(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let src = match args.first() {
            Some(Value::Object(object)) => *object,
            Some(Value::Any(any)) => any
                .as_object()
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                .ok_or(RunError::BadToken)?,
            _ => return Err(RunError::BadToken),
        };
        let callback = match args.get(1).ok_or(RunError::BadToken)? {
            Value::Function(handle) => *handle,
            _ => return Err(RunError::BadToken),
        };
        let kind = kind_arg(args.get(2))?;
        // Read the source eagerly into plain values (GC-safe across callbacks),
        // then build and fill the result.
        let elements = {
            let len = if let Some(len) = crate::typed_arrays::element_len(src) {
                len
            } else {
                zun_gc::with_current_heap(|heap| heap.array_len(src)).ok_or(RunError::BadToken)?
            };
            let mut out = Vec::with_capacity(len);
            for index in 0..len {
                let value = if crate::typed_arrays::element_len(src).is_some() {
                    crate::typed_arrays::read_element(src, index).ok_or(RunError::BadToken)?
                } else {
                    let any = zun_gc::with_current_heap(|heap| heap.array_get(src, index))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                        .ok_or(RunError::BadToken)?
                };
                out.push(value);
            }
            out
        };
        let result = crate::typed_arrays::alloc_of(kind, elements.len()).ok_or(RunError::BadToken)?;
        let roots = [result];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        for (index, value) in elements.iter().enumerate() {
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let callback_args = vec![
                Value::Any(boxed),
                Value::Any(zun_core::Any::float(index as f64)),
            ];
            let produced = self.invoke_callback(callback, callback_args)?;
            crate::typed_arrays::write_element(roots[0], index, &produced);
        }
        Ok(Value::Object(roots[0]))
    }

    /// The logical element at `index`: an accessor element (installed by
    /// `Object.defineProperty(array, "0", { get })`) runs its getter; `None`
    /// past the end of the array. Holes come back as `Any::hole()`.
    pub(crate) fn array_element_value(
        &mut self,
        array: zun_gc::Obj,
        index: usize,
    ) -> Result<Option<zun_core::Any>, RunError> {
        let Some(element) = zun_gc::with_current_heap(|heap| heap.array_get(array, index)) else {
            return Ok(None);
        };
        let Some(addr) = element.as_object() else {
            return Ok(Some(element));
        };
        let record = unsafe { zun_gc::Obj::from_addr(addr as *mut u64) };
        let getter = zun_gc::with_current_heap(|heap| {
            heap.dynamic_has(record, "__elemAcc").then(|| heap.dynamic_accessor_getter(record, "v"))
        });
        match getter {
            None => Ok(Some(element)),
            Some(None) => Ok(Some(zun_core::Any::void())),
            Some(Some(getter)) => {
                let args = self.with_this_arg(getter, array, Vec::new());
                let produced = self.invoke_function_value(getter, args)?;
                let boxed = zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                    .ok_or(RunError::BadToken)?;
                Ok(Some(boxed))
            }
        }
    }

    pub(crate) fn invoke_callback(
        &mut self,
        handle: zun_core::FunctionHandle,
        call_args: Vec<Value>,
    ) -> Result<Value, RunError> {
        // Promise settlers and generator-method values are not plain methods.
        if handle.generator_method().is_some() || self.settler_of(handle).is_some() {
            return self.invoke_function_value(handle, call_args);
        }
        let (method_rid, mut final_args) = self.resolve_call_target(handle, call_args)?;
        let target = self
            .module
            .tables
            .method_defs
            .get(method_rid as usize - 1)
            .ok_or(RunError::BadToken)?;
        let body = read_body(&self.module.code, target.body_offset)
            .map_err(|error| RunError::Malformed(error.to_string()))?;
        // A rest-parameter closure packs the surplus arguments itself (in
        // `call`), so it must see all of them.
        if !self.module.strings.get(target.name).ends_with("@rest>") {
            final_args.truncate(body.param_count as usize);
        }
        self.call(method_rid, final_args, None)
    }

    // --- Timers: the macrotask source of the unified event loop (slice 4) ---

    /// The current virtual time (ms), seeded lazily from the real `nowMillis`
    /// clock on first use. The real clock is only the *base*; the loop advances
    /// this virtual clock by fast-forward between macrotasks rather than
    /// sleeping (see [`Self::fire_one_timer`]).
    fn timer_now(&mut self) -> i64 {
        if !self.async_state.timers.seeded {
            let real = real_now_millis(self.output);
            self.async_state.timers.clock = real;
            self.async_state.timers.seeded = true;
        }
        self.async_state.timers.clock
    }

    /// Arm a timer: allocate a scheduler [`TaskId`] (which doubles as the
    /// numeric handle), compute its due time from the current virtual clock,
    /// and push it onto the timer heap. `period` is `Some` for an interval.
    pub(super) fn timer_arm(&mut self, callback: Value, delay: i64, period: Option<i64>) -> u32 {
        let now = self.timer_now();
        let task = TaskId(self.async_state.next_task);
        self.async_state.next_task += 1;
        let seq = self.async_state.timers.next_seq;
        self.async_state.timers.next_seq += 1;
        self.async_state.timers.entries.push(TimerEntry {
            task,
            due: now.saturating_add(delay.max(0)),
            seq,
            period,
            callback,
            args: Vec::new(),
        });
        task.0
    }

    /// Clear a timer by handle: cancel it on the scheduler's task queue (which
    /// also stops an interval re-arming) and drop any still-pending entry.
    pub(super) fn timer_clear(&mut self, handle: u32) {
        let task = TaskId(handle);
        self.async_state.scheduler.cancel_task(task);
        self.async_state.timers.entries.retain(|timer| timer.task != task);
    }

    /// `queueMicrotask(cb)`: enqueue a callback directly on the reaction
    /// (microtask) queue, so it runs before any macrotask on the next drain.
    /// Modelled as a `then` on an already-resolved promise — the same
    /// enqueue-not-run-inline discipline every reaction follows.
    pub(super) fn queue_microtask(
        &mut self,
        callback: zun_core::FunctionHandle,
    ) -> Result<(), RunError> {
        let promise = zun_gc::with_current_heap_mut(|heap| {
            let promise = heap.alloc_promise();
            heap.promise_resolve(promise, zun_core::Any::null());
            promise
        });
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        let roots = [promise, target];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        self.attach_reaction(
            roots[0],
            roots[1],
            ReactionKind::Microtask { callback: Value::Function(callback) },
        );
        Ok(())
    }

    /// Index of the earliest-due pending timer (min by `(due, seq)`), or `None`
    /// when the heap is empty. Cleared timers are already purged from the heap.
    fn earliest_timer_index(&self) -> Option<usize> {
        let entries = &self.async_state.timers.entries;
        if entries.is_empty() {
            return None;
        }
        let mut best = 0;
        for index in 1..entries.len() {
            let candidate = &entries[index];
            let current = &entries[best];
            if (candidate.due, candidate.seq) < (current.due, current.seq) {
                best = index;
            }
        }
        Some(best)
    }

    /// Fire the single earliest-due ready timer as one macrotask, returning
    /// whether a timer was fired (`false` when none remain). The earliest timer
    /// is always fired regardless of real wall-clock time: if it is not yet due
    /// the virtual clock fast-forwards to its due time (no real sleeping), which
    /// is what makes a timed program terminate deterministically. A due timer is
    /// scheduled on the scheduler's task tier and consumed through it, so the
    /// scheduler stays the authority on micro-before-macro ordering. An interval
    /// re-arms itself (at `now + period`) unless it was cleared meanwhile.
    pub(super) fn fire_one_timer(&mut self) -> Result<bool, RunError> {
        let Some(index) = self.earliest_timer_index() else {
            return Ok(false);
        };
        let entry = self.async_state.timers.entries.remove(index);
        if entry.due > self.async_state.timers.clock {
            self.async_state.timers.clock = entry.due;
        }
        self.async_state.scheduler.schedule_task(entry.task);
        let ran = matches!(
            self.async_state.scheduler.next(),
            Some(zun_sched::Ran::Task(task)) if task == entry.task
        );
        if ran {
            if let Value::Function(handle) = &entry.callback {
                // A throw from a timer callback is swallowed (detached-task
                // semantics, as in the microtask drain); a non-throw fault
                // still aborts the turn.
                match self.invoke_callback(*handle, entry.args.clone()) {
                    Ok(_) | Err(RunError::Thrown(_)) => {}
                    Err(other) => return Err(other),
                }
            }
            if let Some(period) = entry.period {
                if !self.async_state.scheduler.is_task_cancelled(entry.task) {
                    let now = self.async_state.timers.clock;
                    let seq = self.async_state.timers.next_seq;
                    self.async_state.timers.next_seq += 1;
                    self.async_state.timers.entries.push(TimerEntry {
                        task: entry.task,
                        due: now.saturating_add(period.max(0)),
                        seq,
                        period: Some(period),
                        callback: entry.callback.clone(),
                        args: entry.args,
                    });
                }
            }
        }
        Ok(true)
    }

    /// Read the settlement of a managed Promise. Errors if the object is not a
    /// Promise cell.
    fn promise_settlement_of(&self, promise: zun_gc::Obj) -> Result<PromiseSettlement, RunError> {
        zun_gc::with_current_heap(|heap| match heap.promise_state(promise) {
            Some(zun_core::ManagedPromiseState::Resolved) => Ok(PromiseSettlement::Resolved(
                heap.promise_result(promise).unwrap_or_else(zun_core::Any::null),
            )),
            Some(zun_core::ManagedPromiseState::Rejected) => Ok(PromiseSettlement::Rejected(
                heap.promise_error(promise).unwrap_or_else(|| "Promise rejected".into()),
                heap.promise_reason(promise),
            )),
            Some(zun_core::ManagedPromiseState::Pending) => Ok(PromiseSettlement::Pending),
            None => Err(RunError::BadToken),
        })
    }

    /// Resolve a managed Promise through the scheduler: the settled value lives
    /// in the heap cell, and the scheduler is told so every reaction and parked
    /// `await` frame bridged to this promise wakes. Returns whether this call
    /// was the settling one (a second settlement is a no-op).
    pub(super) fn settle_promise_resolve(&mut self, promise: zun_gc::Obj, value: zun_core::Any) -> bool {
        let settled = zun_gc::with_current_heap_mut(|heap| heap.promise_resolve(promise, value))
            .unwrap_or(false);
        if settled {
            self.wake_scheduler(promise, Settlement::Fulfilled);
        }
        settled
    }

    /// Reject a managed Promise through the scheduler (see
    /// [`Self::settle_promise_resolve`]). The promise is bridged first so the
    /// scheduler always learns of the rejection — this is what lets a detached
    /// rejection (nothing awaiting/`.catch`-ing it) be tracked as unhandled.
    pub(super) fn settle_promise_reject(&mut self, promise: zun_gc::Obj, error: String) -> bool {
        self.bridge_pid(promise);
        let settled = zun_gc::with_current_heap_mut(|heap| heap.promise_reject(promise, error))
            .unwrap_or(false);
        if settled {
            self.wake_scheduler(promise, Settlement::Rejected);
        }
        settled
    }

    /// Reject a managed Promise with a first-class reason value (an object such
    /// as an `AggregateError`, or any boxed value) plus its display message.
    /// The value is delivered to `catch`/`await`; the message keeps the string
    /// surface working. Bridged first, exactly like [`Self::settle_promise_reject`].
    pub(super) fn settle_promise_reject_value(
        &mut self,
        promise: zun_gc::Obj,
        reason: zun_core::Any,
        message: String,
    ) -> bool {
        self.bridge_pid(promise);
        let settled =
            zun_gc::with_current_heap_mut(|heap| heap.promise_reject_value(promise, reason, message))
                .unwrap_or(false);
        if settled {
            self.wake_scheduler(promise, Settlement::Rejected);
        }
        settled
    }

    /// OrdinaryToPrimitive for an object operand. Consults
    /// `[Symbol.toPrimitive]` (dynamic symbol slot, or the typed-class
    /// symbol-method table for an instance), then the ordered
    /// `valueOf`/`toString` dynamic function slots (hint "string" tries
    /// `toString` first). Returns the first primitive, or `None`.
    /// A typed instance's `valueOf`/`toString` come from `class_named_methods`
    /// (registered by `classSetMethod`) and receive the instance as `this`.
    /// ECMAScript IsLooselyEqual (`==`).
    pub(crate) fn loose_equals(&mut self, left: Value, right: Value) -> Result<bool, RunError> {
        let left = unbox_arg(&left)?;
        let right = unbox_arg(&right)?;
        let nullish = |value: &Value| matches!(value, Value::Void | Value::Null);
        let number = |value: &Value| match value {
            Value::Int(n, _) => Some(*n as f64),
            Value::Float(n, _) => Some(*n),
            _ => None,
        };
        if nullish(&left) || nullish(&right) {
            return Ok(nullish(&left) && nullish(&right));
        }
        if let (Some(a), Some(b)) = (number(&left), number(&right)) {
            return Ok(a == b);
        }
        // BigInt operands (heap objects) compare by mathematical value.
        let big_text = |value: &Value| -> Option<String> {
            match value {
                Value::Object(obj) => zun_gc::with_current_heap(|heap| heap.bigint_value(*obj).map(|v| v.to_string())),
                _ => None,
            }
        };
        let normalize_digits = |text: &str| -> Option<String> {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Some("0".to_string());
            }
            let (negative, digits) = match trimmed.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
            };
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let stripped = digits.trim_start_matches('0');
            if stripped.is_empty() {
                Some("0".to_string())
            } else if negative {
                Some(format!("-{stripped}"))
            } else {
                Some(stripped.to_string())
            }
        };
        match (big_text(&left), big_text(&right)) {
            (Some(a), Some(b)) => return Ok(a == b),
            (Some(a), None) | (None, Some(a)) => {
                let other = if big_text(&left).is_some() { &right } else { &left };
                match other {
                    Value::Str(text) => return Ok(normalize_digits(text).is_some_and(|n| n == a)),
                    Value::Int(..) | Value::Float(..) => {
                        let n = number(other).unwrap_or(f64::NAN);
                        return Ok(n.is_finite()
                            && n.fract() == 0.0
                            && (n.abs() < 1e38)
                            && format!("{}", n as i128) == a);
                    }
                    Value::Bool(flag) => return Ok(a == if *flag { "1" } else { "0" }),
                    Value::Object(obj) => {
                        let primitive = self.to_primitive_default(*obj)?;
                        let (l, r) = if big_text(&left).is_some() { (left.clone(), primitive) } else { (primitive, right.clone()) };
                        return self.loose_equals(l, r);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        match (&left, &right) {
            (Value::Str(a), Value::Str(b)) => return Ok(a == b),
            (Value::Bool(a), Value::Bool(b)) => return Ok(a == b),
            (Value::Object(a), Value::Object(b)) => return Ok(a.addr() == b.addr()),
            (Value::Function(a), Value::Function(b)) => return Ok(a == b),
            _ => {}
        }
        // Booleans become numbers first.
        if let Value::Bool(flag) = &left {
            let as_number = Value::Float(if *flag { 1.0 } else { 0.0 }, zun_il::NumKind::F64);
            return self.loose_equals(as_number, right);
        }
        if let Value::Bool(flag) = &right {
            let as_number = Value::Float(if *flag { 1.0 } else { 0.0 }, zun_il::NumKind::F64);
            return self.loose_equals(left, as_number);
        }
        // Number against string: the string is converted.
        if let (Some(a), Value::Str(text)) = (number(&left), &right) {
            return Ok(a == js_string_to_number(text));
        }
        if let (Value::Str(text), Some(b)) = (&left, number(&right)) {
            return Ok(js_string_to_number(text) == b);
        }
        // An object against a primitive: ToPrimitive(object), then compare again.
        if let Value::Object(obj) = &left {
            if big_text(&left).is_none() {
                let primitive = self.to_primitive_default(*obj)?;
                return self.loose_equals(primitive, right);
            }
        }
        if let Value::Object(obj) = &right {
            if big_text(&right).is_none() {
                let primitive = self.to_primitive_default(*obj)?;
                return self.loose_equals(left, primitive);
            }
        }
        Ok(false)
    }

    /// ToPrimitive with the default hint: arrays join, objects run their
    /// `valueOf`/`toString` hooks, a bare object is `"[object Object]"`.
    fn to_primitive_default(&mut self, obj: zun_gc::Obj) -> Result<Value, RunError> {
        if let Some(text) = array_join_text(obj) {
            return Ok(Value::Str(text.into()));
        }
        Ok(match self.ordinary_to_primitive(obj, "default")? {
            Some(primitive) => primitive,
            None => Value::Str("[object Object]".into()),
        })
    }

    /// ECMAScript ToNumber of an (already unboxed) value, running object
    /// coercion hooks (`valueOf` / `toString` / `[Symbol.toPrimitive]`).
    /// Relational-comparison operand preparation (the two topmost stack values):
    /// non-BigInt objects become primitives (hint "number"); when the result is
    /// not string-versus-string, both sides become numbers. Plain numeric pairs
    /// are left untouched.
    /// `Array.prototype.join` over an array object: object elements run their
    /// own ToString (`toString` / `Symbol.toPrimitive`), nested arrays join with
    /// commas, and `null`/`undefined`/holes render empty. `None` for a non-array.
    pub(crate) fn array_join_dyn(&mut self, obj: zun_gc::Obj, separator: &str) -> Result<Option<String>, RunError> {
        if zun_gc::with_current_heap(|heap| heap.array_len(obj)).is_none() {
            return Ok(None);
        }
        let mut parts: Vec<String> = Vec::new();
        let mut index = 0usize;
        loop {
            let length = zun_gc::with_current_heap(|heap| heap.array_len(obj)).unwrap_or(0);
            if index >= length {
                break;
            }
            let element = zun_gc::with_current_heap(|heap| heap.array_get(obj, index))
                .unwrap_or_else(zun_core::Any::void);
            index += 1;
            let concrete = zun_gc::with_current_heap(|heap| Value::unbox_any(element, heap));
            let text = match concrete {
                Some(Value::Object(inner))
                    if inner != obj
                        && zun_gc::with_current_heap(|heap| heap.bigint_value(inner).is_none()) =>
                {
                    if zun_gc::with_current_heap(|heap| heap.array_len(inner).is_some()) {
                        self.array_join_dyn(inner, ",")?.unwrap_or_default()
                    } else {
                        match self.call_managed_collection("stringCoerce", &[Value::Object(inner)])? {
                            Value::Str(text) => text.to_string(),
                            other => other.display(),
                        }
                    }
                }
                _ => zun_gc::with_current_heap(|heap| stringify_any(element, heap)).unwrap_or_default(),
            };
            parts.push(text);
        }
        Ok(Some(parts.join(separator)))
    }

    pub(crate) fn coerce_relational_operands(&mut self, stack: &mut Vec<Value>) -> Result<(), RunError> {
        let len = stack.len();
        if len < 2 {
            return Ok(());
        }
        let unbox = |v: &Value| -> Value {
            match v {
                Value::Any(any) if any.as_symbol().is_none() => {
                    zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)).unwrap_or_else(|| v.clone())
                }
                other => other.clone(),
            }
        };
        let (a, b) = (unbox(&stack[len - 2]), unbox(&stack[len - 1]));
        let is_plain_object = |v: &Value| match v {
            Value::Object(obj) => zun_gc::with_current_heap(|heap| heap.bigint_value(*obj).is_none()),
            _ => false,
        };
        let bigint_as_f64 = |v: &Value| -> Option<f64> {
            match v {
                Value::Object(obj) => zun_gc::with_current_heap(|heap| {
                    heap.bigint_value(*obj).map(|big| big.to_string().parse::<f64>().unwrap_or(f64::NAN))
                }),
                _ => None,
            }
        };
        // A BigInt against a non-BigInt primitive compares by (approximate) numeric value.
        match (bigint_as_f64(&a), bigint_as_f64(&b)) {
            (Some(x), None) if !is_plain_object(&b) || matches!(b, Value::Str(_)) => {
                let other = match &b {
                    Value::Str(text) => js_string_to_number(text),
                    other => self.to_number_value(other.clone())?,
                };
                stack[len - 2] = Value::Float(x, zun_il::NumKind::F64);
                stack[len - 1] = Value::Float(other, zun_il::NumKind::F64);
                return Ok(());
            }
            (None, Some(y)) if !is_plain_object(&a) || matches!(a, Value::Str(_)) => {
                let other = match &a {
                    Value::Str(text) => js_string_to_number(text),
                    other => self.to_number_value(other.clone())?,
                };
                stack[len - 2] = Value::Float(other, zun_il::NumKind::F64);
                stack[len - 1] = Value::Float(y, zun_il::NumKind::F64);
                return Ok(());
            }
            _ => {}
        }
        let numeric = |v: &Value| matches!(v, Value::Int(..) | Value::Float(..));
        let both_bigint = bigint_as_f64(&a).is_some() && bigint_as_f64(&b).is_some();
        let needs = !both_bigint
            && !(numeric(&a) && numeric(&b))
            && !(matches!(a, Value::Str(_)) && matches!(b, Value::Str(_)));
        if !needs {
            return Ok(());
        }
        let mut prims = Vec::with_capacity(2);
        for value in [a, b] {
            let prim = match value {
                Value::Object(obj) if is_plain_object(&Value::Object(obj)) => {
                    if let Some(text) = self.array_join_dyn(obj, ",")? {
                        Value::Str(text.into())
                    } else {
                        match self.ordinary_to_primitive(obj, "number")? {
                            Some(prim) => prim,
                            None => Value::Str("[object Object]".into()),
                        }
                    }
                }
                other => other,
            };
            prims.push(prim);
        }
        if !prims.iter().all(|p| matches!(p, Value::Str(_))) {
            for prim in prims.iter_mut() {
                let number = self.to_number_value(prim.clone())?;
                *prim = Value::Float(number, zun_il::NumKind::F64);
            }
        }
        let second = prims.pop().ok_or(RunError::BadToken)?;
        let first = prims.pop().ok_or(RunError::BadToken)?;
        stack[len - 2] = first;
        stack[len - 1] = second;
        Ok(())
    }

    pub(crate) fn to_number_value(&mut self, value: Value) -> Result<f64, RunError> {
        Ok(match value {
            Value::Int(number, _) => number as f64,
            Value::Float(number, _) => number,
            Value::Bool(flag) => {
                if flag {
                    1.0
                } else {
                    0.0
                }
            }
            Value::Null => 0.0,
            Value::Void => f64::NAN,
            Value::Str(text) => js_string_to_number(&text),
            Value::Object(obj) if array_join_text(obj).is_some() => {
                js_string_to_number(&array_join_text(obj).unwrap_or_default())
            }
            Value::Object(obj) => {
                if let Some(prim) = self.ordinary_to_primitive(obj, "number")? {
                    match prim {
                        Value::Str(text) => js_string_to_number(&text),
                        other => zun_gc::with_current_heap(|heap| to_number(&other, heap)),
                    }
                } else {
                    f64::NAN
                }
            }
            _ => f64::NAN,
        })
    }

    pub(super) fn ordinary_to_primitive(
        &mut self,
        obj: zun_gc::Obj,
        hint: &str,
    ) -> Result<Option<Value>, RunError> {
        // A BigInt is a primitive: its string form is its decimal digits (the
        // number hint parses them back). Checked first so the heap object's
        // reserved type id never reaches the class-method tables.
        if let Some(text) = zun_gc::with_current_heap(|heap| heap.bigint_value(obj).map(|v| v.to_string())) {
            return Ok(Some(Value::Str(text.into())));
        }
        let tp = zun_core::WellKnownSymbol::ToPrimitive.id();
        let handle = zun_gc::with_current_heap(|heap| heap.dynamic_get_symbol_function(obj, tp))
            .or_else(|| self.class_symbol_methods.get(&(obj.type_rid(), tp.get())).copied());
        if let Some(handle) = handle {
            let hint_value = zun_gc::with_current_heap_mut(|heap| {
                Value::Any(zun_core::Any::string(heap.intern_string(hint.into())))
            });
            let call_args = self.with_this_arg(handle, obj, vec![hint_value]);
            let result = self.invoke_function_value(handle, call_args)?;
            let result = unbox_arg(&result)?;
            if matches!(result, Value::Object(_) | Value::Function(_)) {
                return Err(RunError::Native("TypeError: Cannot convert object to primitive value".into()));
            }
            return Ok(Some(result));
        }
        let mut rejected = false;
        let order: [&str; 2] = if hint == "string" {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        for name in order {
            // A typed class instance's own `valueOf`/`toString` method is bound
            // with the instance as the leading `this` argument; a dynamic
            // object's function slot is invoked as a plain closure.
            let typed = self
                .class_named_methods
                .get(&(obj.type_rid(), name.to_string()))
                .copied();
            let method = match typed {
                Some(method) => Some((method, vec![Value::Object(obj)])),
                None => zun_gc::with_current_heap(|heap| heap.dynamic_get_function(obj, name))
                    .map(|method| (method, vec![])),
            };
            // String hint: `Object.prototype.toString` always exists and
            // returns a primitive, so a missing own `toString` ends the
            // search (it never falls through to `valueOf`).
            if hint == "string" && name == "toString" && method.is_none() {
                return Ok(None);
            }
            if let Some((method, call_args)) = method {
                let result = self.invoke_function_value(method, call_args)?;
                let result = unbox_arg(&result)?;
                if !matches!(result, Value::Object(_) | Value::Function(_)) {
                    return Ok(Some(result));
                }
                rejected = true;
            }
        }
        if rejected {
            return Err(RunError::Native("TypeError: Cannot convert object to primitive value".into()));
        }
        Ok(None)
    }

    /// Settle `target` with a reaction callback's return value, adopting a
    /// returned Promise (chaining, not wrapping). A still-pending returned
    /// Promise is adopted by attaching a pass-through reaction, so `target`
    /// settles exactly when the returned Promise does — the proper async
    /// chaining the old eager path could not express.
    fn settle_target_with(&mut self, target: zun_gc::Obj, value: Value) {
        if let Some(obj) = as_object_ref(&value) {
            // A promise resolved with itself rejects with a TypeError.
            if obj == target {
                let message = "Chaining cycle detected for promise #<Promise>";
                if let Some(Value::Object(error)) = self.make_error_value("TypeError", message) {
                    let reason = zun_core::Any::object(error.addr() as usize);
                    self.settle_promise_reject_value(target, reason, format!("TypeError: {message}"));
                }
                return;
            }
            if let Some(state) = zun_gc::with_current_heap(|heap| heap.promise_state(obj)) {
                // ECMAScript NewPromiseResolveThenableJob: adopting a native
                // Promise costs a job (which attaches the pass-through reaction)
                // plus the reaction tick.
                let dummy = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                let roots = [target, obj, dummy];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                zun_gc::with_current_heap_mut(|heap| {
                    heap.promise_resolve(roots[2], zun_core::Any::null())
                });
                self.attach_reaction(
                    roots[2],
                    roots[0],
                    ReactionKind::ThenableJob {
                        thenable: Value::Object(roots[1]),
                        then: Value::Void,
                    },
                );
                return;
                #[allow(unreachable_code)]
                match state {
                    zun_core::ManagedPromiseState::Resolved => {
                        let inner = zun_gc::with_current_heap(|heap| heap.promise_result(obj))
                            .unwrap_or_else(zun_core::Any::null);
                        self.settle_promise_resolve(target, inner);
                    }
                    zun_core::ManagedPromiseState::Rejected => {
                        let error = zun_gc::with_current_heap(|heap| heap.promise_error(obj))
                            .unwrap_or_else(|| "Promise rejected".into());
                        let reason = zun_gc::with_current_heap(|heap| heap.promise_reason(obj));
                        self.notice_promise(obj);
                        match reason {
                            Some(reason) => {
                                self.settle_promise_reject_value(target, reason, error);
                            }
                            None => {
                                self.settle_promise_reject(target, error);
                            }
                        }
                    }
                    // Adopt a pending Promise: a pass-through reaction copies its
                    // settlement to `target` when it settles.
                    zun_core::ManagedPromiseState::Pending => {
                        self.attach_reaction(
                            obj,
                            target,
                            ReactionKind::Then {
                                on_fulfilled: None,
                                on_rejected: None,
                            },
                        );
                    }
                }
                return;
            }
            // Generic thenable: an object with a callable `then` is adopted by
            // calling `then(resolve, reject)` with fresh settlers for `target`
            // (the `new Promise` executor mechanism). Function values do not
            // box into `Any`, so a callable `then` lives in the function slot.
            // The call is scheduled as a microtask job: a pre-resolved dummy
            // promise carries a `ThenableJob` reaction, which the scheduler
            // enqueues (ReadyNow) and the turn-drain runs.
            // A typed class instance with a `then` method is a thenable too; its
            // method rid rides the job as an integer (called with the instance
            // as `this`).
            let typed_then = if zun_gc::with_current_heap(|heap| {
                heap.dynamic_get_function(obj, "then").is_none() && !heap.is_dynamic_object(obj)
            }) {
                self.instance_method_rid(obj, "then")
            } else {
                None
            };
            // A plain object's `then` is read once, through its accessor and
            // prototype chain, and adopted only when callable.
            let dynamic_then = if typed_then.is_none()
                && zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj) && heap.array_len(obj).is_none())
            {
                match self.dynamic_member_get(obj, "then") {
                    Ok(Value::Function(handle)) => Some(Value::Function(handle)),
                    Ok(Value::Any(any)) => zun_gc::with_current_heap(|heap| {
                        match crate::collections::unbox_function_box(Value::unbox_any(any, heap).unwrap_or(Value::Void), heap) {
                            Value::Function(handle) => Some(Value::Function(handle)),
                            _ => None,
                        }
                    }),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(then) = typed_then
                .map(|rid| Value::Int(rid as i64, zun_il::NumKind::U32))
                .or(dynamic_then)
            {
                let dummy = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                let roots = [target, obj, dummy];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                zun_gc::with_current_heap_mut(|heap| {
                    heap.promise_resolve(roots[2], zun_core::Any::null())
                });
                self.attach_reaction(
                    roots[2],
                    roots[0],
                    ReactionKind::ThenableJob {
                        thenable: Value::Object(roots[1]),
                        then,
                    },
                );
                return;
            }
        }
        let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        self.settle_promise_resolve(target, boxed);
    }

    /// `asyncGenResume(gen, mode, value)`: advance an async-generator coroutine
    /// (mode 0 = next, 1 = return, 2 = throw) and answer `[kind, value]` where
    /// kind 0 = the body awaits `value`, 1 = it yielded `value`, 2 = it finished
    /// with `value`. A body exception propagates as a thrown value.
    fn async_gen_resume(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let generator = as_object_ref(&unbox_arg(args.first().ok_or(RunError::BadToken)?)?).ok_or(RunError::BadToken)?;
        let mode = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
            Value::Int(n, _) => n,
            Value::Float(n, _) => n as i64,
            _ => 0,
        };
        let value = unbox_arg(args.get(2).unwrap_or(&Value::Void))?;
        let roots = [generator];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let (done, started) = zun_gc::with_current_heap(|heap| {
            heap.generator_state(roots[0])
                .map(|state| (state.done, state.frame.is_some()))
                .unwrap_or((true, false))
        });
        let finish = |kind: i64, value: Value| -> Result<Value, RunError> {
            zun_gc::with_current_heap_mut(|heap| {
                let array = heap.alloc_array(2);
                let kind_any = Value::Float(kind as f64, zun_il::NumKind::F64).box_any(heap)?;
                let value_any = value.box_any(heap)?;
                heap.array_set(array, 0, kind_any);
                heap.array_set(array, 1, value_any);
                Some(Value::Any(zun_core::Any::object(array.addr() as usize)))
            })
            .ok_or(RunError::BadToken)
        };
        match mode {
            1 => {
                if done || !started {
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(state) = heap.generator_state_mut(roots[0]) {
                            state.done = true;
                            state.frame = None;
                        }
                    });
                    return finish(2, value);
                }
                crate::unwind::ASYNC_AWAIT.with(|flag| flag.set(false));
                let result = self.generator_return(roots[0], value)?;
                let flagged = crate::unwind::ASYNC_AWAIT.with(|flag| flag.replace(false));
                let result_obj = as_object_ref(&unbox_arg(&result)?).ok_or(RunError::BadToken)?;
                let (inner, finished) = zun_gc::with_current_heap(|heap| {
                    let inner = heap
                        .dynamic_get(result_obj, "value")
                        .and_then(|any| Value::unbox_any(any, heap))
                        .unwrap_or(Value::Void);
                    let finished = heap
                        .dynamic_get(result_obj, "done")
                        .and_then(|any| any.as_bool())
                        .unwrap_or(true);
                    (inner, finished)
                });
                return finish(if finished { 2 } else if flagged { 0 } else { 1 }, inner);
            }
            2 => {
                if done || !started {
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(state) = heap.generator_state_mut(roots[0]) {
                            state.done = true;
                            state.frame = None;
                        }
                    });
                    return Err(RunError::Thrown(value));
                }
                crate::unwind::GENERATOR_THROW_PENDING.with(|slot| *slot.borrow_mut() = Some(value));
            }
            _ => {
                if done {
                    return finish(2, Value::Void);
                }
                crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = Some(value));
            }
        }
        crate::unwind::ASYNC_AWAIT.with(|flag| flag.set(false));
        let result = self.call_managed_collection("generatorIteratorNext", &[Value::Object(roots[0])]);
        crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = None);
        crate::unwind::GENERATOR_THROW_PENDING.with(|slot| *slot.borrow_mut() = None);
        let awaited = crate::unwind::ASYNC_AWAIT.with(|flag| flag.replace(false));
        let value = result?;
        let finished = zun_gc::with_current_heap(|heap| {
            heap.generator_state(roots[0]).map(|state| state.done).unwrap_or(true)
        });
        finish(if finished { 2 } else if awaited { 0 } else { 1 }, value)
    }

    /// Finish creating a generator object: an `async function*` hands the raw
    /// coroutine to the prelude's `__zunMakeAsyncGen`, which wraps it in an
    /// async-generator object (queueing, promise-returning `next`/`return`/`throw`).
    pub(crate) fn finish_generator_creation(
        &mut self,
        method_rid: u32,
        generator: zun_gc::Obj,
    ) -> Result<Value, RunError> {
        let is_async_gen = self
            .module
            .tables
            .method_defs
            .get((method_rid as usize).wrapping_sub(1))
            .and_then(|method| read_body(&self.module.code, method.body_offset).ok())
            .map(|body| {
                matches!(decode_one(&body.code), Ok((first, _)) if first.opcode == Opcode::AsyncGenStart)
            })
            .unwrap_or(false);
        if !is_async_gen {
            return Ok(Value::Object(generator));
        }
        let maker = self
            .module
            .tables
            .method_defs
            .iter()
            .position(|method| self.module.strings.get(method.name) == "__zunMakeAsyncGen")
            .map(|index| index as u32 + 1)
            .ok_or_else(|| RunError::Native("async generator support is unavailable".into()))?;
        let roots = [generator];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        self.call(maker, vec![Value::Object(roots[0])], None)
    }

    /// Entry of an `async` function: allocate its result Promise and a
    /// generator-style coroutine over the body, run the body to its first
    /// `await` (synchronously, as the spec requires) and hand the Promise back.
    pub(crate) fn async_start(&mut self, method_rid: u32, args: Vec<Value>) -> Result<Value, RunError> {
        let boxed = zun_gc::with_current_heap_mut(|heap| {
            args.iter().map(|value| gen_box(value, heap)).collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let (promise, generator) = zun_gc::with_current_heap_mut(|heap| {
            let promise = heap.alloc_promise();
            let generator = heap.alloc_generator(0, method_rid, boxed);
            (promise, generator)
        });
        let roots = [promise, generator];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        self.async_step(roots[1], roots[0], Ok(Value::Void))?;
        // Closures are called through function values, whose results are
        // dynamically typed (`any`); named functions and methods return the
        // Promise object itself.
        let dynamic_result = self
            .module
            .tables
            .method_defs
            .get(method_rid as usize - 1)
            .map(|method| self.module.strings.get(method.name).starts_with("<closure@"))
            .unwrap_or(false);
        if dynamic_result {
            Ok(Value::Any(zun_core::Any::object(roots[0].addr() as usize)))
        } else {
            Ok(Value::Object(roots[0]))
        }
    }

    /// Advance an async coroutine one step: resume it with `input` (a value, or
    /// an exception raised at the `await`), then either settle `promise` (body
    /// finished or threw) or park the coroutine on the awaited value.
    pub(super) fn async_step(
        &mut self,
        generator: zun_gc::Obj,
        promise: zun_gc::Obj,
        input: Result<Value, Value>,
    ) -> Result<(), RunError> {
        let roots = [generator, promise];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        match input {
            Ok(value) => crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = Some(value)),
            Err(thrown) => {
                crate::unwind::GENERATOR_THROW_PENDING.with(|slot| *slot.borrow_mut() = Some(thrown))
            }
        }
        crate::unwind::ASYNC_BYPASS.with(|flag| flag.set(true));
        let result =
            self.call_managed_collection("generatorIteratorNext", &[Value::Object(roots[0])]);
        crate::unwind::ASYNC_BYPASS.with(|flag| flag.set(false));
        crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = None);
        crate::unwind::GENERATOR_THROW_PENDING.with(|slot| *slot.borrow_mut() = None);
        let done = zun_gc::with_current_heap(|heap| {
            heap.generator_state(roots[0]).map(|state| state.done).unwrap_or(true)
        });
        match result {
            Err(RunError::Thrown(thrown)) => {
                let message = value_to_error_string(&thrown);
                let boxed = zun_gc::with_current_heap_mut(|heap| thrown.box_any(heap));
                match boxed {
                    Some(reason) => {
                        self.settle_promise_reject_value(roots[1], reason, message);
                    }
                    None => {
                        self.settle_promise_reject(roots[1], message);
                    }
                }
                Ok(())
            }
            Err(other) => Err(other),
            Ok(value) if done => {
                self.settle_target_with(roots[1], value);
                Ok(())
            }
            Ok(value) => {
                // `await value`: a Promise is awaited directly; anything else
                // (a plain value or a thenable) is first resolved into a fresh
                // Promise so it takes the same one-tick path.
                let awaited = match as_object_ref(&value) {
                    Some(obj)
                        if zun_gc::with_current_heap(|heap| heap.promise_state(obj)).is_some() =>
                    {
                        obj
                    }
                    _ => {
                        let fresh = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                        let inner = [fresh];
                        let _inner_scope = unsafe { zun_gc::NativeRootScope::enter(&inner) };
                        self.settle_target_with(inner[0], value);
                        inner[0]
                    }
                };
                let awaited_roots = [awaited];
                let _awaited_scope = unsafe { zun_gc::NativeRootScope::enter(&awaited_roots) };
                self.notice_promise(awaited_roots[0]);
                self.attach_reaction(
                    awaited_roots[0],
                    roots[1],
                    ReactionKind::AsyncStep { generator: Value::Object(roots[0]) },
                );
                Ok(())
            }
        }
    }

    /// Attach a reaction to `source` and bridge its control onto the scheduler.
    /// The reaction waits on `source`'s scheduler promise; when that settles
    /// (now or later), the scheduler enqueues this continuation and the
    /// turn-drain runs [`Self::run_reaction`]. An already-settled source yields
    /// `ReadyNow`, which *enqueues* the reaction rather than running it inline —
    /// the async-not-sync guarantee.
    fn attach_reaction(
        &mut self,
        source: zun_gc::Obj,
        target: zun_gc::Obj,
        kind: ReactionKind,
    ) -> ContinuationId {
        let source_pid = self.bridge_pid(source);
        // Bridge the target too (for its side effect: it may reconcile an
        // already-settled target and is the identity a later settle wakes).
        self.bridge_pid(target);
        let cid = ContinuationId(self.async_state.next_cont);
        self.async_state.next_cont += 1;
        self.async_state.reactions.insert(
            cid,
            Reaction {
                source: Value::Object(source),
                target: Value::Object(target),
                kind,
            },
        );
        let state = &mut self.async_state;
        state
            .scheduler
            .park(&mut state.promises, Continuation::new(cid, source_pid));
        cid
    }

    /// Run one scheduled reaction (a microtask): read the source's settlement
    /// from the heap, run the matching callback (or pass the settlement
    /// through), and settle the reaction's target through the scheduler so its
    /// own reactions/frames wake in turn.
    pub(super) fn run_reaction(&mut self, cid: ContinuationId) -> Result<(), RunError> {
        let Some(reaction) = self.async_state.reactions.remove(&cid) else {
            return Ok(());
        };
        let source = as_object_ref(&reaction.source).ok_or(RunError::BadToken)?;
        let target = as_object_ref(&reaction.target).ok_or(RunError::BadToken)?;
        // Root source and target across any callback re-entry; the scope keeps
        // the array current if a collection relocates them.
        let roots = [source, target];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        match reaction.kind {
            ReactionKind::Then {
                on_fulfilled,
                on_rejected,
            } => match self.promise_settlement_of(roots[0])? {
                PromiseSettlement::Resolved(value) => match as_function(&on_fulfilled) {
                    Some(callback) => self.run_then_callback(callback, Value::Any(value), &roots)?,
                    None => {
                        self.settle_promise_resolve(roots[1], value);
                    }
                },
                PromiseSettlement::Rejected(error, reason) => match as_function(&on_rejected) {
                    Some(callback) => {
                        // Deliver a first-class reason value (an object such as
                        // an `AggregateError`) as-is; otherwise hand the
                        // callback the interned string message.
                        let arg = match reason {
                            Some(reason) => reason,
                            None => zun_gc::with_current_heap_mut(|heap| {
                                zun_core::Any::string(
                                    heap.intern_string(std::rc::Rc::from(error.as_str())),
                                )
                            }),
                        };
                        self.run_then_callback(callback, Value::Any(arg), &roots)?;
                    }
                    None => match reason {
                        Some(reason) => {
                            self.settle_promise_reject_value(roots[1], reason, error);
                        }
                        None => {
                            self.settle_promise_reject(roots[1], error);
                        }
                    },
                },
                // A reaction only runs once its source has settled, so a pending
                // read here is a spurious wake; leave the target pending.
                PromiseSettlement::Pending => {}
            },
            ReactionKind::Finally { on_finally } => {
                let settlement = self.promise_settlement_of(roots[0])?;
                let cleanup_error = match as_function(&on_finally) {
                    Some(callback) => match self.invoke_callback(callback, vec![]) {
                        Ok(result) => {
                            // A cleanup that returns a Promise or thenable delays the
                            // pass-through until that value settles.
                            let waits = as_object_ref(&unbox_arg(&result).unwrap_or(Value::Void))
                                .filter(|obj| {
                                    zun_gc::with_current_heap(|heap| {
                                        heap.promise_state(*obj).is_some()
                                            || heap.dynamic_get_function(*obj, "then").is_some()
                                            || (!heap.is_dynamic_object(*obj)
                                                && self.instance_method_rid(*obj, "then").is_some())
                                    })
                                });
                            if let Some(obj) = waits {
                                let wait = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                                let wait_roots = [wait, obj, roots[0], roots[1]];
                                let _wait_scope = unsafe { zun_gc::NativeRootScope::enter(&wait_roots) };
                                self.settle_target_with(wait_roots[0], Value::Object(wait_roots[1]));
                                self.attach_reaction(
                                    wait_roots[0],
                                    wait_roots[3],
                                    ReactionKind::FinallyPass { original: Value::Object(wait_roots[2]) },
                                );
                                return Ok(());
                            }
                            None
                        }
                        Err(RunError::Thrown(value)) => Some(value),
                        Err(other) => return Err(other),
                    },
                    None => None,
                };
                if let Some(error) = cleanup_error {
                    self.reject_with_thrown(roots[1], &error);
                } else {
                    match settlement {
                        PromiseSettlement::Resolved(value) => {
                            self.settle_promise_resolve(roots[1], value);
                        }
                        PromiseSettlement::Rejected(error, Some(reason)) => {
                            self.settle_promise_reject_value(roots[1], reason, error);
                        }
                        PromiseSettlement::Rejected(error, None) => {
                            self.settle_promise_reject(roots[1], error);
                        }
                        PromiseSettlement::Pending => {}
                    }
                }
            }
            ReactionKind::FinallyPass { original } => {
                let waited = self.promise_settlement_of(roots[0])?;
                if let PromiseSettlement::Rejected(error, reason) = waited {
                    match reason {
                        Some(reason) => {
                            self.settle_promise_reject_value(roots[1], reason, error);
                        }
                        None => {
                            self.settle_promise_reject(roots[1], error);
                        }
                    }
                } else if let Some(original) = as_object_ref(&original) {
                    match self.promise_settlement_of(original)? {
                        PromiseSettlement::Resolved(value) => {
                            self.settle_promise_resolve(roots[1], value);
                        }
                        PromiseSettlement::Rejected(error, Some(reason)) => {
                            self.settle_promise_reject_value(roots[1], reason, error);
                        }
                        PromiseSettlement::Rejected(error, None) => {
                            self.settle_promise_reject(roots[1], error);
                        }
                        PromiseSettlement::Pending => {}
                    }
                }
            }
            ReactionKind::Combinator { aggregate, index } => {
                let settlement = self.promise_settlement_of(roots[0])?;
                self.combinator_input_settled(aggregate, index, settlement);
            }
            ReactionKind::Microtask { callback } => {
                if let Some(callback) = as_function(&Some(callback)) {
                    match self.invoke_callback(callback, Vec::new()) {
                        Ok(_) | Err(RunError::Thrown(_)) => {}
                        Err(other) => return Err(other),
                    }
                }
            }
            ReactionKind::AsyncStep { generator } => {
                let generator = as_object_ref(&generator).ok_or(RunError::BadToken)?;
                let input = match crate::settled_await_value(roots[0]) {
                    Ok(Some(value)) => Ok(value),
                    Ok(None) => return Ok(()),
                    Err(RunError::Thrown(thrown)) => Err(thrown),
                    Err(other) => return Err(other),
                };
                self.async_step(generator, roots[1], input)?;
            }
            ReactionKind::ThenableJob { thenable, then } => {
                if matches!(then, Value::Void) {
                    // Native Promise adoption: pass its settlement through.
                    if let Some(source) = as_object_ref(&thenable) {
                        self.attach_reaction(
                            source,
                            roots[1],
                            ReactionKind::Then { on_fulfilled: None, on_rejected: None },
                        );
                    }
                } else if let Value::Int(rid, _) = then {
                    let resolve = self.make_promise_settler(roots[1], SETTLE_RESOLVE);
                    let reject = self.make_promise_settler(roots[1], SETTLE_REJECT);
                    match self.call(
                        rid as u32,
                        vec![thenable, Value::Function(resolve), Value::Function(reject)],
                        None,
                    ) {
                        Ok(_) => {}
                        Err(RunError::Thrown(thrown)) => {
                            let _ = self.invoke_promise_settler(roots[1], SETTLE_REJECT, vec![thrown]);
                        }
                        Err(_) => {
                            self.settle_promise_reject(roots[1], "thenable.then failed".into());
                        }
                    }
                } else if let Some(then) = as_function(&Some(then)) {
                    let resolve = self.make_promise_settler(roots[1], SETTLE_RESOLVE);
                    let reject = self.make_promise_settler(roots[1], SETTLE_REJECT);
                    let then_args = vec![Value::Function(resolve), Value::Function(reject)];
                    let then_args = match as_object_ref(&thenable) {
                        Some(receiver) => self.with_this_arg(then, receiver, then_args),
                        None => then_args,
                    };
                    match self.invoke_function_value(then, then_args) {
                        Ok(_) => {}
                        Err(RunError::Thrown(thrown)) => {
                            let _ =
                                self.invoke_promise_settler(roots[1], SETTLE_REJECT, vec![thrown]);
                        }
                        Err(_) => {
                            self.settle_promise_reject(roots[1], "thenable.then failed".into());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Invoke a `then`/`catch` callback and settle the reaction's target from
    /// its result (adopting a returned Promise) or reject it on a throw.
    /// `roots[1]` is re-read after the re-entry, which may have relocated it.
    fn run_then_callback(
        &mut self,
        callback: zun_core::FunctionHandle,
        arg: Value,
        roots: &[zun_gc::Obj],
    ) -> Result<(), RunError> {
        match self.invoke_callback(callback, vec![arg]) {
            Ok(result) => {
                self.settle_target_with(roots[1], result);
                Ok(())
            }
            Err(RunError::Thrown(value)) => {
                self.reject_with_thrown(roots[1], &value);
                Ok(())
            }
            Err(other) => Err(other),
        }
    }

    /// Reject `target` with a thrown value, keeping an object reason
    /// first-class (a plain string keeps the string surface).
    fn reject_with_thrown(&mut self, target: zun_gc::Obj, value: &Value) {
        match value {
            Value::Str(message) => {
                self.settle_promise_reject(target, message.to_string());
            }
            other => {
                let reason = zun_gc::with_current_heap_mut(|heap| other.clone().box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                let message = value_to_error_string(other);
                self.settle_promise_reject_value(target, reason, message);
            }
        }
    }

    /// `promise.then(onFulfilled?, onRejected?)` / `catch`: create a NEW promise
    /// and attach a reaction to the source. When the source settles (now or
    /// later), the reaction runs the matching callback as a microtask and
    /// settles the new promise. A `then` on a pending source now works — the
    /// behaviour the old eager-at-settle path could not provide.
    fn promise_then(
        &mut self,
        source: zun_gc::Obj,
        on_fulfilled: Option<zun_core::FunctionHandle>,
        on_rejected: Option<zun_core::FunctionHandle>,
    ) -> Result<Value, RunError> {
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        let roots = [source, target];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        self.attach_reaction(
            roots[0],
            roots[1],
            ReactionKind::Then {
                on_fulfilled: on_fulfilled.map(Value::Function),
                on_rejected: on_rejected.map(Value::Function),
            },
        );
        Ok(Value::Object(roots[1]))
    }

    /// `promise.finally(onFinally)`: create a NEW promise and attach a reaction
    /// that runs the cleanup on settlement and passes the source settlement
    /// through, unless the cleanup itself throws.
    fn promise_finally(
        &mut self,
        source: zun_gc::Obj,
        on_finally: Option<zun_core::FunctionHandle>,
    ) -> Result<Value, RunError> {
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        let roots = [source, target];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        self.attach_reaction(
            roots[0],
            roots[1],
            ReactionKind::Finally {
                on_finally: on_finally.map(Value::Function),
            },
        );
        Ok(Value::Object(roots[1]))
    }

    /// Normalize a combinator input to a Promise object: an existing Promise is
    /// used as-is, any other value is wrapped in an already-resolved Promise
    /// (`Promise.resolve` semantics) so every input drives a reaction.
    fn as_input_promise(&mut self, value: &Value) -> zun_gc::Obj {
        if let Some(obj) = as_object_ref(value) {
            if zun_gc::with_current_heap(|heap| heap.promise_state(obj)).is_some() {
                return obj;
            }
        }
        // A thenable object is adopted (its `then` runs as a microtask job).
        if let Some(obj) = as_object_ref(value) {
            let thenable = zun_gc::with_current_heap(|heap| {
                heap.dynamic_get_function(obj, "then").is_some()
                    || (!heap.is_dynamic_object(obj) && self.instance_method_rid(obj, "then").is_some())
            });
            if thenable {
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                let roots = [promise];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                self.settle_target_with(roots[0], Value::Object(obj));
                return roots[0];
            }
        }
        let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        zun_gc::with_current_heap_mut(|heap| {
            let promise = heap.alloc_promise();
            heap.promise_resolve(promise, boxed);
            promise
        })
    }

    /// `Promise.all/allSettled/race/any`: create a NEW promise and an aggregate,
    /// then attach a reaction to each input promise. The aggregate is settled
    /// per spec as the inputs settle through the scheduler, so pending inputs
    /// are handled (the eager path could only read already-settled inputs).
    fn promise_combinator(&mut self, name: &str, array: zun_gc::Obj) -> Result<Value, RunError> {
        let elements = self.read_array_values(array)?;
        let kind = match name {
            "promiseAll" => CombinatorKind::All,
            "promiseAllSettled" => CombinatorKind::AllSettled,
            "promiseRace" => CombinatorKind::Race,
            "promiseAny" => CombinatorKind::Any,
            _ => return Err(RunError::BadToken),
        };
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        // `all`/`allSettled` collect an ordered result array; `any` collects the
        // per-input rejection reasons for its `AggregateError`; `race` keeps no
        // array (first settlement wins).
        let holder = match kind {
            CombinatorKind::All | CombinatorKind::AllSettled | CombinatorKind::Any => {
                Value::Object(zun_gc::with_current_heap_mut(|heap| heap.alloc_array(elements.len())))
            }
            CombinatorKind::Race => Value::Void,
        };
        let aggregate = self.async_state.aggregates.len();
        self.async_state.aggregates.push(Aggregate {
            kind,
            target: Value::Object(target),
            holder,
            remaining: elements.len(),
            settled: false,
        });
        // An empty input settles the aggregate immediately per spec.
        if elements.is_empty() {
            self.conclude_empty_combinator(aggregate);
            return Ok(Value::Object(target));
        }
        for (index, element) in elements.iter().enumerate() {
            let source = self.as_input_promise(element);
            self.attach_reaction(source, target, ReactionKind::Combinator { aggregate, index });
        }
        Ok(Value::Object(target))
    }

    /// Settle an empty-input combinator: `all`/`allSettled` resolve to an empty
    /// array, `race` stays pending forever (spec), `any` rejects (no fulfilment).
    fn conclude_empty_combinator(&mut self, aggregate: usize) {
        let (kind, target, holder) = {
            let agg = &self.async_state.aggregates[aggregate];
            (agg.kind, agg.target.clone(), agg.holder.clone())
        };
        let target = as_object_ref(&target).expect("aggregate target is an object");
        match kind {
            CombinatorKind::All | CombinatorKind::AllSettled => {
                let array = as_object_ref(&holder).expect("all/allSettled holder is an array");
                let boxed = zun_core::Any::object(array.addr() as usize);
                self.settle_promise_resolve(target, boxed);
            }
            CombinatorKind::Race => {}
            CombinatorKind::Any => {
                let errors = as_object_ref(&holder).expect("any holder is an array");
                let reason = self.build_aggregate_error(errors, "All promises were rejected");
                self.settle_promise_reject_value(
                    target,
                    reason,
                    "All promises were rejected".into(),
                );
            }
        }
        self.async_state.aggregates[aggregate].settled = true;
    }

    /// Build an `AggregateError`-shaped dynamic object carrying `errors` (the
    /// collected rejection reasons), `message`, and `name`. A dynamic object
    /// rather than the typed `AggregateError` class: the class is not
    /// guaranteed to be present in the running module (it ships in
    /// `builtin/error.zun`, compiled in only when a program imports it), so the
    /// combinator cannot depend on it. The shape matches the class surface, so
    /// `reason.errors` / `reason.message` / `reason.name` read the same.
    fn build_aggregate_error(&mut self, errors: zun_gc::Obj, message: &str) -> zun_core::Any {
        let roots = [errors];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        zun_gc::with_current_heap_mut(|heap| {
            let object = heap.alloc_dynamic();
            let name = Value::Str(std::rc::Rc::from("AggregateError"))
                .box_any(heap)
                .unwrap_or_else(zun_core::Any::null);
            heap.dynamic_set(object, "name", name);
            let message = Value::Str(std::rc::Rc::from(message))
                .box_any(heap)
                .unwrap_or_else(zun_core::Any::null);
            heap.dynamic_set(object, "message", message);
            heap.dynamic_set(object, "errors", zun_core::Any::object(roots[0].addr() as usize));
            zun_core::Any::object(object.addr() as usize)
        })
    }

    /// Fold one settled input into its combinator aggregate and settle the
    /// aggregate target when the per-spec condition is met.
    fn combinator_input_settled(
        &mut self,
        aggregate: usize,
        index: usize,
        settlement: PromiseSettlement,
    ) {
        if self.async_state.aggregates[aggregate].settled {
            return;
        }
        let kind = self.async_state.aggregates[aggregate].kind;
        match kind {
            CombinatorKind::All => match settlement {
                PromiseSettlement::Resolved(value) => {
                    let holder =
                        as_object_ref(&self.async_state.aggregates[aggregate].holder).unwrap();
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(holder, index, value));
                    let remaining = {
                        let agg = &mut self.async_state.aggregates[aggregate];
                        agg.remaining -= 1;
                        agg.remaining
                    };
                    if remaining == 0 {
                        let (target, holder) = {
                            let agg = &self.async_state.aggregates[aggregate];
                            (
                                as_object_ref(&agg.target).unwrap(),
                                as_object_ref(&agg.holder).unwrap(),
                            )
                        };
                        let boxed = zun_core::Any::object(holder.addr() as usize);
                        self.settle_promise_resolve(target, boxed);
                        self.async_state.aggregates[aggregate].settled = true;
                    }
                }
                PromiseSettlement::Rejected(error, reason) => {
                    let target =
                        as_object_ref(&self.async_state.aggregates[aggregate].target).unwrap();
                    match reason {
                        Some(reason) => {
                            self.settle_promise_reject_value(target, reason, error);
                        }
                        None => {
                            self.settle_promise_reject(target, error);
                        }
                    }
                    self.async_state.aggregates[aggregate].settled = true;
                }
                PromiseSettlement::Pending => {}
            },
            CombinatorKind::AllSettled => {
                let descriptor = match settlement {
                    PromiseSettlement::Resolved(value) => zun_gc::with_current_heap_mut(|heap| {
                        let object = heap.alloc_dynamic();
                        let status = Value::Str(std::rc::Rc::from("fulfilled"))
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        heap.dynamic_set(object, "status", status);
                        heap.dynamic_set(object, "value", value);
                        zun_core::Any::object(object.addr() as usize)
                    }),
                    PromiseSettlement::Rejected(error, reason) => {
                        zun_gc::with_current_heap_mut(|heap| {
                            let object = heap.alloc_dynamic();
                            let status = Value::Str(std::rc::Rc::from("rejected"))
                                .box_any(heap)
                                .unwrap_or_else(zun_core::Any::null);
                            heap.dynamic_set(object, "status", status);
                            // Carry the first-class reason when present, else the
                            // string message.
                            let reason = reason.unwrap_or_else(|| {
                                Value::Str(std::rc::Rc::from(error.as_str()))
                                    .box_any(heap)
                                    .unwrap_or_else(zun_core::Any::null)
                            });
                            heap.dynamic_set(object, "reason", reason);
                            zun_core::Any::object(object.addr() as usize)
                        })
                    }
                    PromiseSettlement::Pending => return,
                };
                let holder = as_object_ref(&self.async_state.aggregates[aggregate].holder).unwrap();
                zun_gc::with_current_heap_mut(|heap| heap.array_set(holder, index, descriptor));
                let remaining = {
                    let agg = &mut self.async_state.aggregates[aggregate];
                    agg.remaining -= 1;
                    agg.remaining
                };
                if remaining == 0 {
                    let (target, holder) = {
                        let agg = &self.async_state.aggregates[aggregate];
                        (
                            as_object_ref(&agg.target).unwrap(),
                            as_object_ref(&agg.holder).unwrap(),
                        )
                    };
                    let boxed = zun_core::Any::object(holder.addr() as usize);
                    self.settle_promise_resolve(target, boxed);
                    self.async_state.aggregates[aggregate].settled = true;
                }
            }
            CombinatorKind::Race => {
                let target = as_object_ref(&self.async_state.aggregates[aggregate].target).unwrap();
                match settlement {
                    PromiseSettlement::Resolved(value) => {
                        self.settle_promise_resolve(target, value);
                    }
                    PromiseSettlement::Rejected(error, Some(reason)) => {
                        self.settle_promise_reject_value(target, reason, error);
                    }
                    PromiseSettlement::Rejected(error, None) => {
                        self.settle_promise_reject(target, error);
                    }
                    PromiseSettlement::Pending => return,
                }
                self.async_state.aggregates[aggregate].settled = true;
            }
            CombinatorKind::Any => match settlement {
                PromiseSettlement::Resolved(value) => {
                    let target =
                        as_object_ref(&self.async_state.aggregates[aggregate].target).unwrap();
                    self.settle_promise_resolve(target, value);
                    self.async_state.aggregates[aggregate].settled = true;
                }
                PromiseSettlement::Rejected(error, reason) => {
                    // Collect this input's rejection reason into the ordered
                    // errors array (a first-class reason as-is, else the string
                    // message boxed).
                    let holder =
                        as_object_ref(&self.async_state.aggregates[aggregate].holder).unwrap();
                    let reason = reason.unwrap_or_else(|| {
                        zun_gc::with_current_heap_mut(|heap| {
                            Value::Str(std::rc::Rc::from(error.as_str()))
                                .box_any(heap)
                                .unwrap_or_else(zun_core::Any::null)
                        })
                    });
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(holder, index, reason));
                    let remaining = {
                        let agg = &mut self.async_state.aggregates[aggregate];
                        agg.remaining -= 1;
                        agg.remaining
                    };
                    if remaining == 0 {
                        let (target, holder) = {
                            let agg = &self.async_state.aggregates[aggregate];
                            (
                                as_object_ref(&agg.target).unwrap(),
                                as_object_ref(&agg.holder).unwrap(),
                            )
                        };
                        // All inputs rejected: reject with an `AggregateError`
                        // carrying the collected reasons (per spec).
                        let aggregate_error =
                            self.build_aggregate_error(holder, "All promises were rejected");
                        self.settle_promise_reject_value(
                            target,
                            aggregate_error,
                            "All promises were rejected".into(),
                        );
                        self.async_state.aggregates[aggregate].settled = true;
                    }
                }
                PromiseSettlement::Pending => {}
            },
        }
    }
}

/// Settlement of a managed Promise, read into owned data so it survives a
/// re-entry that may relocate the heap.
enum PromiseSettlement {
    Pending,
    Resolved(zun_core::Any),
    /// The display message plus an optional first-class reason value (an object
    /// such as an `AggregateError`, or any boxed value) when the rejection
    /// carried one.
    Rejected(String, Option<zun_core::Any>),
}

/// A stored reaction callback's function handle, if present.
fn as_function(value: &Option<Value>) -> Option<zun_core::FunctionHandle> {
    match value {
        Some(Value::Function(handle)) => Some(*handle),
        _ => None,
    }
}

/// The real wall-clock time in milliseconds, read through the `zun:core`
/// `time` group's `nowMillis` native. Used only as the *base* the virtual timer clock is
/// seeded from; the event loop advances from there by fast-forward.
fn real_now_millis(output: &mut dyn super::Output) -> i64 {
    match zun_natives::resolve("time", "nowMillis") {
        Some((id, _)) => match zun_natives::dispatch(id, &[], output) {
            Ok(zun_natives::NativeValue::Int(ms)) => ms,
            _ => 0,
        },
        None => 0,
    }
}

/// A value's object reference, unwrapping an object-tagged `Any`.
fn as_object_ref(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Object(object) => Some(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
        _ => None,
    }
}

/// Unwrap a `functionBox` wrapper back to its callable. Function values cannot
/// be boxed into `Any`, so a function carried through an `Any`-boxing argument
/// lane (the bound-function record's `__args` array) is wrapped in a dynamic
/// object holding the callable in a `__boxedFn` function slot. A plain object
/// without that slot, and every non-object value, passes through unchanged.
pub(crate) fn unbox_function_box(value: Value, heap: &zun_gc::Heap) -> Value {
    if let Value::Object(object) = value {
        if let Some(handle) = heap.dynamic_get_function(object, "__boxedFn") {
            return Value::Function(handle);
        }
    }
    value
}

/// Render a thrown value / rejection reason as a Promise rejection message
/// (Zun's managed Promise stores a string reason). A boxed `Any` is unwrapped
/// first, so a `string` reason survives being carried as `any`.
fn value_to_error_string(value: &Value) -> String {
    match value {
        Value::Str(message) => message.to_string(),
        Value::Any(any) => match zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)) {
            Some(Value::Str(message)) => message.to_string(),
            Some(inner) => inner.display(),
            None => value.display(),
        },
        other => other.display(),
    }
}

/// Interpret an argument as a `[[Prototype]]` link: `null`/`undefined` clears
/// it, an object reference becomes the prototype, anything else is ignored.
fn prototype_from_arg(value: &Value) -> Option<zun_core::Any> {
    match value {
        Value::Null | Value::Void => None,
        Value::Object(object) => Some(zun_core::Any::object(object.addr() as usize)),
        Value::Any(any) => match any.tag() {
            zun_core::AnyTag::Null | zun_core::AnyTag::Void => None,
            zun_core::AnyTag::Object => Some(*any),
            _ => None,
        },
        _ => None,
    }
}

/// An integer index argument, accepting either numeric representation and
/// truncating a float toward zero (the IL carries relative indices as `i64`).
fn int_arg(value: Option<&Value>) -> Result<i64, RunError> {
    match value.ok_or(RunError::BadToken)? {
        Value::Int(value, _) => Ok(*value),
        Value::Float(value, _) => Ok(*value as i64),
        _ => Err(RunError::BadToken),
    }
}

/// The target element kind a `from`/`of` static carries, decoded from its
/// table-order discriminant argument.
fn kind_arg(value: Option<&Value>) -> Result<zun_core::TypedElement, RunError> {
    match value.ok_or(RunError::BadToken)? {
        Value::Int(value, _) => {
            zun_core::TypedElement::from_index(*value as u32).ok_or(RunError::BadToken)
        }
        _ => Err(RunError::BadToken),
    }
}

/// Numeric equality between a typed-array element and a search value.
/// `indexOf` uses strict equality (NaN never matches); `includes` uses
/// SameValueZero (NaN matches NaN). Comparison is on `f64` for mixed kinds;
/// two integers compare exactly.
fn numeric_same(element: &Value, needle: &Value, same_value_zero: bool) -> bool {
    if let (Value::Int(a, _), Value::Int(b, _)) = (element, needle) {
        return a == b;
    }
    let as_f64 = |value: &Value| match value {
        Value::Int(value, _) => Some(*value as f64),
        Value::Float(value, _) => Some(*value),
        _ => None,
    };
    match (as_f64(element), as_f64(needle)) {
        (Some(a), Some(b)) => {
            if a.is_nan() && b.is_nan() {
                same_value_zero
            } else {
                a == b
            }
        }
        _ => false,
    }
}

/// Coerce a boxed value to a property-key string (ECMAScript ToPropertyKey for
/// the string cases this floor supports).
fn stringify_any(any: zun_core::Any, heap: &zun_gc::Heap) -> Option<String> {
    match Value::unbox_any(any, heap)? {
        Value::Str(text) => Some(text.to_string()),
        Value::Int(value, _) => Some(value.to_string()),
        Value::Float(value, _) => Some(zun_natives::format_float(value)),
        Value::Bool(value) => Some(value.to_string()),
        // A nested array joins recursively (`[[1, 2], 3].join()` is "1,2,3").
        Value::Object(obj) => {
            let length = heap.array_len(obj)?;
            let parts: Vec<String> = (0..length)
                .map(|index| {
                    let element = heap.array_get(obj, index).unwrap_or_else(zun_core::Any::void);
                    stringify_any(element, heap).unwrap_or_default()
                })
                .collect();
            Some(parts.join(","))
        }
        _ => None,
    }
}

/// Unbox an argument to its concrete value (an `Any` carrier is unwrapped).
pub(super) fn unbox_arg(value: &Value) -> Result<Value, RunError> {
    match value {
        Value::Any(any) => {
            zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)).ok_or(RunError::BadToken)
        }
        other => Ok(other.clone()),
    }
}

/// ECMAScript truthiness for a callback result (unboxing an `Any` carrier
/// first): `false`, `0`/`NaN`, `""`, `null` and `undefined` are falsy; every
/// object, function and non-empty string is truthy.
pub(crate) fn is_truthy(value: &Value, heap: &zun_gc::Heap) -> bool {
    let concrete = match value {
        Value::Any(any) => Value::unbox_any(*any, heap).unwrap_or(Value::Void),
        other => other.clone(),
    };
    match concrete {
        Value::Bool(flag) => flag,
        Value::Int(number, _) => number != 0,
        Value::Float(number, _) => number != 0.0 && !number.is_nan(),
        Value::Str(text) => !text.is_empty(),
        Value::Null | Value::Void => false,
        Value::Object(object) => !heap
            .bigint_value(object)
            .map(|big| big.is_zero())
            .unwrap_or(false),
        Value::Function(_) | Value::Any(_) => true,
    }
}

/// Coerce a comparator result (unboxing an `Any` carrier) to the number whose
/// sign decides the ordering. Non-numeric results yield `NaN`, treated by the
/// caller as "already ordered" (a stable no-swap).
/// ECMAScript ToNumber applied to a string: trim leading/trailing whitespace,
/// an empty result is `0`, `0x`/`0o`/`0b` prefixes parse in their radix, and
/// everything else (decimal, scientific, `Infinity`) goes through Rust's `f64`
/// parser; a failed parse is `NaN`.
pub(super) fn js_string_to_number(text: &str) -> f64 {
    // ECMAScript StrWhiteSpace: includes U+FEFF, excludes U+0085.
    let trimmed = text.trim_matches(|c: char| {
        matches!(c, '﻿') || (c.is_whitespace() && c != '')
    });
    if trimmed.is_empty() {
        return 0.0;
    }
    let radix = |prefix: &str, base: u32| -> Option<f64> {
        trimmed
            .strip_prefix(prefix)
            .filter(|rest| !rest.is_empty())
            .and_then(|rest| i64::from_str_radix(rest, base).ok())
            .map(|value| value as f64)
    };
    if let Some(value) = radix("0x", 16).or_else(|| radix("0X", 16)) {
        return value;
    }
    if let Some(value) = radix("0o", 8).or_else(|| radix("0O", 8)) {
        return value;
    }
    if let Some(value) = radix("0b", 2).or_else(|| radix("0B", 2)) {
        return value;
    }
    match trimmed {
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        // Rust accepts `inf`/`infinity`/`nan` in any case; JS does not.
        _ if trimmed.chars().any(|c| c.is_alphabetic() && c != 'e' && c != 'E') => f64::NAN,
        _ => trimmed.parse::<f64>().unwrap_or(f64::NAN),
    }
}

/// `Array.prototype.toString` (join with `,`) for an array object, or `None`
/// when `obj` is not an array.
pub(crate) fn array_join_text(obj: zun_gc::Obj) -> Option<String> {
    zun_gc::with_current_heap(|heap| {
        let length = heap.array_len(obj)?;
        let mut parts = Vec::with_capacity(length);
        for index in 0..length {
            let element = heap.array_get(obj, index).unwrap_or_else(zun_core::Any::void);
            parts.push(stringify_any(element, heap).unwrap_or_default());
        }
        Some(parts.join(","))
    })
}

fn to_number(value: &Value, heap: &zun_gc::Heap) -> f64 {
    let concrete = match value {
        Value::Any(any) => Value::unbox_any(*any, heap).unwrap_or(Value::Void),
        other => other.clone(),
    };
    match concrete {
        Value::Int(number, _) => number as f64,
        Value::Float(number, _) => number,
        Value::Bool(flag) => {
            if flag {
                1.0
            } else {
                0.0
            }
        }
        _ => f64::NAN,
    }
}

/// ECMAScript strict equality (`===`); with `zero_aware` it is SameValueZero
/// (`NaN` equals `NaN`, as `includes` requires).
fn strict_equals(left: &Value, right: &Value, zero_aware: bool) -> bool {
    let number = |value: &Value| match value {
        Value::Int(value, _) => Some(*value as f64),
        Value::Float(value, _) => Some(*value),
        _ => None,
    };
    if let (Some(left), Some(right)) = (number(left), number(right)) {
        if left.is_nan() && right.is_nan() {
            return zero_aware;
        }
        return left == right;
    }
    same_value(left, right)
}

/// ECMAScript `SameValue`: like `===` but `NaN` equals `NaN` and `+0` differs
/// from `-0`. Numbers compare by IEEE-754 bit pattern after the `NaN` check.
pub(super) fn same_value(left: &Value, right: &Value) -> bool {
    let number = |value: &Value| match value {
        Value::Int(value, _) => Some(*value as f64),
        Value::Float(value, _) => Some(*value),
        _ => None,
    };
    if let (Some(left), Some(right)) = (number(left), number(right)) {
        if left.is_nan() && right.is_nan() {
            return true;
        }
        return left.to_bits() == right.to_bits();
    }
    match (left, right) {
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Str(left), Value::Str(right)) => left == right,
        (Value::Object(left), Value::Object(right)) => left.addr() == right.addr(),
        (Value::Null, Value::Null) => true,
        (Value::Void, Value::Void) => true,
        (Value::Any(left), Value::Any(right)) => match (left.as_symbol(), right.as_symbol()) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        },
        _ => false,
    }
}

/// Marker slot naming a holder object that carries a suspended function value.
const GENERATOR_FUNCTION_SLOT: &str = "$genfn";

/// Box a value for a generator's saved frame. Function values have no `Any`
/// form, so they ride in a holder object's function slot.
pub(crate) fn gen_box(value: &Value, heap: &mut zun_gc::Heap) -> Option<zun_core::Any> {
    if let Value::Function(function) = value {
        let holder = heap.alloc_dynamic();
        heap.dynamic_set_function(holder, GENERATOR_FUNCTION_SLOT.to_string(), *function);
        return Some(zun_core::Any::object(holder.addr() as usize));
    }
    value.box_any(heap)
}

/// Inverse of [`gen_box`].
fn gen_unbox(any: zun_core::Any, heap: &zun_gc::Heap) -> Option<Value> {
    let value = Value::unbox_any(any, heap)?;
    if let Value::Object(object) = &value {
        if let Some(function) = heap.dynamic_get_function(*object, GENERATOR_FUNCTION_SLOT) {
            return Some(Value::Function(function));
        }
    }
    Some(value)
}

/// The built-in constructor name of a runtime-owned heap object (`Array`,
/// `Map`, `Set`, `Promise`, `RegExp`, plain `Object`), or `None` for a typed
/// class instance.
pub(crate) fn builtin_constructor_name(heap: &zun_gc::Heap, obj: zun_gc::Obj) -> Option<&'static str> {
    if heap.array_len(obj).is_some() {
        Some("Array")
    } else if heap.map_len(obj).is_some() {
        Some("Map")
    } else if heap.set_len(obj).is_some() {
        Some("Set")
    } else if heap.promise_state(obj).is_some() {
        Some("Promise")
    } else if heap.regex_source(obj).is_some() {
        Some("RegExp")
    } else if heap.date_millis(obj).is_some() {
        Some("Date")
    } else if heap.is_dynamic_object(obj)
        && !heap.is_instance_bag(obj)
        && heap.dynamic_get(obj, "constructor").is_none()
        && heap.dynamic_get_function(obj, "constructor").is_none()
    {
        Some("Object")
    } else {
        None
    }
}

thread_local! {
    /// Set while a `recv.m?.(..)` probe call runs: a missing member answers
    /// `BadToken` (-> `undefined`) instead of throwing `TypeError`.
    static OPTIONAL_PROBE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
