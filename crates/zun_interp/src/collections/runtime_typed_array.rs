//! Typed-array extension methods and callback invocation helpers.
//!
//! Methods moved verbatim out of the collection dispatch module.

use super::*;

impl<'a> Interp<'a> {
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
    pub(crate) fn call_typed_array_ext(
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
                let view =
                    obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let src = obj(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let offset = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as usize,
                    _ => return Err(RunError::BadToken),
                };
                let elements = source_elements(src).ok_or(RunError::BadToken)?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                // Out of range reports `false` rather than throwing a
                // RangeError (the whole set surface here yields a bool).
                if offset
                    .checked_add(elements.len())
                    .is_none_or(|end| end > length)
                {
                    return Ok(Value::Bool(false));
                }
                for (index, value) in elements.iter().enumerate() {
                    if crate::typed_arrays::write_element(view, offset + index, value) != Some(true)
                    {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(true))
            })()),
            "typedArraySlice" => Some((|| {
                let view =
                    obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let (start, end) = (int_arg(args.get(1))?, int_arg(args.get(2))?);
                let element = crate::typed_arrays::element_of(view).ok_or(RunError::BadToken)?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let begin = relative(start, length);
                let finish = relative(end, length).max(begin);
                // Read the source range before allocating the result so the
                // view is never read across the allocation that could move it.
                let mut values = Vec::with_capacity(finish - begin);
                for index in begin..finish {
                    values.push(
                        crate::typed_arrays::read_element(view, index).ok_or(RunError::BadToken)?,
                    );
                }
                let result = crate::typed_arrays::alloc_of(element, values.len())
                    .ok_or(RunError::BadToken)?;
                for (index, value) in values.iter().enumerate() {
                    crate::typed_arrays::write_element(result, index, value);
                }
                Ok(Value::Object(result))
            })()),
            "typedArrayCopyWithin" => Some((|| {
                let view =
                    obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
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
                    values.push(
                        crate::typed_arrays::read_element(view, from + index)
                            .ok_or(RunError::BadToken)?,
                    );
                }
                for (index, value) in values.iter().enumerate() {
                    crate::typed_arrays::write_element(view, dest + index, value);
                }
                Ok(Value::Object(view))
            })()),
            "typedArrayIndexOf" | "typedArrayIncludes" => Some((|| {
                let view =
                    obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // The search value arrives boxed to `Any`; unbox it to the
                // concrete numeric value the elements compare against.
                let needle = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .ok_or(RunError::BadToken)?
                    }
                    other => other.clone(),
                };
                let from = int_arg(args.get(2))?;
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let start = relative(from, length);
                let includes = name == "typedArrayIncludes";
                for index in start..length {
                    let element =
                        crate::typed_arrays::read_element(view, index).ok_or(RunError::BadToken)?;
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
                let view =
                    obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let separator = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(text) => text.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let length = crate::typed_arrays::element_len(view).ok_or(RunError::BadToken)?;
                let mut parts = Vec::with_capacity(length);
                for index in 0..length {
                    let element =
                        crate::typed_arrays::read_element(view, index).ok_or(RunError::BadToken)?;
                    let text = zun_gc::with_current_heap_mut(|heap| {
                        element
                            .box_any(heap)
                            .and_then(|any| stringify_any(any, heap))
                    })
                    .unwrap_or_default();
                    parts.push(text);
                }
                Ok(Value::Str(parts.join(&separator).into()))
            })()),
            "typedArrayForEach"
            | "typedArrayMap"
            | "typedArrayFilter"
            | "typedArrayFind"
            | "typedArrayFindIndex"
            | "typedArraySome"
            | "typedArrayEvery" => Some(self.typed_array_iterating(name, args)),
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
                let buffer =
                    obj(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
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
    pub(crate) fn typed_array_iterating(
        &mut self,
        name: &str,
        args: &[Value],
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
        // Every callback can evacuate the view, the `map` result and the callback's closure
        // environment, so reads go through the guards.
        zun_gc::root!(view = view);
        zun_gc::root!(result = result);
        zun_gc::root!(callback = callback);
        let mut kept: Vec<Value> = Vec::new();
        for index in 0..length {
            let value =
                crate::typed_arrays::read_element(view.get(), index).ok_or(RunError::BadToken)?;
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let callback_args = vec![
                Value::Any(boxed),
                Value::Any(zun_core::Any::float(index as f64)),
                Value::Any(zun_core::Any::object(view.get().addr() as usize)),
            ];
            let produced = self.invoke_callback(callback.get(), callback_args)?;
            match name {
                "typedArrayForEach" => {}
                "typedArrayMap" => {
                    crate::typed_arrays::write_element(result.get(), index, &produced);
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
            "typedArrayMap" => Ok(Value::Object(result.get())),
            "typedArrayFilter" => {
                let result =
                    crate::typed_arrays::alloc_of(element, kept.len()).ok_or(RunError::BadToken)?;
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
    pub(crate) fn typed_array_reduce(
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
                return match self
                    .new_error_object("TypeError", "Reduce of empty array with no initial value")
                {
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
        // Every callback can evacuate the view, the accumulator holder and the callback's
        // closure environment, so reads go through the guards.
        zun_gc::root!(view = view);
        zun_gc::root!(accumulator = accumulator);
        zun_gc::root!(callback = callback);
        for step in start_step..length {
            let index = at(step);
            let element =
                crate::typed_arrays::read_element(view.get(), index).ok_or(RunError::BadToken)?;
            let boxed_element = zun_gc::with_current_heap_mut(|heap| element.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let acc = zun_gc::with_current_heap(|heap| heap.array_get(accumulator.get(), 0))
                .ok_or(RunError::BadToken)?;
            let callback_args = vec![
                Value::Any(acc),
                Value::Any(boxed_element),
                Value::Any(zun_core::Any::float(index as f64)),
                Value::Any(zun_core::Any::object(view.get().addr() as usize)),
            ];
            let next = self.invoke_callback(callback.get(), callback_args)?;
            let boxed = zun_gc::with_current_heap_mut(|heap| next.box_any(heap))
                .ok_or(RunError::BadToken)?;
            zun_gc::with_current_heap_mut(|heap| heap.array_set(accumulator.get(), 0, boxed));
        }
        let result = zun_gc::with_current_heap(|heap| heap.array_get(accumulator.get(), 0))
            .ok_or(RunError::BadToken)?;
        zun_gc::with_current_heap(|heap| Value::unbox_any(result, heap)).ok_or(RunError::BadToken)
    }

    /// `TypedArray.from(source, mapFn)`: the mapped static. Reads the source
    /// into values, runs each through the callback, and stores the result into
    /// a fresh view of the discriminant-named kind.
    pub(crate) fn typed_array_from_mapped(&mut self, args: &[Value]) -> Result<Value, RunError> {
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
        let result =
            crate::typed_arrays::alloc_of(kind, elements.len()).ok_or(RunError::BadToken)?;
        // Every callback can evacuate the result view and the callback's closure environment,
        // so both are read back through guards.
        zun_gc::root!(result = result);
        zun_gc::root!(callback = callback);
        for (index, value) in elements.iter().enumerate() {
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let callback_args = vec![
                Value::Any(boxed),
                Value::Any(zun_core::Any::float(index as f64)),
            ];
            let produced = self.invoke_callback(callback.get(), callback_args)?;
            crate::typed_arrays::write_element(result.get(), index, &produced);
        }
        Ok(Value::Object(result.get()))
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
            heap.dynamic_has(record, "__elemAcc")
                .then(|| heap.dynamic_accessor_getter(record, "v"))
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
        // A built-in method value used as a callback (`xs.map(Math.sqrt)`) runs
        // with an `undefined` receiver.
        if handle.builtin_method().is_some_and(|kind| kind >= 16) {
            return self.invoke_builtin_with_this(handle, Value::Void, call_args);
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
}
