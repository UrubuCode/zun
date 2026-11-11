//! Map and Set core native methods (hot).
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    #[inline]
    pub(super) fn col_mapset(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "mapNew" => {
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_map());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "mapFrom" => {
                // `new Map(iterable)` where the iterable is an ARRAY of
                // `[key, value]` pairs. Arbitrary iterator-protocol objects are
                // deferred; only the array form is accepted here.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_map());
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                // Root both the source and the target: `map_insert` may grow the
                // table (allocating), relocating either handle.
                let roots = [source, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let entry = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let pair = entry
                        .as_object()
                        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                        .ok_or_else(|| {
                            RunError::Native(
                                "TypeError: Iterator value is not an entry object".into(),
                            )
                        })?;
                    let key = zun_gc::with_current_heap(|heap| heap.array_get(pair, 0))
                        .ok_or(RunError::BadToken)?;
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(pair, 1))
                        .unwrap_or_else(zun_core::Any::void);
                    zun_gc::with_current_heap_mut(|heap| heap.map_insert(roots[1], key, value));
                }
                Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
            }
            "mapSet" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.map_insert(object, key, value));
                Ok(Value::Void)
            }
            "mapSetChain" => {
                let recv = args.first().ok_or(RunError::BadToken)?.clone();
                let object = object(&recv).ok_or(RunError::BadToken)?;
                let key = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.map_insert(object, key, value));
                Ok(recv)
            }
            "setAddChain" => {
                let recv = args.first().ok_or(RunError::BadToken)?.clone();
                let object = object(&recv).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.set_add(object, value))
                    .ok_or(RunError::BadToken)?;
                Ok(recv)
            }
            "mapSetI32" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.map_insert(object, zun_core::Any::int(key), zun_core::Any::int(value))
                    })
                    .is_none(),
                ))
            }
            "mapSetI64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.map_insert(object, zun_core::Any::int(key), zun_core::Any::int(value))
                    })
                    .is_none(),
                ))
            }
            "mapSetF64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let value = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.map_insert(
                            object,
                            zun_core::Any::float(key),
                            zun_core::Any::float(value),
                        )
                    })
                    .is_none(),
                ))
            }
            "mapHasI32" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.map_has(object, zun_core::Any::int(key)))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "mapHasI64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.map_has(object, zun_core::Any::int(key)))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "mapHasF64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| {
                        heap.map_has(object, zun_core::Any::float(key))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "mapGetI32Or" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let fallback = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let value = zun_gc::with_current_heap(|heap| {
                    heap.map_get(object, zun_core::Any::int(key))
                        .and_then(|value| value.as_int())
                })
                .unwrap_or(fallback);
                Ok(Value::Int(value, zun_il::NumKind::I32))
            }
            "mapGetI64Or" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let fallback = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let value = zun_gc::with_current_heap(|heap| {
                    heap.map_get(object, zun_core::Any::int(key))
                        .and_then(|value| value.as_int())
                })
                .unwrap_or(fallback);
                Ok(Value::Int(value, zun_il::NumKind::I64))
            }
            "mapGetF64Or" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let fallback = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                let value = zun_gc::with_current_heap(|heap| {
                    heap.map_get(object, zun_core::Any::float(key))
                        .and_then(|value| value.as_float())
                })
                .unwrap_or(fallback);
                Ok(Value::Float(value, zun_il::NumKind::F64))
            }
            "mapDeleteI32" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.map_remove(object, zun_core::Any::int(key))
                    })
                    .is_some(),
                ))
            }
            "mapDeleteI64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.map_remove(object, zun_core::Any::int(key))
                    })
                    .is_some(),
                ))
            }
            "mapDeleteF64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.map_remove(object, zun_core::Any::float(key))
                    })
                    .is_some(),
                ))
            }
            "mapGet" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(zun_gc::with_current_heap(|heap| heap.map_get(object, key))
                    .map(Value::Any)
                    .unwrap_or(Value::Void))
            }
            "optionalProbeOk" => {
                // True for a managed (typed, non-proxy) object whose methods are not
                // plain properties: the `recv.m?.()` probe may resolve them.
                let ok = match unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    Ok(Value::Object(obj)) => zun_gc::with_current_heap(|heap| {
                        !heap.is_dynamic_object(obj) && heap.proxy_target(obj).is_none()
                    }),
                    _ => false,
                };
                Ok(Value::Bool(ok))
            }
            "mapHas" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.map_has(object, key))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "mapSize" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.map_len(object))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "mapDelete" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.map_remove(object, key)).is_some(),
                ))
            }
            "mapClear" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.map_clear(object))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Void)
            }
            "mapValues" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = zun_gc::with_current_heap(|heap| heap.map_entries(object))
                    .ok_or(RunError::BadToken)?;
                let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(entries.len()));
                for (index, (_, value)) in entries.into_iter().enumerate() {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.array_set(array, index, value);
                    });
                }
                let iterator =
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(array))
                        .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "mapKeys" | "mapEntries" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = zun_gc::with_current_heap(|heap| heap.map_entries(object))
                    .ok_or(RunError::BadToken)?;
                let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(entries.len()));
                for (index, (key, value)) in entries.into_iter().enumerate() {
                    let item = if name == "mapKeys" {
                        key
                    } else {
                        zun_gc::with_current_heap_mut(|heap| {
                            let item = heap.alloc_array(2);
                            let _ = heap.array_set(item, 0, key);
                            let _ = heap.array_set(item, 1, value);
                            zun_core::Any::object(item.addr() as usize)
                        })
                    };
                    zun_gc::with_current_heap_mut(|heap| {
                        let _ = heap.array_set(array, index, item);
                    });
                }
                let iterator =
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(array))
                        .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "mapForEach" => {
                // `callback(value, key, map)` over a live cursor: entries added
                // during iteration are visited, deleted ones are skipped.
                let map_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                self.collection_for_each(map_object, callback, false)
            }
            "setNew" => {
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_set());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "setFrom" => {
                // `new Set(iterable)` where the iterable is an ARRAY of
                // elements. Arbitrary iterator-protocol objects are deferred;
                // only the array form is accepted here.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_set());
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let roots = [source, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.set_add(roots[1], value));
                }
                Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
            }
            "setFromIterable" => {
                // `new Set(iterable)` for a non-array built-in iterable: a
                // string (iterated by Unicode code point), another Set (its
                // elements), or another Map (its `[key, value]` entry pairs).
                // An array argument takes the `setFrom` fast path in sema and
                // does not reach here, but is handled too for `any` receivers.
                // Arbitrary user `[Symbol.iterator]` objects are deferred.
                let arg = args.first().ok_or(RunError::BadToken)?;
                if matches!(unbox_arg(arg), Ok(Value::Void) | Ok(Value::Null)) {
                    let empty = zun_gc::with_current_heap_mut(|heap| heap.alloc_set());
                    return Ok(Value::Any(zun_core::Any::object(empty.addr() as usize)));
                }
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_set());
                // String: intern each code point and add it. Interned strings
                // are referenced by a stable id, so no rooting dance is needed.
                if let Value::Str(value) = arg {
                    let points: Vec<String> =
                        value.chars().map(|point| point.to_string()).collect();
                    let roots = [target];
                    let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                    for point in points {
                        zun_gc::with_current_heap_mut(|heap| {
                            let id = heap.intern_string(point.as_str());
                            heap.set_add(roots[0], zun_core::Any::string(id));
                        });
                    }
                    return Ok(Value::Any(zun_core::Any::object(roots[0].addr() as usize)));
                }
                let source = object(arg).ok_or_else(|| {
                    RunError::Native("TypeError: value is not iterable".to_string())
                })?;
                // Snapshot the source elements into a rooted array, then drain
                // it into the target set (the mapForEach materialization idiom:
                // a GC during `set_add` relocates the rooted array transparently).
                let values: Vec<zun_core::Any> = if let Some(values) =
                    zun_gc::with_current_heap(|heap| heap.set_values(source))
                {
                    values
                } else if let Some(entries) =
                    zun_gc::with_current_heap(|heap| heap.map_entries(source))
                {
                    // A Set built from a Map iterates `[key, value]` pairs.
                    zun_gc::with_current_heap_mut(|heap| {
                        entries
                            .into_iter()
                            .map(|(key, value)| {
                                let pair = heap.alloc_array(2);
                                let _ = heap.array_set(pair, 0, key);
                                let _ = heap.array_set(pair, 1, value);
                                zun_core::Any::object(pair.addr() as usize)
                            })
                            .collect()
                    })
                } else if let Some(length) =
                    zun_gc::with_current_heap(|heap| heap.array_len(source))
                {
                    (0..length)
                        .map(|index| {
                            zun_gc::with_current_heap(|heap| heap.array_get(source, index))
                                .ok_or(RunError::BadToken)
                        })
                        .collect::<Result<_, _>>()?
                } else {
                    // An arbitrary user `[Symbol.iterator]` iterable: drive
                    // the iteration protocol, adding each produced value.
                    let target = self.drive_iterable(source, target, |heap, set, value| {
                        heap.set_add(set, value);
                    })?;
                    return Ok(Value::Any(zun_core::Any::object(target.addr() as usize)));
                };
                let length = values.len();
                let staged = zun_gc::with_current_heap_mut(|heap| {
                    let staged = heap.alloc_array(length);
                    for (index, value) in values.into_iter().enumerate() {
                        heap.array_set(staged, index, value);
                    }
                    staged
                });
                let roots = [staged, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.set_add(roots[1], value));
                }
                Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
            }
            "mapFromIterable" => {
                // `new Map(iterable)` for a non-array built-in iterable: another
                // Map (its `[key, value]` entries) or another Set whose elements
                // are `[key, value]` pair arrays. An array argument takes the
                // `mapFrom` fast path in sema; it is handled here too for `any`
                // receivers. A string is not a valid map iterable (its elements
                // are not pairs) and is rejected, matching JS's TypeError.
                let arg = args.first().ok_or(RunError::BadToken)?;
                // `new Map(null)` / `new Map(undefined)` is an empty map.
                if matches!(unbox_arg(arg), Ok(Value::Void) | Ok(Value::Null)) {
                    let empty = zun_gc::with_current_heap_mut(|heap| heap.alloc_map());
                    return Ok(Value::Any(zun_core::Any::object(empty.addr() as usize)));
                }
                if matches!(arg, Value::Str(_)) {
                    return Err(RunError::Native(
                        "Map constructor requires an iterable of [key, value] pairs".into(),
                    ));
                }
                let source = object(arg).ok_or_else(|| {
                    RunError::Native("TypeError: value is not iterable".to_string())
                })?;
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_map());
                // A Map source contributes its entries directly; snapshot them
                // into rooted key/value arrays (the mapForEach idiom).
                if let Some(entries) = zun_gc::with_current_heap(|heap| heap.map_entries(source)) {
                    let length = entries.len();
                    let (keys, values) = zun_gc::with_current_heap_mut(|heap| {
                        let keys = heap.alloc_array(length);
                        let values = heap.alloc_array(length);
                        for (index, (key, value)) in entries.into_iter().enumerate() {
                            heap.array_set(keys, index, key);
                            heap.array_set(values, index, value);
                        }
                        (keys, values)
                    });
                    let roots = [keys, values, target];
                    let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                    for index in 0..length {
                        let key = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                            .ok_or(RunError::BadToken)?;
                        let value =
                            zun_gc::with_current_heap(|heap| heap.array_get(roots[1], index))
                                .ok_or(RunError::BadToken)?;
                        zun_gc::with_current_heap_mut(|heap| heap.map_insert(roots[2], key, value));
                    }
                    return Ok(Value::Any(zun_core::Any::object(roots[2].addr() as usize)));
                }
                // A Set source (or an array): each element must be a `[key,
                // value]` pair array. Snapshot the elements into a rooted array
                // and read each pair out of it.
                let elements: Vec<zun_core::Any> = if let Some(values) =
                    zun_gc::with_current_heap(|heap| heap.set_values(source))
                {
                    values
                } else if let Some(length) =
                    zun_gc::with_current_heap(|heap| heap.array_len(source))
                {
                    (0..length)
                        .map(|index| {
                            zun_gc::with_current_heap(|heap| heap.array_get(source, index))
                                .ok_or(RunError::BadToken)
                        })
                        .collect::<Result<_, _>>()?
                } else {
                    // An arbitrary user `[Symbol.iterator]` iterable yielding
                    // `[key, value]` pair arrays: drive the protocol.
                    let target = self.drive_iterable(source, target, |heap, map, value| {
                        if let Some(pair) = value
                            .as_object()
                            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                        {
                            let key = heap.array_get(pair, 0).unwrap_or_else(zun_core::Any::void);
                            let val = heap.array_get(pair, 1).unwrap_or_else(zun_core::Any::void);
                            heap.map_insert(map, key, val);
                        }
                    })?;
                    return Ok(Value::Any(zun_core::Any::object(target.addr() as usize)));
                };
                let length = elements.len();
                let staged = zun_gc::with_current_heap_mut(|heap| {
                    let staged = heap.alloc_array(length);
                    for (index, value) in elements.into_iter().enumerate() {
                        heap.array_set(staged, index, value);
                    }
                    staged
                });
                let roots = [staged, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let entry = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let pair = entry
                        .as_object()
                        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                        .ok_or_else(|| {
                            RunError::Native(
                                "TypeError: Iterator value is not an entry object".into(),
                            )
                        })?;
                    let key = zun_gc::with_current_heap(|heap| heap.array_get(pair, 0))
                        .ok_or(RunError::BadToken)?;
                    let value = zun_gc::with_current_heap(|heap| heap.array_get(pair, 1))
                        .unwrap_or_else(zun_core::Any::void);
                    zun_gc::with_current_heap_mut(|heap| heap.map_insert(roots[1], key, value));
                }
                Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
            }
            "setAdd" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.set_add(object, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "setAddI32" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.set_add(object, zun_core::Any::int(value))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "setAddI64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.set_add(object, zun_core::Any::int(value))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "setAddF64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.set_add(object, zun_core::Any::float(value))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "setHasI32" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| {
                        heap.set_has(object, zun_core::Any::int(value))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "setHasI64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| {
                        heap.set_has(object, zun_core::Any::int(value))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "setHasF64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| {
                        heap.set_has(object, zun_core::Any::float(value))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "setDeleteI32" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.set_remove(object, zun_core::Any::int(value))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "setDeleteI64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.set_remove(object, zun_core::Any::int(value))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "setDeleteF64" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.set_remove(object, zun_core::Any::float(value))
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "setHas" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.set_has(object, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "setSize" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Int(
                    zun_gc::with_current_heap(|heap| heap.set_len(object))
                        .ok_or(RunError::BadToken)? as i64,
                    zun_il::NumKind::U64,
                ))
            }
            "setDelete" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap_mut(|heap| heap.set_remove(object, value))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "setClear" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| heap.set_clear(object))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Void)
            }
            "setValues" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // A live cursor: deletions and additions during iteration are seen.
                let iterator =
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_set_iterator(object))
                        .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "setEntries" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let values = zun_gc::with_current_heap(|heap| heap.set_values(object))
                    .ok_or(RunError::BadToken)?;
                let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(values.len()));
                for (index, value) in values.into_iter().enumerate() {
                    let pair = zun_gc::with_current_heap_mut(|heap| {
                        let pair = heap.alloc_array(2);
                        let _ = heap.array_set(pair, 0, value);
                        let _ = heap.array_set(pair, 1, value);
                        pair
                    });
                    zun_gc::with_current_heap_mut(|heap| {
                        let _ = heap.array_set(
                            array,
                            index,
                            zun_core::Any::object(pair.addr() as usize),
                        );
                    });
                }
                let iterator =
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_array_iterator(array))
                        .ok_or(RunError::BadToken)?;
                Ok(Value::Object(iterator))
            }
            "setForEach" => {
                // `callback(value, value, set)` over a live cursor.
                let set_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                self.collection_for_each(set_object, callback, true)
            }
            _ => Err(RunError::BadToken),
        }
    }
}
