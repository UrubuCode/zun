//! Iterator protocol, generator, dynamic member-get, and function-invocation helpers.
//!
//! Methods moved verbatim out of the collection dispatch module.

use super::*;

impl<'a> Interp<'a> {
    /// ECMAScript `GetIterator(source)`: resolve the iterator object that drives
    /// a for-of / spread / collection-constructor over `source`.
    ///
    /// A built-in iterator object drives itself; a raw array/set/map is wrapped
    /// in the matching built-in iterator; a user value implementing
    /// `[Symbol.iterator]` (an object literal's symbol-function slot or a typed
    /// class's computed-symbol method) has it invoked to produce the iterator.
    pub(crate) fn resolve_iterator(
        &mut self,
        source: zun_gc::Obj,
    ) -> Result<zun_gc::Obj, RunError> {
        let iterator_id = zun_core::WellKnownSymbol::Iterator.id();
        let is_builtin_iter = zun_gc::with_current_heap(|heap| {
            heap.array_iterator_done(source).is_some()
                || heap.set_iterator_done(source).is_some()
                || heap.map_iterator_done(source).is_some()
        });
        if is_builtin_iter
            || zun_gc::with_current_heap(|heap| heap.generator_state(source).is_some())
        {
            return Ok(source);
        }
        if zun_gc::with_current_heap(|heap| {
            heap.array_len(source).is_some()
                && heap
                    .dynamic_get_symbol_function(source, iterator_id)
                    .is_none()
        }) {
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
        let handle =
            zun_gc::with_current_heap(|heap| heap.dynamic_get_symbol_function(source, iterator_id))
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
        object(&result).ok_or_else(|| {
            RunError::Native("TypeError: [Symbol.iterator]() did not return an object".into())
        })
    }

    /// Advance a resolved iterator object once, returning `(value, done)`. A
    /// built-in iterator advances through its native cursor; a user iterator has
    /// its `next()` method called and the `{ value, done }` result read back.
    pub(crate) fn protocol_step(&mut self, it: zun_gc::Obj) -> Result<(Value, bool), RunError> {
        if zun_gc::with_current_heap(|heap| heap.generator_state(it).is_some()) {
            let finished =
                self.call_managed_collection("generatorIteratorDone", &[Value::Object(it)])?;
            if matches!(finished, Value::Bool(true)) {
                return Ok((Value::Void, true));
            }
            let value =
                self.call_managed_collection("generatorIteratorNext", &[Value::Object(it)])?;
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
            return Err(RunError::Native(
                "TypeError: iterator has no next() method".into(),
            ));
        };
        let roots = [it];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let next_args = self.with_this_arg(next, it, Vec::new());
        let result = self.invoke_function_value(next, next_args)?;
        let Some(result_obj) = object(&result) else {
            return Err(RunError::Native(
                "TypeError: iterator.next() did not return an object".into(),
            ));
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
    pub(crate) fn protocol_iterator_new(&mut self, source: zun_gc::Obj) -> Result<Value, RunError> {
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
    pub(crate) fn close_generator(
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
            saved
                .locals
                .iter()
                .map(|value| gen_unbox(*value, heap))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let stack = zun_gc::with_current_heap(|heap| {
            saved
                .eval_stack
                .iter()
                .map(|value| gen_unbox(*value, heap))
                .collect::<Option<Vec<_>>>()
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
    pub(crate) fn protocol_done(&mut self, wrapper: zun_gc::Obj) -> Result<Value, RunError> {
        let roots = [wrapper];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let has = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(roots[0], "__zun_has")
                .and_then(|any| any.as_bool())
        })
        .unwrap_or(false);
        let done_cached = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(roots[0], "__zun_done")
                .and_then(|any| any.as_bool())
        })
        .unwrap_or(false);
        if has || done_cached {
            return Ok(Value::Bool(done_cached));
        }
        let it = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(roots[0], "__zun_it")
                .and_then(|any| any.as_object())
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
    pub(crate) fn protocol_next(&mut self, wrapper: zun_gc::Obj) -> Result<Value, RunError> {
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
    pub(crate) fn make_iter_result(&mut self, value: Value, done: bool) -> Value {
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
    pub(crate) fn generator_return(
        &mut self,
        generator: zun_gc::Obj,
        value: Value,
    ) -> Result<Value, RunError> {
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
            saved
                .locals
                .iter()
                .map(|v| gen_unbox(*v, heap))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let stack = zun_gc::with_current_heap(|heap| {
            saved
                .eval_stack
                .iter()
                .map(|v| gen_unbox(*v, heap))
                .collect::<Option<Vec<_>>>()
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
                    frame
                        .locals
                        .iter()
                        .map(|v| gen_box(v, heap))
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or(RunError::BadToken)?;
                let eval_stack = zun_gc::with_current_heap_mut(|heap| {
                    frame
                        .stack
                        .iter()
                        .map(|v| gen_box(v, heap))
                        .collect::<Option<Vec<_>>>()
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
                crate::unwind::GENERATOR_SENT
                    .with(|sent| *sent.borrow_mut() = args.first().map(|_| first.clone()));
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
                    crate::unwind::GENERATOR_THROW_PENDING
                        .with(|slot| *slot.borrow_mut() = Some(first));
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
    pub(crate) fn builtin_iterator_result(
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
            let finished =
                self.call_managed_collection("generatorIteratorDone", &[Value::Object(it)])?;
            if matches!(finished, Value::Bool(true)) {
                // The first completion reports the generator's return value
                // (what `yield*` evaluates to); later ones report undefined.
                let returned = zun_gc::with_current_heap_mut(|heap| {
                    heap.generator_state_mut(it)
                        .map(|state| std::mem::replace(&mut state.current, zun_core::Any::void()))
                });
                let value = returned
                    .and_then(|any| zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)))
                    .unwrap_or(Value::Void);
                (value, true)
            } else {
                (
                    self.call_managed_collection("generatorIteratorNext", &[Value::Object(it)])?,
                    false,
                )
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
        Ok(Some(Value::Any(zun_core::Any::object(
            result.addr() as usize
        ))))
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
            object(&self.protocol_iterator_new(source)?).ok_or(RunError::BadToken)?
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
            let class_proto = if hop == 0 {
                self.instance_class_prototype(receiver)
            } else {
                None
            };
            let Some(next) = class_proto.or_else(|| {
                zun_gc::with_current_heap(|heap| {
                    heap.dynamic_prototype(current)
                        .and_then(|any| any.as_object())
                        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                })
            }) else {
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
                    heap.dynamic_get_function(current, name)
                        .map(Value::Function)
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
            let name_any = Value::Str(class_name.into())
                .box_any(heap)
                .unwrap_or_else(zun_core::Any::null);
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
            (!heap.is_dynamic_object(obj) || heap.is_instance_bag(obj))
                && heap.array_len(obj).is_none()
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
            let name_any = Value::Str(class_name.into())
                .box_any(heap)
                .unwrap_or_else(zun_core::Any::null);
            heap.dynamic_set(record, "name", name_any);
            heap.dynamic_set(
                record,
                "__classDesc",
                zun_core::Any::class_descriptor(token),
            );
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
            let size = zun_gc::with_current_heap(|heap| {
                heap.map_len(object).or_else(|| heap.set_len(object))
            });
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
        Ok(self
            .prototype_chain_get(object, name)?
            .unwrap_or(Value::Void))
    }

    /// Arguments for invoking an object-literal accessor/method closure: a
    /// closure that uses `this` (marked `@this`) takes the receiver first.
    /// `proxy.key` read through an `any` receiver: the handler's `get` trap
    /// (called as `(target, key, receiver)` with the handler as `this`), or the
    /// target's own property when there is none.
    pub(crate) fn proxy_dynamic_get(
        &mut self,
        proxy: zun_gc::Obj,
        key: &str,
    ) -> Result<Value, RunError> {
        let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy)) else {
            return Err(RunError::BadToken);
        };
        let trap = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "get"));
        let result = match trap {
            Some(callback) => {
                let handler =
                    zun_gc::with_current_heap(|heap| heap.proxy_handler(proxy)).unwrap_or(target);
                let args = vec![
                    Value::Object(target),
                    Value::Str(key.into()),
                    Value::Object(proxy),
                ];
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
            other => zun_gc::with_current_heap_mut(|heap| other.box_any(heap))
                .map(Value::Any)
                .unwrap_or(Value::Void),
        })
    }

    /// `proxy.key = value` through an `any` receiver: the `set` trap
    /// (`(target, key, value, receiver)`) or a plain store on the target.
    pub(crate) fn proxy_dynamic_set(
        &mut self,
        proxy: zun_gc::Obj,
        key: &str,
        value: Value,
    ) -> Result<(), RunError> {
        let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy)) else {
            return Err(RunError::BadToken);
        };
        let trap = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "set"));
        match trap {
            Some(callback) => {
                let handler =
                    zun_gc::with_current_heap(|heap| heap.proxy_handler(proxy)).unwrap_or(target);
                let boxed = match &value {
                    Value::Function(_) | Value::Any(_) => value.clone(),
                    other => zun_gc::with_current_heap_mut(|heap| other.box_any(heap))
                        .map(Value::Any)
                        .unwrap_or(Value::Void),
                };
                let args = vec![
                    Value::Object(target),
                    Value::Str(key.into()),
                    boxed,
                    Value::Object(proxy),
                ];
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
    pub(crate) fn function_meta(
        &self,
        handle: zun_core::FunctionHandle,
        name: &str,
    ) -> Option<Value> {
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
        let method = self
            .module
            .tables
            .method_defs
            .get((rid as usize).wrapping_sub(1))?;
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
}
