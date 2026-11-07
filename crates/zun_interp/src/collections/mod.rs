//! Managed collection dispatch extracted from the interpreter core.

pub(super) use super::state::{
    Aggregate, CombinatorKind, ExecutionFrame, ExecutionOutcome, Reaction, ReactionKind, TimerEntry,
};
pub(super) use super::value::Value;
pub(super) use super::{Interp, RunError};
pub(super) use zun_il::{decode_one, Opcode};
pub(super) use zun_module::read_body;
pub(super) use zun_sched::{Continuation, ContinuationId, Settlement, TaskId};

mod array_construct;
mod array_iterate;
mod array_mutate;
mod dynamic_call;
mod iterators;
mod mapset;
mod misc_ops;

mod object_core;
mod object_descriptor;

mod function;
mod promise;
mod proxy;
mod reflect;
mod regexp;
mod string_num;

mod bigint;
mod buffer;
mod class_decorator;
mod generator;
mod runtime_async;
mod runtime_class_proxy;
mod runtime_coerce;
mod runtime_descriptor;
mod runtime_iter;
mod runtime_reaction;
mod runtime_typed_array;
mod weak;

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
        Value::Function(handle) => Some(zun_gc::with_current_heap_mut(|heap| {
            heap.function_cell(*handle)
        })),
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
    pub(super) fn collection_for_each(
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
            let Some(slot) =
                zun_gc::with_current_heap(|heap| heap.collection_cursor(roots[0], index))
            else {
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

    pub(crate) fn call_managed_collection(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RunError> {
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
                if let Some(object) =
                    self.new_error_object("TypeError", &format!("{member} is not a function"))
                {
                    return Err(RunError::Thrown(Value::Object(object)));
                }
            }
        }
        result
    }

    pub(super) fn call_managed_collection_inner(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RunError> {
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
            "reflectSet"
                | "reflectHas"
                | "reflectDelete"
                | "reflectOwnKeys"
                | "reflectDefineProperty"
                | "objectDefineProperty"
                | "objectDefineProperties"
                | "objectGetOwnPropertyDescriptor"
                | "objectGetOwnPropertyDescriptors"
                | "objectGetOwnPropertyNames"
                | "objectKeys"
                | "objectValues"
                | "objectEntries"
                | "objectFreeze"
                | "objectIsFrozen"
                | "objectSeal"
                | "objectIsSealed"
                | "objectPreventExtensions"
                | "objectIsExtensible"
                | "objectHasOwn"
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
            "bigIntPow" | "bigIntAnd" | "bigIntOr" | "bigIntXor" | "bigIntShl" | "bigIntShr" => {
                self.col_bigint(name, args)
            }
            "bigIntNot" => self.col_bigint(name, args),
            "bigIntAsIntN" | "bigIntAsUintN" => self.col_bigint(name, args),
            "bigIntAdd" | "bigIntSub" | "bigIntMul" | "bigIntDiv" | "bigIntRem"
            | "bigIntCompare" | "bigIntEq" | "bigIntNe" | "bigIntLt" | "bigIntLe" | "bigIntGt"
            | "bigIntGe" => self.col_bigint(name, args),
            "bigIntNeg" => self.col_bigint(name, args),
            "bigIntToString" => self.col_bigint(name, args),
            "regexpIsMatch" => self.col_regexp(name, args),
            "regexpSearch" | "regexpSplit" | "regexpSplitLimit" | "regexpReplace" => {
                self.col_regexp(name, args)
            }
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
            "dynamicCall0" | "dynamicCall1" | "dynamicCall2" | "dynamicCall3" | "dynamicCall4"
            | "dynamicCall5" | "dynamicCall6" => self.col_dynamic_call(name, args),
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
            "numberToFixedJs"
            | "numberToExponentialJs"
            | "numberToPrecisionJs"
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
                            Some(Value::Object(obj)) => {
                                zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some())
                            }
                            _ => false,
                        }
                    };
                    let (a_big, b_big) = (big(args.first()), big(args.get(1)));
                    if a_big && b_big {
                        return self.call_managed_collection(
                            "bigIntAdd",
                            &[unbox_arg(&args[0])?, unbox_arg(&args[1])?],
                        );
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
            "arrayIndexOf" | "arrayLastIndexOf" | "arrayIncludes" => {
                self.col_array_mutate(name, args)
            }
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
            "promiseAll" | "promiseAllSettled" | "promiseRace" | "promiseAny" => {
                self.col_promise(name, args)
            }
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
            "objectDefineProperty" | "reflectDefineProperty" => {
                self.col_object_descriptor(name, args)
            }
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
}

