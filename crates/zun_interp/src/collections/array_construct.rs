//! Array isArray/destructure/array-like/from/spread bulk native methods (hot).
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    #[inline]
    pub(super) fn col_array_construct(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RunError> {
        match name {
            "arrayIsArray" => {
                let value = args.first().ok_or(RunError::BadToken)?;
                let is_array = object(value)
                    .map(|obj| {
                        zun_gc::with_current_heap(|heap| {
                            let mut obj = obj;
                            while let Some(target) = heap.proxy_target(obj) {
                                obj = target;
                            }
                            heap.array_len(obj).is_some() && !heap.array_is_arguments(obj)
                        })
                    })
                    .unwrap_or(false);
                Ok(Value::Bool(is_array))
            }
            "arrayDestructure" => {
                // `[a, b] = iterable`: take at most `count` values through the
                // iteration protocol, then close the iterator if it has more.
                let concrete = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .unwrap_or(Value::Void)
                    }
                    other => other.clone(),
                };
                let count = match args.get(1) {
                    Some(Value::Int(n, _)) => *n as usize,
                    Some(Value::Float(n, _)) => *n as usize,
                    _ => 0,
                };
                let source = match &concrete {
                    Value::Object(_) | Value::Any(_) => object(&concrete),
                    _ => None,
                };
                let lazy = source.filter(|source| {
                    zun_gc::with_current_heap(|heap| heap.array_len(*source).is_none())
                });
                let Some(source) = lazy else {
                    return self.call_managed_collection("arrayFrom", &args[..1]);
                };
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                // Resolving and stepping the iterator run user code that can evacuate the
                // target and the wrapper; every read goes through the guards.
                zun_gc::root!(target = target);
                let wrapper =
                    object(&self.protocol_iterator_new(source)?).ok_or(RunError::BadToken)?;
                zun_gc::root!(wrapper = wrapper);
                let mut exhausted = false;
                for _ in 0..count {
                    if matches!(self.protocol_done(wrapper.get())?, Value::Bool(true)) {
                        exhausted = true;
                        break;
                    }
                    let value = self.protocol_next(wrapper.get())?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    zun_gc::with_current_heap_mut(|heap| heap.array_push(target.get(), boxed));
                }
                if !exhausted {
                    self.call_managed_collection(
                        "generatorIteratorClose",
                        &[Value::Object(wrapper.get()), Value::Bool(false)],
                    )?;
                }
                Ok(Value::Object(target.get()))
            }
            "arrayLikeFill" => {
                // `Array.prototype.fill.call(receiver, value, start, end)`: an Array
                // takes the typed path; an array-like uses the generic algorithm.
                let target = object(args.first().ok_or(RunError::BadToken)?).ok_or_else(|| {
                    RunError::Native(
                        "TypeError: Array.prototype.fill called on null or undefined".into(),
                    )
                })?;
                let list =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let mut params: Vec<Value> = Vec::with_capacity(3);
                for index in 0..3 {
                    let any = zun_gc::with_current_heap(|heap| heap.array_get(list, index));
                    params.push(match any {
                        Some(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                            .unwrap_or(Value::Void),
                        None => Value::Void,
                    });
                }
                if zun_gc::with_current_heap(|heap| heap.array_len(target)).is_some() {
                    let mut forwarded = vec![Value::Object(target)];
                    forwarded.extend(params);
                    return self.call_managed_collection("arrayFill", &forwarded);
                }
                let length_value = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get(target, "length")
                        .and_then(|any| Value::unbox_any(any, heap))
                });
                let raw = match length_value {
                    Some(value) => self.to_number_value(value)?,
                    None => 0.0,
                };
                let length = if raw.is_nan() || raw <= 0.0 {
                    0.0
                } else {
                    raw.trunc().min(9007199254740991.0)
                };
                let mut bounds = [0.0f64; 2];
                for (slot, default) in [(1usize, 0.0f64), (2usize, length)] {
                    let value = params[slot].clone();
                    let relative = if matches!(value, Value::Void) {
                        default
                    } else {
                        let n = self.to_number_value(value)?;
                        let integer = if n.is_nan() { 0.0 } else { n.trunc() };
                        if integer < 0.0 {
                            (length + integer).max(0.0)
                        } else {
                            integer.min(length)
                        }
                    };
                    bounds[slot - 1] = relative;
                }
                let boxed = zun_gc::with_current_heap_mut(|heap| params[0].box_any(heap))
                    .ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| {
                    let mut index = bounds[0];
                    while index < bounds[1] {
                        heap.dynamic_set(target, format!("{}", index as u64), boxed);
                        index += 1.0;
                    }
                });
                Ok(Value::Object(target))
            }
            "arrayLikeReverse" => {
                // `Array.prototype.reverse.call(receiver)`: an Array reverses in place;
                // an array-like swaps present indices (holes move with their slots).
                let first = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                if let Value::Str(_) = first {
                    return Err(RunError::Native(
                        "TypeError: Cannot assign to read only property '0' of object '[object String]'".into(),
                    ));
                }
                let target = object(&first).ok_or_else(|| {
                    RunError::Native(
                        "TypeError: Array.prototype.reverse called on null or undefined".into(),
                    )
                })?;
                if zun_gc::with_current_heap(|heap| heap.array_len(target)).is_some() {
                    return self.call_managed_collection("arrayReverse", &[Value::Object(target)]);
                }
                let length_value = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get(target, "length")
                        .and_then(|any| Value::unbox_any(any, heap))
                });
                let raw = match length_value {
                    Some(value) => self.to_number_value(value)?,
                    None => 0.0,
                };
                let length = if raw.is_nan() || raw <= 0.0 {
                    0u64
                } else {
                    raw.trunc().min(4294967295.0) as u64
                };
                zun_gc::with_current_heap_mut(|heap| {
                    for lower in 0..length / 2 {
                        let upper = length - 1 - lower;
                        let (lk, uk) = (lower.to_string(), upper.to_string());
                        let lv = heap
                            .dynamic_has(target, &lk)
                            .then(|| heap.dynamic_get(target, &lk))
                            .flatten();
                        let uv = heap
                            .dynamic_has(target, &uk)
                            .then(|| heap.dynamic_get(target, &uk))
                            .flatten();
                        match uv {
                            Some(v) => {
                                heap.dynamic_set(target, lk.clone(), v);
                            }
                            None => {
                                heap.dynamic_delete(target, &lk);
                            }
                        }
                        match lv {
                            Some(v) => {
                                heap.dynamic_set(target, uk.clone(), v);
                            }
                            None => {
                                heap.dynamic_delete(target, &uk);
                            }
                        }
                    }
                });
                Ok(Value::Object(target))
            }
            "arrayLikePush" | "arrayLikePop" => {
                // `Array.prototype.push/pop.call(receiver, ...)`: an Array mutates in
                // place; any other object is treated as an array-like (ToLength on
                // `length`, index keys, `length` written back).
                let target = object(args.first().ok_or(RunError::BadToken)?).ok_or_else(|| {
                    RunError::Native(
                        "TypeError: Array.prototype method called on null or undefined".into(),
                    )
                })?;
                let is_push = name == "arrayLikePush";
                let items: Vec<zun_core::Any> = if is_push {
                    let list =
                        object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap(|heap| {
                        let length = heap.array_len(list).unwrap_or(0);
                        (0..length)
                            .filter_map(|index| heap.array_get(list, index))
                            .collect()
                    })
                } else {
                    Vec::new()
                };
                if zun_gc::with_current_heap(|heap| heap.array_len(target)).is_some() {
                    if is_push {
                        let mut length = 0usize;
                        for item in items {
                            length =
                                zun_gc::with_current_heap_mut(|heap| heap.array_push(target, item))
                                    .unwrap_or(length);
                        }
                        if length == 0 {
                            length = zun_gc::with_current_heap(|heap| heap.array_len(target))
                                .unwrap_or(0);
                        }
                        return Ok(Value::Float(length as f64, zun_il::NumKind::F64));
                    }
                    let popped = zun_gc::with_current_heap_mut(|heap| heap.array_pop(target));
                    return match popped {
                        Some(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                            .ok_or(RunError::BadToken),
                        None => Ok(Value::Void),
                    };
                }
                if zun_gc::with_current_heap(|heap| heap.proxy_target(target).is_some()) {
                    return self.proxy_push_pop(target, is_push, items);
                }
                let length_value = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get(target, "length")
                        .and_then(|any| Value::unbox_any(any, heap))
                });
                let raw = match length_value {
                    Some(value) => self.to_number_value(value)?,
                    None => 0.0,
                };
                let length = if raw.is_nan() || raw <= 0.0 {
                    0.0
                } else {
                    raw.trunc().min(9007199254740991.0)
                };
                if is_push {
                    let count = items.len() as f64;
                    if length + count > 9007199254740991.0 {
                        return Err(RunError::Native(format!(
                            "TypeError: Pushing {count} elements on an array-like of length {length} is disallowed, as the total surpasses 2**53-1"
                        )));
                    }
                    zun_gc::with_current_heap_mut(|heap| {
                        for (offset, item) in items.into_iter().enumerate() {
                            heap.dynamic_set(
                                target,
                                format!("{}", length as u64 + offset as u64),
                                item,
                            );
                        }
                        let total = Value::Float(length + count, zun_il::NumKind::F64)
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        heap.dynamic_set(target, "length", total);
                    });
                    return Ok(Value::Float(length + count, zun_il::NumKind::F64));
                }
                if length == 0.0 {
                    zun_gc::with_current_heap_mut(|heap| {
                        let zero = Value::Float(0.0, zun_il::NumKind::F64)
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        heap.dynamic_set(target, "length", zero);
                    });
                    return Ok(Value::Void);
                }
                let last = format!("{}", length as u64 - 1);
                let element = zun_gc::with_current_heap(|heap| heap.dynamic_get(target, &last));
                let value = match element {
                    Some(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                        .unwrap_or(Value::Void),
                    None => Value::Void,
                };
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_delete(target, &last);
                    let total = Value::Float(length - 1.0, zun_il::NumKind::F64)
                        .box_any(heap)
                        .unwrap_or_else(zun_core::Any::null);
                    heap.dynamic_set(target, "length", total);
                });
                Ok(value)
            }
            "arrayFromArrayLike" => {
                // CreateListFromArrayLike: `length` then index reads, never iteration.
                let concrete = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .unwrap_or(Value::Void)
                    }
                    other => other.clone(),
                };
                if let Value::Str(text) = &concrete {
                    // A string is array-like over its UTF-16 code units. Rust
                    // strings cannot hold lone surrogates, so the low half of an
                    // astral character is a U+FFFD placeholder (the length holds).
                    let mut units: Vec<String> = Vec::new();
                    for ch in text.chars() {
                        units.push(ch.to_string());
                        if ch.len_utf16() == 2 {
                            units.push("\u{FFFD}".to_string());
                        }
                    }
                    let array = zun_gc::with_current_heap_mut(|heap| {
                        let array = heap.alloc_array(units.len());
                        for (index, unit) in units.iter().enumerate() {
                            let value = zun_core::Any::string(heap.intern_string(unit.as_str()));
                            heap.array_set(array, index, value);
                        }
                        array
                    });
                    return Ok(Value::Object(array));
                }
                let Some(source) = (match &concrete {
                    Value::Object(_) => object(&concrete),
                    _ => None,
                }) else {
                    return Err(RunError::Native(
                        "TypeError: CreateListFromArrayLike called on non-object".into(),
                    ));
                };
                if zun_gc::with_current_heap(|heap| heap.array_len(source)).is_some() {
                    return Ok(Value::Object(source));
                }
                let length_value = self.dynamic_member_get(source, "length")?;
                let length_value = match length_value {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                            .unwrap_or(Value::Void)
                    }
                    other => other,
                };
                let raw = self.to_number_value(length_value)?;
                let length = if raw.is_nan() || raw <= 0.0 {
                    0usize
                } else {
                    raw.trunc().min(4294967295.0) as usize
                };
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                let roots = [source, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    // An absent index stays a hole so generic receivers keep `in` shape.
                    let element = self.dynamic_member_get(roots[0], &index.to_string())?;
                    let present = if zun_gc::with_current_heap(|heap| {
                        heap.proxy_target(roots[0]).is_some()
                    }) {
                        matches!(element, Value::Void)
                            .then(|| self.proxy_key_op(roots[0], &index.to_string(), false))
                            .transpose()?
                            .unwrap_or(true)
                    } else {
                        !matches!(element, Value::Void)
                            || zun_gc::with_current_heap(|heap| {
                                heap.dynamic_has(roots[0], &index.to_string())
                            })
                    };
                    if !present {
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.array_push(roots[1], zun_core::Any::hole())
                        });
                        continue;
                    }
                    let boxed = match &element {
                        Value::Function(handle) => zun_gc::with_current_heap_mut(|heap| {
                            let record = heap.alloc_dynamic();
                            heap.dynamic_set_function(record, "__boxedFn", *handle);
                            Value::Object(record).box_any(heap)
                        }),
                        other => zun_gc::with_current_heap_mut(|heap| other.box_any(heap)),
                    }
                    .unwrap_or_else(zun_core::Any::void);
                    zun_gc::with_current_heap_mut(|heap| heap.array_push(roots[1], boxed));
                }
                Ok(Value::Object(roots[1]))
            }
            "destructOpen" => {
                // Lazy array-destructuring source: an Array (or any eagerly
                // materialisable value) becomes an Array; a protocol iterable
                // becomes an iterator wrapper pulled one element at a time.
                let concrete = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .unwrap_or(Value::Void)
                    }
                    other => other.clone(),
                };
                let source = match &concrete {
                    Value::Object(_) | Value::Any(_) => object(&concrete),
                    _ => None,
                };
                let lazy = source.filter(|source| {
                    zun_gc::with_current_heap(|heap| heap.array_len(*source).is_none())
                });
                let Some(source) = lazy else {
                    return self.call_managed_collection("arrayFrom", &args[..1]);
                };
                self.protocol_iterator_new(source)
            }
            "destructStep" => {
                let wrapper =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.array_len(wrapper)).is_some() {
                    return self.call_managed_collection("arrayAt", args);
                }
                if matches!(self.protocol_done(wrapper)?, Value::Bool(true)) {
                    return Ok(Value::Void);
                }
                self.protocol_next(wrapper)
            }
            "destructRest" => {
                let wrapper =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.array_len(wrapper)).is_some() {
                    return self.call_managed_collection("arraySlice", args);
                }
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                let roots = [wrapper, target];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                while !matches!(self.protocol_done(roots[0])?, Value::Bool(true)) {
                    let value = self.protocol_next(roots[0])?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    zun_gc::with_current_heap_mut(|heap| heap.array_push(roots[1], boxed));
                }
                Ok(Value::Object(roots[1]))
            }
            "destructFinish" => {
                // Close the iterator unless it already reported `done`.
                let wrapper =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.array_len(wrapper)).is_some() {
                    return Ok(Value::Bool(true));
                }
                let exhausted = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get(wrapper, "__zun_done")
                        .and_then(|any| any.as_bool())
                })
                .unwrap_or(false);
                if exhausted {
                    return Ok(Value::Bool(true));
                }
                let throwing = matches!(args.get(1), Some(Value::Bool(true)));
                self.call_managed_collection(
                    "generatorIteratorClose",
                    &[Value::Object(wrapper), Value::Bool(throwing)],
                )
            }
            "arrayFrom" | "arrayFromMapped" => {
                // Build the base array from the source (a managed Array or a
                // string; other iterables are deferred).
                let concrete = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .unwrap_or(Value::Void)
                    }
                    other => other.clone(),
                };
                let base = match &concrete {
                    Value::Str(text) => {
                        let points: Vec<String> =
                            text.chars().map(|point| point.to_string()).collect();
                        zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(points.len());
                            for (index, point) in points.iter().enumerate() {
                                let value =
                                    zun_core::Any::string(heap.intern_string(point.as_str()));
                                heap.array_set(array, index, value);
                            }
                            array
                        })
                    }
                    _ => {
                        let Some(source) = object(&concrete) else {
                            return Err(RunError::Thrown(Value::Str(zun_core::ZStr::from(
                                "Array.from: value is not array-like",
                            ))));
                        };
                        // A proxy is read through its traps (`length`, then each index).
                        if zun_gc::with_current_heap(|heap| heap.proxy_target(source).is_some()) {
                            let list = self.call_managed_collection(
                                "arrayFromArrayLike",
                                &[Value::Object(source)],
                            )?;
                            if name == "arrayFrom" {
                                return Ok(list);
                            }
                            let mut forwarded = args.to_vec();
                            forwarded[0] = list;
                            return self.call_managed_collection(name, &forwarded);
                        }
                        // A typed array copies its numeric elements into a plain array.
                        if let Some(count) = crate::typed_arrays::element_len(source) {
                            let result =
                                zun_gc::with_current_heap_mut(|heap| heap.alloc_array(count));
                            for index in 0..count {
                                if let Some(value) =
                                    crate::typed_arrays::read_element(source, index)
                                {
                                    zun_gc::with_current_heap_mut(|heap| {
                                        if let Some(any) = value.box_any(heap) {
                                            heap.array_set(result, index, any);
                                        }
                                    });
                                }
                            }
                            if name == "arrayFrom" {
                                return Ok(Value::Object(result));
                            }
                            // Mapped form: fall through with the copy as the source.
                            let _ = result;
                        }
                        let Some(length) = zun_gc::with_current_heap(|heap| {
                            heap.array_len(source).filter(|_| {
                                heap.dynamic_get_symbol_function(
                                    source,
                                    zun_core::WellKnownSymbol::Iterator.id(),
                                )
                                .is_none()
                            })
                        }) else {
                            // The mapped form streams the iterable: each element is
                            // mapped as it is produced, and a throwing mapper closes
                            // the iterator (IteratorClose) before propagating.
                            if name == "arrayFromMapped" {
                                if let Ok(wrapper_value) = self.protocol_iterator_new(source) {
                                    let wrapper =
                                        object(&wrapper_value).ok_or(RunError::BadToken)?;
                                    let callback = match args.get(1).ok_or(RunError::BadToken)? {
                                        Value::Function(handle) => *handle,
                                        _ => return Err(RunError::BadToken),
                                    };
                                    let result =
                                        zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                                    // Stepping the iterator and the mapper both re-enter managed
                                    // code and can evacuate the wrapper, the result and the mapper's
                                    // closure environment; each is read back through its guard.
                                    zun_gc::root!(wrapper = wrapper);
                                    zun_gc::root!(result = result);
                                    zun_gc::root!(callback = callback);
                                    let mut index = 0usize;
                                    loop {
                                        if matches!(
                                            self.protocol_done(wrapper.get())?,
                                            Value::Bool(true)
                                        ) {
                                            break;
                                        }
                                        let value = self.protocol_next(wrapper.get())?;
                                        let callback_args = vec![
                                            value,
                                            Value::Any(zun_core::Any::float(index as f64)),
                                        ];
                                        match self.invoke_callback(callback.get(), callback_args) {
                                            Ok(produced) => {
                                                let boxed = zun_gc::with_current_heap_mut(|heap| {
                                                    produced.box_any(heap)
                                                })
                                                .ok_or(RunError::BadToken)?;
                                                zun_gc::with_current_heap_mut(|heap| {
                                                    heap.array_push(result.get(), boxed)
                                                });
                                            }
                                            Err(error) => {
                                                let _ = self.call_managed_collection(
                                                    "generatorIteratorClose",
                                                    &[
                                                        Value::Object(wrapper.get()),
                                                        Value::Bool(true),
                                                    ],
                                                );
                                                return Err(error);
                                            }
                                        }
                                        index += 1;
                                    }
                                    return Ok(Value::Object(result.get()));
                                }
                            }
                            // Any other iterable (Set, Map, iterator, user
                            // iterable) is drained through the iteration protocol.
                            let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                            let drained =
                                self.drive_iterable(source, target, |heap, array, value| {
                                    heap.array_push(array, value);
                                });
                            let array = match drained {
                                Ok(array) => array,
                                Err(error) => {
                                    // A present, non-undefined `[Symbol.iterator]` that fails is an error, not an array-like.
                                    let present = zun_gc::with_current_heap(|heap| {
                                        heap.dynamic_get_symbol(
                                            source,
                                            zun_core::WellKnownSymbol::Iterator.id(),
                                        )
                                        .and_then(|any| Value::unbox_any(any, heap))
                                        .is_some_and(|v| !matches!(v, Value::Void))
                                    });
                                    if present {
                                        return Err(error);
                                    }
                                    // An array-like plain object (`{ length: n, 0: .. }`).
                                    let built = zun_gc::with_current_heap_mut(|heap| {
                                        if !heap.is_dynamic_object(source) {
                                            return None;
                                        }
                                        let length =
                                            heap.dynamic_get(source, "length").and_then(|any| {
                                                match Value::unbox_any(any, heap) {
                                                    Some(Value::Float(n, _)) if n >= 0.0 => {
                                                        Some(n as usize)
                                                    }
                                                    Some(Value::Int(n, _)) if n >= 0 => {
                                                        Some(n as usize)
                                                    }
                                                    _ => None,
                                                }
                                            });
                                        let length = length.unwrap_or(0);
                                        let array = heap.alloc_array(length);
                                        for index in 0..length {
                                            // Index reads walk the prototype chain
                                            // (an inherited element fills the slot).
                                            let key = index.to_string();
                                            let mut cursor = Some(source);
                                            let mut found = None;
                                            while let Some(current) = cursor {
                                                if let Some(value) = heap.dynamic_get(current, &key)
                                                {
                                                    found = Some(value);
                                                    break;
                                                }
                                                cursor = heap
                                                    .dynamic_prototype(current)
                                                    .and_then(|proto| proto.as_object())
                                                    .map(|addr| unsafe {
                                                        zun_gc::Obj::from_addr(addr as *mut u64)
                                                    });
                                            }
                                            let value = found.unwrap_or_else(zun_core::Any::void);
                                            heap.array_set(array, index, value);
                                        }
                                        Some(array)
                                    });
                                    built.ok_or_else(|| {
                                        RunError::Thrown(Value::Str(zun_core::ZStr::from(
                                            "Array.from: value is not array-like",
                                        )))
                                    })?
                                }
                            };
                            if name == "arrayFrom" {
                                return Ok(Value::Object(array));
                            }
                            // Mapped form below re-reads `base`; hand the drained
                            // array to the shared tail by recursing on it.
                            let mut forwarded = args.to_vec();
                            forwarded[0] = Value::Object(array);
                            return self.call_managed_collection(name, &forwarded);
                        };
                        zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(length);
                            for index in 0..length {
                                let value = heap
                                    .array_get(source, index)
                                    .unwrap_or_else(zun_core::Any::void);
                                // Array.from materialises holes as undefined.
                                let value = if value.is_hole() {
                                    zun_core::Any::void()
                                } else {
                                    value
                                };
                                heap.array_set(array, index, value);
                            }
                            array
                        })
                    }
                };
                if name == "arrayFrom" {
                    return Ok(Value::Object(base));
                }
                // Mapped form: invoke the callback `(value, index)` per element.
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(base))
                    .ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(length));
                // The mapper can evacuate the base, the result and its own closure
                // environment between iterations.
                zun_gc::root!(base = base);
                zun_gc::root!(result = result);
                zun_gc::root!(callback = callback);
                for index in 0..length {
                    let element =
                        zun_gc::with_current_heap(|heap| heap.array_get(base.get(), index))
                            .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                    ];
                    let produced = self.invoke_callback(callback.get(), callback_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.array_set(result.get(), index, boxed)
                    });
                }
                Ok(Value::Object(result.get()))
            }
            "arraySpreadAppend" => {
                // Append every element of `source` (a managed Array or a
                // string, matching `arrayFrom`) to `target`, in order.
                let target =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let concrete = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .unwrap_or(Value::Void)
                    }
                    other => other.clone(),
                };
                match &concrete {
                    Value::Str(text) => {
                        // Spread a string by Unicode code point.
                        let points: Vec<String> =
                            text.chars().map(|point| point.to_string()).collect();
                        let roots = [target];
                        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                        for point in points {
                            zun_gc::with_current_heap_mut(|heap| {
                                let id = heap.intern_string(point.as_str());
                                heap.array_push(roots[0], zun_core::Any::string(id));
                            });
                        }
                    }
                    _ => {
                        let Some(source) = object(&concrete) else {
                            return match self
                                .new_error_object("TypeError", "spread: value is not iterable")
                            {
                                Some(error) => Err(RunError::Thrown(Value::Object(error))),
                                None => Err(RunError::BadToken),
                            };
                        };
                        // A managed Array keeps its direct fast path; anything
                        // else (a Set/Map or a user `[Symbol.iterator]` object)
                        // is drained through the iteration protocol.
                        if let Some(length) = zun_gc::with_current_heap(|heap| {
                            heap.array_len(source).filter(|_| {
                                heap.dynamic_get_symbol_function(
                                    source,
                                    zun_core::WellKnownSymbol::Iterator.id(),
                                )
                                .is_none()
                            })
                        }) {
                            let roots = [source, target];
                            let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                            for index in 0..length {
                                let value = zun_gc::with_current_heap(|heap| {
                                    heap.array_get(roots[0], index)
                                })
                                .unwrap_or_else(zun_core::Any::void);
                                // Spread materialises holes as undefined.
                                let value = if value.is_hole() {
                                    zun_core::Any::void()
                                } else {
                                    value
                                };
                                zun_gc::with_current_heap_mut(|heap| {
                                    heap.array_push(roots[1], value)
                                });
                            }
                        } else {
                            self.drive_iterable(source, target, |heap, array, value| {
                                heap.array_push(array, value);
                            })?;
                        }
                    }
                }
                Ok(Value::Object(target))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
