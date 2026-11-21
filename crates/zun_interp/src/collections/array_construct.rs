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
                        "TypeError: Cannot convert undefined or null to object".into(),
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
                // A typed array is filled in place, through its own elements.
                if crate::typed_arrays::element_len(target).is_some() {
                    return self.array_method_on_typed_array("fill", target, params);
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
                        "TypeError: Cannot convert undefined or null to object".into(),
                    )
                })?;
                if zun_gc::with_current_heap(|heap| heap.array_len(target)).is_some() {
                    return self.call_managed_collection("arrayReverse", &[Value::Object(target)]);
                }
                // A typed array is reversed in place, through its own elements.
                if crate::typed_arrays::element_len(target).is_some() {
                    return self.array_method_on_typed_array("reverse", target, Vec::new());
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
                        "TypeError: Cannot convert undefined or null to object".into(),
                    )
                })?;
                let is_push = name == "arrayLikePush";
                // A typed array cannot change its length: the generic algorithm
                // throws at the step that would.
                if crate::typed_arrays::element_len(target).is_some() {
                    let method = if is_push { "push" } else { "pop" };
                    let items = match (is_push, args.get(1).and_then(object)) {
                        (true, Some(list)) => self.read_array_values(list)?,
                        _ => Vec::new(),
                    };
                    return self.array_method_on_typed_array(method, target, items);
                }
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
            "arrayFromArgumentList" => {
                // An argument list must be an object: a string is array-like for
                // the callers of `arrayFromArrayLike`, but not here.
                let primitive = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .unwrap_or(Value::Void),
                    other => other.clone(),
                };
                // A BigInt is a heap object here but a primitive in JS.
                let is_object = match &primitive {
                    Value::Object(o) => {
                        zun_gc::with_current_heap(|heap| heap.bigint_value(*o).is_none())
                    }
                    _ => false,
                };
                if !is_object {
                    return Err(RunError::Native(
                        "TypeError: CreateListFromArrayLike called on non-object".into(),
                    ));
                }
                self.col_array_construct("arrayFromArrayLikeArgs", args)
            }
            "arrayFromArrayLike" | "arrayFromArrayLikeArgs" => {
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
                    // The receiver of a borrowed `Array.prototype` method goes
                    // through ToObject: `null` and `undefined` cannot convert,
                    // and any other primitive is a wrapper with no indexed
                    // properties. An argument list never reaches this arm
                    // with a primitive (`arrayFromArgumentList` rejects one).
                    if name == "arrayFromArrayLike" {
                        if matches!(concrete, Value::Void | Value::Null) {
                            return Err(RunError::Native(
                                "TypeError: Cannot convert undefined or null to object".into(),
                            ));
                        }
                        return Ok(Value::Object(zun_gc::with_current_heap_mut(|heap| {
                            heap.alloc_array(0)
                        })));
                    }
                    return Err(RunError::Native(
                        "TypeError: CreateListFromArrayLike called on non-object".into(),
                    ));
                };
                if zun_gc::with_current_heap(|heap| heap.array_len(source)).is_some() {
                    return Ok(Value::Object(source));
                }
                // A typed array is an array-like over its elements.
                if crate::typed_arrays::element_len(source).is_some() {
                    return Ok(Value::Object(self.typed_array_snapshot(source)));
                }
                // A `String` wrapper instance is an array-like over the code
                // units of the string it holds. Its indices are not data slots,
                // so the generic index loop below read `undefined` for each and
                // `Array.prototype.join.call(Object("xyz"), "+")` answered "++".
                if let Some(text) = self.string_wrapper_text(source) {
                    return self.col_array_construct(name, &[Value::Str(text.into())]);
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
                // An argument list is bounded by the engine's stack, an ordinary
                // array-like by the array length limit; both are a RangeError
                // rather than an attempt to build the list.
                let (limit, limit_message) = if name == "arrayFromArrayLikeArgs" {
                    (1_048_576.0, "Maximum call stack size exceeded")
                } else {
                    (4294967295.0, "Invalid array length")
                };
                let length = if raw.is_nan() || raw <= 0.0 {
                    0usize
                } else if raw.trunc() > limit {
                    return Err(match self.new_error_object("RangeError", limit_message) {
                        Some(object) => RunError::Thrown(Value::Object(object)),
                        None => RunError::Native(format!("RangeError: {limit_message}")),
                    });
                } else {
                    raw.trunc() as usize
                };
                let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                // `dynamic_member_get` and `proxy_key_op` run user getters and proxy traps, which
                // can collect; the source and the result are read back through guards after each.
                zun_gc::root!(source = source);
                zun_gc::root!(target = target);
                for index in 0..length {
                    // An absent index stays a hole so generic receivers keep `in` shape.
                    let element = self.dynamic_member_get(source.get(), &index.to_string())?;
                    let present = if zun_gc::with_current_heap(|heap| {
                        heap.proxy_target(source.get()).is_some()
                    }) {
                        matches!(element, Value::Void)
                            .then(|| self.proxy_key_op(source.get(), &index.to_string(), false))
                            .transpose()?
                            .unwrap_or(true)
                    } else {
                        !matches!(element, Value::Void)
                            || zun_gc::with_current_heap(|heap| {
                                heap.dynamic_has(source.get(), &index.to_string())
                            })
                    };
                    if !present {
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.array_push(target.get(), zun_core::Any::hole())
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
                    zun_gc::with_current_heap_mut(|heap| heap.array_push(target.get(), boxed));
                }
                Ok(Value::Object(target.get()))
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
                // `protocol_done`/`protocol_next` run user `next()` code and can collect; the
                // wrapper and the rest array are read back through guards.
                zun_gc::root!(wrapper = wrapper);
                zun_gc::root!(target = target);
                while !matches!(self.protocol_done(wrapper.get())?, Value::Bool(true)) {
                    let value = self.protocol_next(wrapper.get())?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    zun_gc::with_current_heap_mut(|heap| heap.array_push(target.get(), boxed));
                }
                Ok(Value::Object(target.get()))
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
                // `Array.from(items, mapFn)`: the mapper is validated before the
                // source is touched. `undefined` means no mapper; anything that is
                // neither undefined nor a function is a TypeError. A mapper that
                // arrives boxed as `any` is unwrapped to its function handle.
                let mapped_args;
                let args = if name == "arrayFromMapped" {
                    match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                        Value::Function(handle) => {
                            mapped_args = [args[0].clone(), Value::Function(handle)];
                            &mapped_args[..]
                        }
                        Value::Void => {
                            return self.call_managed_collection("arrayFrom", &args[..1]);
                        }
                        other => {
                            let shown = match other {
                                Value::Str(text) => format!("string \"{text}\""),
                                Value::Int(number, _) => format!("number {number}"),
                                Value::Float(number, _) if number.fract() == 0.0 => {
                                    format!("number {}", number as i64)
                                }
                                Value::Float(number, _) => format!("number {number}"),
                                Value::Bool(flag) => format!("boolean {flag}"),
                                Value::Null => "object null".to_string(),
                                _ => "object".to_string(),
                            };
                            return Err(RunError::Native(format!(
                                "TypeError: {shown} is not a function"
                            )));
                        }
                    }
                } else {
                    args
                };
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
                        // A typed array is read element by element through the
                        // shared view reader, so a BigInt kind yields BigInts.
                        if name == "arrayFrom"
                            && crate::typed_arrays::element_len(source).is_some()
                        {
                            return Ok(Value::Object(self.typed_array_snapshot(source)));
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
                                        other => super::callback_handle(other)
                                            .ok_or(RunError::BadToken)?,
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
                                        // An object literal's `[Symbol.iterator]() {}`
                                        // lands in the symbol-**function** slot, not the
                                        // data slot, so both have to be consulted: only
                                        // checking the data slot made a throwing
                                        // iterator look like a missing one, and the
                                        // array-like fallback below answered `[]` where
                                        // the TypeError had to propagate.
                                        heap.dynamic_get_symbol_function(
                                            source,
                                            zun_core::WellKnownSymbol::Iterator.id(),
                                        )
                                        .is_some()
                                            || heap
                                                .dynamic_get_symbol(
                                                    source,
                                                    zun_core::WellKnownSymbol::Iterator.id(),
                                                )
                                                .and_then(|any| Value::unbox_any(any, heap))
                                                .is_some_and(|v| !matches!(v, Value::Void))
                                    });
                                    if present {
                                        return Err(error);
                                    }
                                    // An array-like object: `length` through
                                    // ToLength, then one index read each, which
                                    // runs getters and walks the prototype
                                    // chain. The hand-rolled reader that used
                                    // to stand here required a numeric own
                                    // `length` and read only data slots, so a
                                    // string length or an accessor index
                                    // produced an empty array.
                                    let list = self.call_managed_collection(
                                        "arrayFromArrayLike",
                                        &[Value::Object(source)],
                                    )?;
                                    let built = object(&list).ok_or(RunError::BadToken)?;
                                    // CreateListFromArrayLike leaves an absent
                                    // index a hole, so a generic receiver keeps
                                    // its `in` shape; `Array.from` instead
                                    // defines every index, so an absent one is
                                    // a present `undefined`.
                                    zun_gc::with_current_heap_mut(|heap| {
                                        let length = heap.array_len(built).unwrap_or(0);
                                        for index in 0..length {
                                            if heap
                                                .array_get(built, index)
                                                .is_some_and(|element| element.is_hole())
                                            {
                                                heap.array_set(
                                                    built,
                                                    index,
                                                    zun_core::Any::void(),
                                                );
                                            }
                                        }
                                    });
                                    built
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
                    other => super::callback_handle(other).ok_or(RunError::BadToken)?,
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
                // The spread appends into `target` and the iteration protocol can run user code
                // that collects; `target` is also returned at the end, so it is held in a guard.
                zun_gc::root!(target = target);
                match &concrete {
                    Value::Str(text) => {
                        // Spread a string by Unicode code point.
                        let points: Vec<String> =
                            text.chars().map(|point| point.to_string()).collect();
                        for point in points {
                            zun_gc::with_current_heap_mut(|heap| {
                                let id = heap.intern_string(point.as_str());
                                heap.array_push(target.get(), zun_core::Any::string(id));
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
                            zun_gc::root!(source = source);
                            for index in 0..length {
                                let value = zun_gc::with_current_heap(|heap| {
                                    heap.array_get(source.get(), index)
                                })
                                .unwrap_or_else(zun_core::Any::void);
                                // Spread materialises holes as undefined.
                                let value = if value.is_hole() {
                                    zun_core::Any::void()
                                } else {
                                    value
                                };
                                zun_gc::with_current_heap_mut(|heap| {
                                    heap.array_push(target.get(), value)
                                });
                            }
                        } else {
                            self.drive_iterable(source, target.get(), |heap, array, value| {
                                heap.array_push(array, value);
                            })?;
                        }
                    }
                }
                Ok(Value::Object(target.get()))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
