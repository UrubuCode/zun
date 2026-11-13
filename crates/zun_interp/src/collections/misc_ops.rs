//! Array hole/arguments, async/sync iterator open, for-in, and related native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_misc(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "arrayDeleteIndex" => {
                // `delete arr[i]`: leave a hole at an in-range index.
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                    Value::Int(value, _) if value >= 0 => value as usize,
                    Value::Float(value, _) if value >= 0.0 && value.fract() == 0.0 => {
                        value as usize
                    }
                    _ => return Ok(Value::Bool(true)),
                };
                if self
                    .guard_array(array, crate::integrity::ArrayOp::Remove)
                    .is_err()
                {
                    return Err(RunError::Native(
                        "TypeError: Cannot delete property of [object Array]".into(),
                    ));
                }
                zun_gc::with_current_heap_mut(|heap| {
                    if heap.array_len(array).is_some_and(|len| index < len) {
                        heap.array_set(array, index, zun_core::Any::hole());
                    }
                });
                Ok(Value::Bool(true))
            }
            "arrayMakeHole" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let index = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                    Value::Int(value, _) => value as usize,
                    Value::Float(value, _) => value as usize,
                    _ => return Err(RunError::BadToken),
                };
                zun_gc::with_current_heap_mut(|heap| {
                    heap.array_set(array, index, zun_core::Any::hole())
                });
                Ok(Value::Bool(true))
            }
            "arrayPushHole" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.array_push(array, zun_core::Any::hole()));
                Ok(Value::Bool(true))
            }
            "setCallArgc" => {
                let count = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                    Value::Float(value, _) => value as usize,
                    Value::Int(value, _) => value as usize,
                    _ => 0,
                };
                crate::unwind::CALL_ARGC_MARK.with(|mark| mark.set(Some(count)));
                Ok(args.first().ok_or(RunError::BadToken)?.clone())
            }
            "argumentsObject" => {
                // Trim the `[p1, ..., ...rest]` array to the number of arguments
                // actually passed (omitted defaults are not counted).
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let actual = crate::unwind::CALL_ARGC_CURRENT.with(|current| current.get());
                zun_gc::with_current_heap_mut(|heap| {
                    if let Some(len) = heap.array_len(array) {
                        if actual < len {
                            heap.array_resize(array, actual);
                        }
                    }
                    heap.array_mark_arguments(array);
                });
                Ok(Value::Object(array))
            }
            "generatorReturnValue" => {
                let value = crate::unwind::GENERATOR_RETURN_VALUE
                    .with(|slot| slot.borrow().clone())
                    .unwrap_or(Value::Void);
                let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                    .unwrap_or_else(zun_core::Any::void);
                Ok(Value::Any(boxed))
            }
            "asyncAwaitMark" => {
                crate::unwind::ASYNC_AWAIT.with(|flag| flag.set(true));
                Ok(args.first().cloned().unwrap_or(Value::Void))
            }
            "asyncGenResume" => self.async_gen_resume(args),
            "syncIterOpen" => {
                // GetIterator(src, sync): the iterator object behind `for-of` over
                // any iterable (built-in collections, strings, generators, user
                // iterables). Throws `TypeError` when `src` is not iterable.
                let value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let iterator = match value {
                    Value::Str(_) => self.call_managed_collection(
                        "stringIteratorNew",
                        std::slice::from_ref(&value),
                    )?,
                    other => match object(&other) {
                        Some(source) => Value::Object(self.resolve_iterator(source)?),
                        None => {
                            return Err(RunError::Native("TypeError: value is not iterable".into()))
                        }
                    },
                };
                let obj = object(&iterator).ok_or(RunError::BadToken)?;
                Ok(Value::Any(zun_core::Any::object(obj.addr() as usize)))
            }
            "asyncIterOpen" => {
                // `source[Symbol.asyncIterator]()` when the source carries that
                // method (own symbol slot or a class symbol method); `undefined`
                // otherwise so the prelude falls back to the sync iterator.
                let Some(source) =
                    object(&unbox_arg(args.first().ok_or(RunError::BadToken)?)?)
                else {
                    return Ok(Value::Void);
                };
                // Keep-alive only: `invoke_function_value` at the end can collect, but the receiver
                // is not read after it (it rides in `call_args`).
                zun_gc::root!(source = source);
                let async_id = zun_core::WellKnownSymbol::AsyncIterator.id();
                let handle = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_symbol_function(source.get(), async_id)
                })
                .or_else(|| {
                    self.class_symbol_methods
                        .get(&(source.get().type_rid(), async_id.get()))
                        .copied()
                });
                let Some(handle) = handle else {
                    return Ok(Value::Void);
                };
                let call_args = self.with_this_arg(handle, source.get(), Vec::new());
                let result = self.invoke_function_value(handle, call_args)?;
                match object(&unbox_arg(&result)?) {
                    Some(obj) => Ok(Value::Any(zun_core::Any::object(obj.addr() as usize))),
                    None => Err(RunError::Native(format!(
                        "TypeError: [Symbol.asyncIterator]() did not return an object ({result:?})"
                    ))),
                }
            }
            "generatorSent" => Ok(crate::unwind::GENERATOR_SENT
                .with(|sent| sent.borrow_mut().take())
                .unwrap_or(Value::Void)),
            "forInHas" => {
                // for-in liveness: is `key` still a property of `object` (own or
                // inherited)? Unknown receivers answer `true` (never skip).
                let Value::Object(mut current) =
                    unbox_arg(args.first().ok_or(RunError::BadToken)?)?
                else {
                    return Ok(Value::Bool(true));
                };
                let Value::Str(key) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? else {
                    return Ok(Value::Bool(true));
                };
                let key = key.to_string();
                let mut present = true;
                for _ in 0..256 {
                    let (is_array, is_dynamic, found) = zun_gc::with_current_heap(|heap| {
                        let array_len = heap.array_len(current);
                        let dynamic = heap.is_dynamic_object(current);
                        let mut found = false;
                        if let Some(len) = array_len {
                            if key == "length" {
                                found = true;
                            } else if let Ok(index) = key.parse::<usize>() {
                                if index.to_string() == key {
                                    found = index < len
                                        && heap
                                            .array_get(current, index)
                                            .is_some_and(|any| !any.is_hole());
                                }
                            }
                        }
                        if !found && dynamic {
                            found = heap.dynamic_has(current, &key)
                                || heap.dynamic_get_function(current, &key).is_some()
                                || heap.dynamic_accessor_getter(current, &key).is_some()
                                || heap
                                    .dynamic_keys(current)
                                    .is_some_and(|keys| keys.contains(&key));
                        }
                        (array_len.is_some(), dynamic, found)
                    });
                    let is_bag = zun_gc::with_current_heap(|heap| heap.is_instance_bag(current));
                    if (!is_array && !is_dynamic) || (is_bag && !found) {
                        present = true;
                        break;
                    }
                    if found {
                        present = true;
                        break;
                    }
                    present = false;
                    let next = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_prototype(current)
                            .and_then(|any| any.as_object())
                            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                    });
                    match next {
                        Some(next) => current = next,
                        None => break,
                    }
                }
                Ok(Value::Bool(present))
            }
            "arrayHas" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| {
                    let len = heap.array_len(array).unwrap_or(0);
                    if key == "length" {
                        return true;
                    }
                    match key.parse::<u64>() {
                        Ok(index) if index.to_string() == key => {
                            (index as usize) < len
                                && !heap
                                    .array_get(array, index as usize)
                                    .is_some_and(|element| element.is_hole())
                        }
                        _ => false,
                    }
                })))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
