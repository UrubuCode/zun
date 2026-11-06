//! Array slice/reverse/indexOf/rest/toSorted/with/concat bulk native methods (hot).
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    #[inline]
    pub(super) fn col_array_mutate(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "arraySliceArgs" => {
                // `slice(...args)` with a runtime argument list.
                let list =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let mut forwarded = vec![args[0].clone()];
                for index in 0..2 {
                    let element = zun_gc::with_current_heap(|heap| {
                        heap.array_get(list, index).and_then(|any| Value::unbox_any(any, heap))
                    });
                    forwarded.push(match element {
                        Some(Value::Str(text)) => Value::Float(
                            text.trim().parse::<f64>().unwrap_or(f64::NAN),
                            zun_il::NumKind::F64,
                        ),
                        Some(Value::Bool(b)) => Value::Float(f64::from(u8::from(b)), zun_il::NumKind::F64),
                        Some(Value::Null) => Value::Float(0.0, zun_il::NumKind::F64),
                        Some(other) => other,
                        None => Value::Void,
                    });
                }
                self.call_managed_collection("arraySliceRange", &forwarded)
            }
            "arraySpliceArgs" => {
                // `splice(...args)` with a runtime argument list: JS clamping rules.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let list =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let removed = zun_gc::with_current_heap_mut(|heap| {
                    let to_integer = |any: zun_core::Any| -> f64 {
                        let n = match Value::unbox_any(any, heap) {
                            Some(Value::Float(n, _)) => n,
                            Some(Value::Int(n, _)) => n as f64,
                            Some(Value::Bool(b)) => f64::from(u8::from(b)),
                            Some(Value::Null) => 0.0,
                            Some(Value::Str(text)) => text.trim().parse::<f64>().unwrap_or(f64::NAN),
                            _ => f64::NAN,
                        };
                        if n.is_nan() { 0.0 } else { n.trunc() }
                    };
                    let length = heap.array_len(source).unwrap_or(0);
                    let count = heap.array_len(list).unwrap_or(0);
                    let len_f = length as f64;
                    let (start, delete_count) = if count == 0 {
                        (0usize, 0usize)
                    } else {
                        let raw = to_integer(heap.array_get(list, 0).unwrap_or_else(zun_core::Any::void));
                        let start = if raw < 0.0 { (len_f + raw).max(0.0) } else { raw.min(len_f) } as usize;
                        let delete_count = if count == 1 {
                            length - start
                        } else {
                            let raw = to_integer(heap.array_get(list, 1).unwrap_or_else(zun_core::Any::void));
                            raw.max(0.0).min((length - start) as f64) as usize
                        };
                        (start, delete_count)
                    };
                    let result = heap.alloc_array(delete_count);
                    for offset in 0..delete_count {
                        if let Some(element) = heap.array_remove(source, start) {
                            heap.array_set(result, offset, element);
                        }
                    }
                    for (offset, index) in (2..count).enumerate() {
                        if let Some(element) = heap.array_get(list, index) {
                            heap.array_insert(source, start + offset, element);
                        }
                    }
                    result
                });
                Ok(Value::Object(removed))
            }
            "arraySpliceInsert" => {
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
                let items =
                    object(args.get(3).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let removed = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let start = start.min(length);
                    let delete_count = delete_count.min(length - start);
                    let result = heap.alloc_array(delete_count);
                    for offset in 0..delete_count {
                        if let Some(element) = heap.array_remove(source, start) {
                            heap.array_set(result, offset, element);
                        }
                    }
                    let insert_count = heap.array_len(items).unwrap_or(0);
                    for offset in 0..insert_count {
                        if let Some(element) = heap.array_get(items, offset) {
                            heap.array_insert(source, start + offset, element);
                        }
                    }
                    result
                });
                Ok(Value::Object(removed))
            }
            "arraySlice" => {
                // `source.slice(start..)` into a fresh managed Array. Backs the
                // `...rest` element of array destructuring.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let start = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let start = start.min(length);
                    let result = heap.alloc_array(length - start);
                    for offset in 0..(length - start) {
                        let element = heap
                            .array_get(source, start + offset)
                            .map(|a| if a.is_hole() { zun_core::Any::void() } else { a }).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, offset, element);
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arraySliceRange" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // Object arguments run ToPrimitive (valueOf / toString), start first.
                let mut args = args.to_vec();
                for slot in 1..args.len().min(3) {
                    if let Value::Object(_) | Value::Any(_) = args[slot] {
                        let raw = unbox_arg(&args[slot])?;
                        if matches!(raw, Value::Object(_)) {
                            let number = self.to_number_value(raw)?;
                            args[slot] = Value::Float(number, zun_il::NumKind::F64);
                        }
                    }
                }
                let args = &args[..];
                let relative = |value: Option<&Value>, default: f64, length: f64| -> f64 {
                    let raw = match value {
                        Some(Value::Float(number, _)) => *number,
                        Some(Value::Int(number, _)) => *number as f64,
                        _ => default,
                    };
                    let integer = if raw.is_nan() { 0.0 } else { raw.trunc() };
                    if integer < 0.0 {
                        (length + integer).max(0.0)
                    } else {
                        integer.min(length)
                    }
                };
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let start = relative(args.get(1), 0.0, length as f64) as usize;
                    let end = relative(args.get(2), length as f64, length as f64) as usize;
                    let count = end.saturating_sub(start);
                    let result = heap.alloc_array(count);
                    for offset in 0..count {
                        let element = heap
                            .array_get(source, start + offset)
                            .unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, offset, element);
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arrayReverse" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    for index in 0..length / 2 {
                        let left = heap.array_get(source, index).unwrap_or_else(zun_core::Any::void);
                        let right = heap
                            .array_get(source, length - 1 - index)
                            .unwrap_or_else(zun_core::Any::void);
                        heap.array_set(source, index, right);
                        heap.array_set(source, length - 1 - index, left);
                    }
                });
                Ok(Value::Object(source))
            }
            "arrayIndexOf" | "arrayLastIndexOf" | "arrayIncludes" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let needle = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)?;
                let from = match args.get(2) {
                    Some(Value::Float(number, _)) => Some(*number),
                    Some(Value::Int(number, _)) => Some(*number as f64),
                    _ => None,
                };
                let found = zun_gc::with_current_heap(|heap| {
                    let length = heap.array_len(source).unwrap_or(0) as f64;
                    let resolve = |from: f64| {
                        let integer = if from.is_nan() { 0.0 } else { from.trunc() };
                        if integer < 0.0 { (length + integer).max(0.0) } else { integer }
                    };
                    let element = |index: usize| {
                        let any = heap.array_get(source, index).unwrap_or_else(zun_core::Any::void);
                        Value::unbox_any(any, heap).unwrap_or(Value::Void)
                    };
                    let zero_aware = name == "arrayIncludes";
                    // `indexOf`/`lastIndexOf` skip holes; `includes` reads them as undefined.
                    let skip_holes = !zero_aware;
                    let is_hole = |index: usize| heap.array_get(source, index).is_some_and(|any| any.is_hole());
                    let equal = |left: &Value, right: &Value| strict_equals(left, right, zero_aware);
                    if name == "arrayLastIndexOf" {
                        // `lastIndexOf` searches backwards from `from` (default
                        // length - 1); a negative `from` counts from the end.
                        let start = match from {
                            None => length - 1.0,
                            Some(from) => {
                                let integer = if from.is_nan() { 0.0 } else { from.trunc() };
                                if integer < 0.0 { length + integer } else { integer.min(length - 1.0) }
                            }
                        };
                        if start < 0.0 {
                            return None;
                        }
                        let mut index = start as usize;
                        loop {
                            if !(skip_holes && is_hole(index)) && equal(&element(index), &needle) {
                                return Some(index);
                            }
                            if index == 0 {
                                return None;
                            }
                            index -= 1;
                        }
                    }
                    let start = resolve(from.unwrap_or(0.0));
                    let mut index = start as usize;
                    while (index as f64) < length {
                        if !(skip_holes && is_hole(index)) && equal(&element(index), &needle) {
                            return Some(index);
                        }
                        index += 1;
                    }
                    None
                });
                Ok(match name {
                    "arrayIncludes" => Value::Bool(found.is_some()),
                    _ => Value::Float(
                        found.map(|index| index as f64).unwrap_or(-1.0),
                        zun_il::NumKind::F64,
                    ),
                })
            }
            "anyIsUndefined" => Ok(Value::Bool(match args.first().ok_or(RunError::BadToken)? {
                Value::Void => true,
                Value::Any(any) => any.tag() == zun_core::AnyTag::Void,
                _ => false,
            })),
            "coalesceUndefined" => {
                // Destructuring default: `fallback` when `value` is undefined.
                let value = args.first().ok_or(RunError::BadToken)?.clone();
                let is_undefined = match &value {
                    Value::Void => true,
                    Value::Any(any) => any.tag() == zun_core::AnyTag::Void,
                    _ => false,
                };
                if is_undefined {
                    Ok(args.get(1).ok_or(RunError::BadToken)?.clone())
                } else {
                    Ok(value)
                }
            }
            "objectRest" => {
                // Copy every own-enumerable string key of `source` except the
                // names listed in `excludedKeys` into a fresh dynamic object.
                let source = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let excluded =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let excluded_count = heap.array_len(excluded).unwrap_or(0);
                    let mut excluded_names = Vec::with_capacity(excluded_count);
                    let mut excluded_symbols = Vec::new();
                    for index in 0..excluded_count {
                        if let Some(any) = heap.array_get(excluded, index) {
                            if let Some(symbol) = any.as_symbol() {
                                excluded_symbols.push(symbol);
                                continue;
                            }
                            // Static keys are strings; a computed key may also be a number.
                            if let Some(key) = stringify_any(any, heap) {
                                excluded_names.push(key);
                            }
                        }
                    }
                    let result = heap.alloc_dynamic();
                    for key in heap.dynamic_keys(source).unwrap_or_default() {
                        if !excluded_names.iter().any(|name| name == &key) {
                            if let Some(value) = heap.dynamic_get(source, &key) {
                                heap.dynamic_set(result, key, value);
                            }
                        }
                    }
                    // Own enumerable symbol-keyed properties are rest members too.
                    for symbol in heap.dynamic_symbol_keys(source) {
                        if excluded_symbols.contains(&symbol) || !heap.dynamic_is_symbol_enumerable(source, symbol) {
                            continue;
                        }
                        if let Some(value) = heap.dynamic_get_symbol(source, symbol) {
                            heap.dynamic_set_symbol(result, symbol, value);
                        }
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arrayReduceNoInit" | "arrayReduceRightNoInit" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let forward = name == "arrayReduceNoInit";
                if length == 0 {
                    return match self.new_error_object("TypeError", "Reduce of empty array with no initial value") {
                        Some(error) => Err(RunError::Thrown(Value::Object(error))),
                        None => Err(RunError::BadToken),
                    };
                }
                // Seed from the first (forward) or last (reverse) present element;
                // holes are skipped, and an all-hole array has no seed.
                let seed_step = (0..length).find(|&step| {
                    let index = if forward { step } else { length - 1 - step };
                    zun_gc::with_current_heap(|heap| heap.array_get(source, index))
                        .is_some_and(|any| !any.is_hole())
                });
                let Some(seed_step) = seed_step else {
                    return match self.new_error_object("TypeError", "Reduce of empty array with no initial value") {
                        Some(error) => Err(RunError::Thrown(Value::Object(error))),
                        None => Err(RunError::BadToken),
                    };
                };
                let seed_index = if forward { seed_step } else { length - 1 - seed_step };
                let accumulator = zun_gc::with_current_heap_mut(|heap| {
                    let holder = heap.alloc_array(1);
                    let seed = heap.array_get(source, seed_index).unwrap_or_else(zun_core::Any::void);
                    heap.array_set(holder, 0, seed);
                    holder
                });
                let roots = [source, accumulator];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for step in (seed_step + 1)..length {
                    let index = if forward { step } else { length - 1 - step };
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
                    let boxed = zun_gc::with_current_heap_mut(|heap| next.box_any(heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(roots[1], 0, boxed));
                }
                let result = zun_gc::with_current_heap(|heap| heap.array_get(roots[1], 0))
                    .ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap(|heap| Value::unbox_any(result, heap))
                    .ok_or(RunError::BadToken)
            }
            "arrayToSorted" | "arrayToSortedBy" => {
                // ES2023 `toSorted(cmp?)`: a sorted COPY; the receiver is
                // untouched. Clone into a fresh array, then run the same
                // insertion sort `sort` uses (comparator re-enters the interp).
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let comparator = if name == "arrayToSortedBy" {
                    match args.get(1).ok_or(RunError::BadToken)? {
                        Value::Function(handle) => Some(*handle),
                        _ => return Err(RunError::BadToken),
                    }
                } else {
                    None
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let copy = zun_gc::with_current_heap_mut(|heap| {
                    let copy = heap.alloc_array(length);
                    for index in 0..length {
                        let element =
                            heap.array_get(source, index).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(copy, index, element);
                    }
                    copy
                });
                let roots = [copy];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for i in 1..length {
                    let mut j = i;
                    while j > 0 {
                        let left = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], j - 1))
                            .ok_or(RunError::BadToken)?;
                        let right = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], j))
                            .ok_or(RunError::BadToken)?;
                        let ordered_before = match comparator {
                            Some(handle) => {
                                let outcome = self.invoke_callback(
                                    handle,
                                    vec![Value::Any(left), Value::Any(right)],
                                )?;
                                !(zun_gc::with_current_heap(|heap| to_number(&outcome, heap)) > 0.0)
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
            "arrayToReversed" => {
                // `toReversed()`: a reversed COPY; the receiver is untouched.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let result = heap.alloc_array(length);
                    for index in 0..length {
                        let element = heap
                            .array_get(source, length - 1 - index)
                            .map(|a| if a.is_hole() { zun_core::Any::void() } else { a }).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, index, element);
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arrayWith" => {
                // `with(index, value)`: a COPY with `index` replaced. A negative
                // index counts from the end; an out-of-range index throws a
                // RangeError-equivalent.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let raw = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value,
                    Value::Float(value, _) => *value as i64,
                    _ => return Err(RunError::BadToken),
                };
                let value = any(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let resolved = if raw < 0 { raw + length as i64 } else { raw };
                if resolved < 0 || resolved >= length as i64 {
                    return match self.new_error_object("RangeError", "Invalid index : out of range") {
                        Some(object) => Err(RunError::Thrown(Value::Object(object))),
                        None => Err(RunError::Native("RangeError: Invalid index : out of range".into())),
                    };
                }
                let target = resolved as usize;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let result = heap.alloc_array(length);
                    for index in 0..length {
                        let element = if index == target {
                            value
                        } else {
                            heap.array_get(source, index).map(|a| if a.is_hole() { zun_core::Any::void() } else { a }).unwrap_or_else(zun_core::Any::void)
                        };
                        heap.array_set(result, index, element);
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arrayToSpliced" => {
                // `toSpliced(start, deleteCount, ...items)`: a COPY with the
                // splice applied; the receiver is untouched.
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
                let items =
                    object(args.get(3).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let length = heap.array_len(source).unwrap_or(0);
                    let start = start.min(length);
                    let delete_count = delete_count.min(length - start);
                    let insert_count = heap.array_len(items).unwrap_or(0);
                    let result = heap.alloc_array(length - delete_count + insert_count);
                    let mut out = 0usize;
                    for index in 0..start {
                        let element =
                            heap.array_get(source, index).map(|a| if a.is_hole() { zun_core::Any::void() } else { a }).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, out, element);
                        out += 1;
                    }
                    for offset in 0..insert_count {
                        let element =
                            heap.array_get(items, offset).map(|a| if a.is_hole() { zun_core::Any::void() } else { a }).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, out, element);
                        out += 1;
                    }
                    for index in (start + delete_count)..length {
                        let element =
                            heap.array_get(source, index).map(|a| if a.is_hole() { zun_core::Any::void() } else { a }).unwrap_or_else(zun_core::Any::void);
                        heap.array_set(result, out, element);
                        out += 1;
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            "arrayConcat" => {
                // `concat(...items)`: a FRESH array of the receiver's elements
                // then each item. An item is spread when it is a managed Array
                // (unless its `[Symbol.isConcatSpreadable]` is explicitly
                // false) or a dynamic object whose `[Symbol.isConcatSpreadable]`
                // is true (read via `length` + index keys); otherwise it is
                // appended as a single element.
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let items =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let mut flat: Vec<zun_core::Any> = Vec::new();
                    let source_len = heap.array_len(source).unwrap_or(0);
                    for index in 0..source_len {
                        flat.push(heap.array_get(source, index).unwrap_or_else(zun_core::Any::void));
                    }
                    let item_count = heap.array_len(items).unwrap_or(0);
                    for item_index in 0..item_count {
                        let item = heap.array_get(items, item_index).unwrap_or_else(zun_core::Any::void);
                        let obj = item
                            .as_object()
                            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                        let Some(obj) = obj else {
                            flat.push(item);
                            continue;
                        };
                        let flag = heap
                            .dynamic_get_symbol(obj, zun_core::WellKnownSymbol::IsConcatSpreadable.id())
                            .and_then(|any| match Value::unbox_any(any, heap) {
                                Some(Value::Bool(b)) => Some(b),
                                _ => None,
                            });
                        if let Some(length) = heap.array_len(obj) {
                            if flag == Some(false) {
                                flat.push(item);
                            } else {
                                for index in 0..length {
                                    flat.push(
                                        heap.array_get(obj, index).unwrap_or_else(zun_core::Any::void),
                                    );
                                }
                            }
                        } else if flag == Some(true) && heap.is_dynamic_object(obj) {
                            let length = heap
                                .dynamic_get(obj, "length")
                                .and_then(|any| match Value::unbox_any(any, heap) {
                                    Some(Value::Float(n, _)) if n >= 0.0 => Some(n as usize),
                                    Some(Value::Int(n, _)) if n >= 0 => Some(n as usize),
                                    _ => None,
                                })
                                .unwrap_or(0);
                            for index in 0..length {
                                flat.push(
                                    heap.dynamic_get(obj, &index.to_string())
                                        .unwrap_or_else(zun_core::Any::void),
                                );
                            }
                        } else {
                            flat.push(item);
                        }
                    }
                    let result = heap.alloc_array(flat.len());
                    for (index, value) in flat.into_iter().enumerate() {
                        heap.array_set(result, index, value);
                    }
                    result
                });
                Ok(Value::Object(result))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