/// Settlement of a managed Promise, read into owned data so it survives a
/// re-entry that may relocate the heap.
pub(super) enum PromiseSettlement {
    Pending,
    Resolved(zun_core::Any),
    /// The display message plus an optional first-class reason value (an object
    /// such as an `AggregateError`, or any boxed value) when the rejection
    /// carried one.
    Rejected(String, Option<zun_core::Any>),
}

/// A stored reaction callback's function handle, if present.
pub(super) fn as_function(value: &Option<Value>) -> Option<zun_core::FunctionHandle> {
    match value {
        Some(Value::Function(handle)) => Some(*handle),
        _ => None,
    }
}

/// The real wall-clock time in milliseconds, read through the `zun:core`
/// `time` group's `nowMillis` native. Used only as the *base* the virtual timer clock is
/// seeded from; the event loop advances from there by fast-forward.
pub(super) fn real_now_millis(output: &mut dyn super::Output) -> i64 {
    match zun_natives::resolve("time", "nowMillis") {
        Some((id, _)) => match zun_natives::dispatch(id, &[], output) {
            Ok(zun_natives::NativeValue::Int(ms)) => ms,
            _ => 0,
        },
        None => 0,
    }
}

/// A value's object reference, unwrapping an object-tagged `Any`.
pub(super) fn as_object_ref(value: &Value) -> Option<zun_gc::Obj> {
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
pub(super) fn value_to_error_string(value: &Value) -> String {
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
pub(super) fn prototype_from_arg(value: &Value) -> Option<zun_core::Any> {
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
pub(super) fn int_arg(value: Option<&Value>) -> Result<i64, RunError> {
    match value.ok_or(RunError::BadToken)? {
        Value::Int(value, _) => Ok(*value),
        Value::Float(value, _) => Ok(*value as i64),
        _ => Err(RunError::BadToken),
    }
}

/// The target element kind a `from`/`of` static carries, decoded from its
/// table-order discriminant argument.
pub(super) fn kind_arg(value: Option<&Value>) -> Result<zun_core::TypedElement, RunError> {
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
pub(super) fn numeric_same(element: &Value, needle: &Value, same_value_zero: bool) -> bool {
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
pub(super) fn stringify_any(any: zun_core::Any, heap: &zun_gc::Heap) -> Option<String> {
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
                    let element = heap
                        .array_get(obj, index)
                        .unwrap_or_else(zun_core::Any::void);
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
    let trimmed = text.trim_matches(|c: char| matches!(c, '﻿') || (c.is_whitespace() && c != ''));
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
        _ if trimmed
            .chars()
            .any(|c| c.is_alphabetic() && c != 'e' && c != 'E') =>
        {
            f64::NAN
        }
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
            let element = heap
                .array_get(obj, index)
                .unwrap_or_else(zun_core::Any::void);
            parts.push(stringify_any(element, heap).unwrap_or_default());
        }
        Some(parts.join(","))
    })
}

pub(super) fn to_number(value: &Value, heap: &zun_gc::Heap) -> f64 {
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
pub(super) fn strict_equals(left: &Value, right: &Value, zero_aware: bool) -> bool {
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
pub(super) const GENERATOR_FUNCTION_SLOT: &str = "$genfn";

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
pub(super) fn gen_unbox(any: zun_core::Any, heap: &zun_gc::Heap) -> Option<Value> {
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
pub(crate) fn builtin_constructor_name(
    heap: &zun_gc::Heap,
    obj: zun_gc::Obj,
) -> Option<&'static str> {
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
