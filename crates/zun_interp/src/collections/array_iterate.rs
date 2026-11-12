//! Array iteration/reduce/sort/splice/flat/fill bulk native methods (hot).
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    #[inline]
    #[allow(clippy::neg_cmp_op_on_partial_ord)] // !(x > 0.0) preserves JS sort NaN semantics
    pub(super) fn col_array_iterate(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RunError> {
        match name {
            "arrayForEach" | "arrayMap" | "arrayFilter" | "arrayFind" | "arrayFindIndex"
            | "arraySome" | "arrayEvery" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                // A fresh result array is needed only for `map`/`filter`.
                let result = if name == "arrayMap" {
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_array(length))
                } else if name == "arrayFilter" {
                    zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0))
                } else {
                    source
                };
                // Root both the source and the result across callback re-entry;
                // a GC during `invoke_callback` may relocate either, so every
                // access reads the (possibly rewritten) addresses back out.
                let roots = [source, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    // `find`/`findIndex` visit holes as `undefined`; every other
                    // method skips holes and indices past a shrunken array.
                    let visits_holes = matches!(name, "arrayFind" | "arrayFindIndex");
                    let raw_element = self.array_element_value(roots[0], index)?;
                    let element = match raw_element {
                        Some(element) if !(element.is_hole() && !visits_holes) => element,
                        _ if visits_holes => zun_core::Any::void(),
                        _ => {
                            if name == "arrayMap" {
                                zun_gc::with_current_heap_mut(|heap| {
                                    heap.array_set(roots[1], index, zun_core::Any::hole())
                                });
                            }
                            continue;
                        }
                    };
                    let element = if element.is_hole() {
                        zun_core::Any::void()
                    } else {
                        element
                    };
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let produced = self.invoke_callback(callback, callback_args)?;
                    match name {
                        "arrayForEach" => {}
                        "arrayMap" => {
                            let boxed =
                                zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                                    .ok_or(RunError::BadToken)?;
                            zun_gc::with_current_heap_mut(|heap| {
                                heap.array_set(roots[1], index, boxed)
                            });
                        }
                        "arrayFilter" => {
                            let keep = zun_gc::with_current_heap(|heap| is_truthy(&produced, heap));
                            if keep {
                                let boxed = zun_gc::with_current_heap_mut(|heap| {
                                    Value::Any(element).box_any(heap)
                                })
                                .ok_or(RunError::BadToken)?;
                                zun_gc::with_current_heap_mut(|heap| {
                                    heap.array_push(roots[1], boxed)
                                });
                            }
                        }
                        "arrayFind" => {
                            if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                                // Return the concrete element value (unboxed),
                                // matching ordinary element access.
                                return zun_gc::with_current_heap(|heap| {
                                    Value::unbox_any(element, heap)
                                })
                                .ok_or(RunError::BadToken);
                            }
                        }
                        "arrayFindIndex" => {
                            if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                                return Ok(Value::Int(index as i64, zun_il::NumKind::I32));
                            }
                        }
                        "arraySome" => {
                            if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                                return Ok(Value::Bool(true));
                            }
                        }
                        "arrayEvery" => {
                            if !zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                                return Ok(Value::Bool(false));
                            }
                        }
                        _ => unreachable!(),
                    }
                }
                match name {
                    "arrayForEach" => Ok(Value::Void),
                    "arrayMap" | "arrayFilter" => Ok(Value::Object(roots[1])),
                    "arrayFind" => Ok(Value::Void),
                    "arrayFindIndex" => Ok(Value::Int(-1, zun_il::NumKind::I32)),
                    "arraySome" => Ok(Value::Bool(false)),
                    "arrayEvery" => Ok(Value::Bool(true)),
                    _ => unreachable!(),
                }
            }
            "arrayReduce" | "arrayReduceRight" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let initial = args.get(2).ok_or(RunError::BadToken)?.clone();
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                // Hold the accumulator inside a one-slot GC array so it survives
                // callback re-entry regardless of whether it is an object.
                let accumulator = zun_gc::with_current_heap_mut(|heap| {
                    let holder = heap.alloc_array(1);
                    if let Some(boxed) = initial.box_any_cell(heap) {
                        heap.array_set(holder, 0, boxed);
                    }
                    holder
                });
                let roots = [source, accumulator];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for step in 0..length {
                    let index = if name == "arrayReduce" {
                        step
                    } else {
                        length - 1 - step
                    };
                    // Holes (and indices deleted or truncated meanwhile) are skipped.
                    let Some(element) = self.array_element_value(roots[0], index)? else {
                        continue;
                    };
                    if element.is_hole() {
                        continue;
                    }
                    let acc = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(acc),
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let next = self.invoke_callback(callback, callback_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| next.box_any_cell(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(roots[1], 0, boxed));
                }
                let result = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
                    .ok_or(RunError::BadToken)?;
                // Unbox to the concrete accumulator value so the result matches
                // the representation an in-source reducer would have returned.
                zun_gc::with_current_heap(|heap| Value::unbox_any(result, heap))
                    .ok_or(RunError::BadToken)
            }
            "arraySortBy" | "arraySortDefault" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let comparator = if name == "arraySortBy" {
                    match args.get(1).ok_or(RunError::BadToken)? {
                        Value::Function(handle) => Some(*handle),
                        Value::Void => None,
                        Value::Any(a) if a.tag() == zun_core::AnyTag::Void => None,
                        _ => return Err(RunError::BadToken),
                    }
                } else {
                    None
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let roots = [source];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                // Insertion sort so each comparison re-enters the interpreter at
                // most once; elements are read back from the rooted source after
                // every comparator call in case a GC relocated the backing store.
                for i in 1..length {
                    let mut j = i;
                    while j > 0 {
                        let left =
                            zun_gc::with_current_heap(|heap| heap.array_get(roots[0], j - 1))
                                .ok_or(RunError::BadToken)?;
                        let right = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], j))
                            .ok_or(RunError::BadToken)?;
                        let ordered_before = match comparator {
                            Some(handle) => {
                                let outcome = self.invoke_callback(
                                    handle,
                                    vec![Value::Any(left), Value::Any(right)],
                                )?;
                                // `<= 0` keeps order; a non-numeric (`NaN`)
                                // result is treated as `0` (stable, no swap).
                                !(self.dyn_to_number(&outcome)? > 0.0)
                            }
                            None => {
                                let (a, b) = zun_gc::with_current_heap(|heap| {
                                    (
                                        stringify_any(left, heap).unwrap_or_default(),
                                        stringify_any(right, heap).unwrap_or_default(),
                                    )
                                });
                                a <= b
                            }
                        };
                        if ordered_before {
                            break;
                        }
                        zun_gc::with_current_heap_mut(|heap| {
                            let a = heap.array_get(roots[0], j - 1);
                            let b = heap.array_get(roots[0], j);
                            if let (Some(a), Some(b)) = (a, b) {
                                heap.array_set(roots[0], j - 1, b);
                                heap.array_set(roots[0], j, a);
                            }
                        });
                        j -= 1;
                    }
                }
                Ok(Value::Object(roots[0]))
            }
            "arrayJoin" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let separator = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(text) => text.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let joined = self.array_join_dyn(source, &separator)?.unwrap_or_default();
                Ok(Value::Str(zun_core::wtf16::normalize(joined).into()))
            }
            "arrayShift" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let removed = zun_gc::with_current_heap_mut(|heap| {
                    if heap.array_len(source).unwrap_or(0) == 0 {
                        None
                    } else {
                        heap.array_remove(source, 0)
                            .and_then(|value| Value::unbox_any(value, heap))
                    }
                });
                Ok(removed.unwrap_or(Value::Void))
            }
            "arrayUnshift" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let length = zun_gc::with_current_heap_mut(|heap| {
                    heap.array_insert(source, 0, value);
                    heap.array_len(source).unwrap_or(0)
                });
                Ok(Value::Int(length as i64, zun_il::NumKind::U64))
            }
            "arraySplice" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let start = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let delete_count = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let removed = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let start = start.min(length);
                    let delete_count = delete_count.min(length - start);
                    let result = heap.alloc_array(delete_count);
                    for offset in 0..delete_count {
                        // Elements shift down on each removal, so the removal
                        // index stays `start`.
                        if let Some(element) = heap.array_remove(source, start) {
                            heap.array_set(result, offset, element);
                        }
                    }
                    result
                });
                Ok(Value::Object(removed))
            }
            "arrayAt" => {
                // ECMAScript relative indexing: a negative index counts from the
                // end; an out-of-range index yields `undefined`.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let raw = self.dyn_to_number(args.get(1).unwrap_or(&Value::Void))?;
                let mut index = raw.trunc();
                if index < 0.0 {
                    index += length as f64;
                }
                if index < 0.0 || index >= length as f64 {
                    return Ok(Value::Void);
                }
                let element =
                    zun_gc::with_current_heap(|heap| heap.array_get(source, index as usize))
                        .ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap(|heap| Value::unbox_any(element, heap))
                    .ok_or(RunError::BadToken)
            }
            "arrayFindLast" | "arrayFindLastIndex" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let roots = [source];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let mut step = length;
                while step > 0 {
                    step -= 1;
                    let index = step;
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let produced = self.invoke_callback(callback, callback_args)?;
                    if zun_gc::with_current_heap(|heap| is_truthy(&produced, heap)) {
                        if name == "arrayFindLast" {
                            return zun_gc::with_current_heap(|heap| {
                                Value::unbox_any(element, heap)
                            })
                            .ok_or(RunError::BadToken);
                        }
                        return Ok(Value::Int(index as i64, zun_il::NumKind::I32));
                    }
                }
                if name == "arrayFindLast" {
                    Ok(Value::Void)
                } else {
                    Ok(Value::Int(-1, zun_il::NumKind::I32))
                }
            }
            "arrayFlat" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let depth_raw = self.dyn_to_number(args.get(1).unwrap_or(&Value::Void))?;
                let depth = if depth_raw.is_nan() || depth_raw < 0.0 {
                    0
                } else {
                    depth_raw.trunc() as i64
                };
                // No callback re-entry: the whole flatten runs inside one heap
                // borrow, so no GC can relocate the arrays mid-build.
                let result = zun_gc::with_current_heap_mut(|heap| {
                    fn flatten(
                        heap: &mut zun_gc::Heap,
                        src: zun_gc::Obj,
                        depth: i64,
                        out: zun_gc::Obj,
                    ) {
                        let length = heap.array_len(src).unwrap_or(0);
                        for index in 0..length {
                            let Some(element) = heap.array_get(src, index) else {
                                continue;
                            };
                            if element.is_hole() {
                                continue;
                            }
                            let child = (depth > 0)
                                .then(|| element.as_object())
                                .flatten()
                                .map(|addr| unsafe { zun_gc::Obj::from_addr(addr as *mut u64) })
                                .filter(|obj| heap.array_len(*obj).is_some());
                            match child {
                                Some(child) => flatten(heap, child, depth - 1, out),
                                None => {
                                    heap.array_push(out, element);
                                }
                            }
                        }
                    }
                    let out = heap.alloc_array(0);
                    flatten(heap, source, depth, out);
                    out
                });
                Ok(Value::Object(result))
            }
            "arrayFlatMap" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                let roots = [source, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let produced = self.invoke_callback(callback, callback_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    // Flatten one level: a returned array has its elements spread.
                    zun_gc::with_current_heap_mut(|heap| {
                        let child = boxed
                            .as_object()
                            .map(|addr| unsafe { zun_gc::Obj::from_addr(addr as *mut u64) })
                            .filter(|obj| heap.array_len(*obj).is_some());
                        match child {
                            Some(child) => {
                                let child_len = heap.array_len(child).unwrap_or(0);
                                for inner in 0..child_len {
                                    if let Some(entry) = heap.array_get(child, inner) {
                                        heap.array_push(roots[1], entry);
                                    }
                                }
                            }
                            None => {
                                heap.array_push(roots[1], boxed);
                            }
                        }
                    });
                }
                Ok(Value::Object(roots[1]))
            }
            "arrayGroup" | "arrayGroupToMap" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let to_map = name == "arrayGroupToMap";
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    if to_map {
                        heap.alloc_map()
                    } else {
                        heap.alloc_dynamic()
                    }
                });
                let roots = [source, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let callback_args = vec![
                        Value::Any(element),
                        Value::Any(zun_core::Any::float(index as f64)),
                        Value::Any(zun_core::Any::object(roots[0].addr() as usize)),
                    ];
                    let produced = self.invoke_callback(callback, callback_args)?;
                    let key_text = if to_map {
                        None
                    } else {
                        Some(unbox_arg(&produced)?.display())
                    };
                    let boxed = zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        let existing = match &key_text {
                            Some(key) => heap.dynamic_get(roots[1], key),
                            None => heap.map_get(roots[1], boxed),
                        };
                        let bucket = existing
                            .and_then(|any| any.as_object())
                            .map(|addr| unsafe { zun_gc::Obj::from_addr(addr as *mut u64) });
                        let bucket = match bucket {
                            Some(bucket) => bucket,
                            None => {
                                let bucket = heap.alloc_array(0);
                                let any = zun_core::Any::object(bucket.addr() as usize);
                                match &key_text {
                                    Some(key) => {
                                        heap.dynamic_set(roots[1], key.clone(), any);
                                    }
                                    None => {
                                        heap.map_insert(roots[1], boxed, any);
                                    }
                                }
                                bucket
                            }
                        };
                        heap.array_push(bucket, element);
                    });
                }
                if to_map {
                    Ok(Value::Any(zun_core::Any::object(roots[1].addr() as usize)))
                } else {
                    Ok(Value::Object(roots[1]))
                }
            }
            "arrayKeys" | "arrayValues" | "arrayEntries" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let mode = match name {
                    "arrayKeys" => 1,
                    "arrayValues" => 0,
                    _ => 2,
                };
                let result = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_array_iterator_mode(source, mode)
                })
                .ok_or(RunError::BadToken)?;
                Ok(Value::Object(result))
            }
            "arrayFill" | "arrayCopyWithin" => {
                // `fill(value, start, end)` / `copyWithin(target, start, end)`; an
                // undefined bound takes its default, negatives count from the end.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                self.guard_array(source, crate::integrity::ArrayOp::Add)
                    .map_err(|_| RunError::Native("TypeError: Cannot assign to read only property of object '[object Array]'".into()))?;
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source)).unwrap_or(0);
                let len_f = length as f64;
                let bound = |value: &Value, default: f64| -> f64 {
                    let raw = match value {
                        Value::Int(n, _) => *n as f64,
                        Value::Float(n, _) => *n,
                        Value::Void => return default,
                        Value::Bool(b) => f64::from(u8::from(*b)),
                        Value::Null => 0.0,
                        Value::Str(text) => text.trim().parse::<f64>().unwrap_or(0.0),
                        _ => 0.0,
                    };
                    let integer = if raw.is_nan() { 0.0 } else { raw.trunc() };
                    if integer < 0.0 {
                        (len_f + integer).max(0.0)
                    } else {
                        integer.min(len_f)
                    }
                };
                let a1 = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)?;
                let a2 = unbox_arg(args.get(2).ok_or(RunError::BadToken)?)?;
                let a3 = unbox_arg(args.get(3).ok_or(RunError::BadToken)?)?;
                if name == "arrayFill" {
                    let start = bound(&a2, 0.0) as usize;
                    let end = bound(&a3, len_f) as usize;
                    let boxed = zun_gc::with_current_heap_mut(|heap| a1.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        for index in start..end {
                            heap.array_set(source, index, boxed);
                        }
                    });
                } else {
                    let to = bound(&a1, 0.0) as usize;
                    let from = bound(&a2, 0.0) as usize;
                    let end = bound(&a3, len_f) as usize;
                    let count = end.saturating_sub(from).min(length - to);
                    zun_gc::with_current_heap_mut(|heap| {
                        let copied: Vec<zun_core::Any> = (0..count)
                            .map(|offset| {
                                heap.array_get(source, from + offset)
                                    .unwrap_or_else(zun_core::Any::void)
                            })
                            .collect();
                        for (offset, value) in copied.into_iter().enumerate() {
                            heap.array_set(source, to + offset, value);
                        }
                    });
                }
                Ok(Value::Object(source))
            }
            "methodCallSpread" => {
                // `(recv.method as any)(...args)` on a dynamic receiver: array
                // methods dispatch by name; a dynamic object calls its own function
                // property with itself as `this`.
                let receiver = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let method = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                    Value::Str(text) => text.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let list = args.get(2).ok_or(RunError::BadToken)?.clone();
                let Some(target) = object(&receiver) else {
                    return Err(RunError::Native(
                        "TypeError: Cannot read properties of undefined".into(),
                    ));
                };
                let is_array = zun_gc::with_current_heap(|heap| heap.array_len(target).is_some());
                if is_array {
                    let native = match method.as_str() {
                        "slice" => "arraySliceArgs",
                        "splice" => "arraySpliceArgs",
                        _ => {
                            return Err(RunError::Native(format!(
                                "TypeError: array.{method} is not supported with spread arguments"
                            )))
                        }
                    };
                    return self.call_managed_collection(native, &[Value::Object(target), list]);
                }
                let handle =
                    zun_gc::with_current_heap(|heap| heap.dynamic_get_function(target, &method));
                let Some(handle) = handle else {
                    return Err(RunError::Native(format!(
                        "TypeError: {method} is not a function"
                    )));
                };
                let list_obj = object(&list).ok_or(RunError::BadToken)?;
                let call_values: Vec<Value> = zun_gc::with_current_heap(|heap| {
                    let len = heap.array_len(list_obj).unwrap_or(0);
                    (0..len)
                        .map(|i| {
                            heap.array_get(list_obj, i)
                                .and_then(|any| Value::unbox_any(any, heap))
                                .unwrap_or(Value::Void)
                        })
                        .collect()
                });
                let call_args = self.with_this_arg(handle, target, call_values);
                self.invoke_function_value(handle, call_args)
            }
            _ => Err(RunError::BadToken),
        }
    }
}
