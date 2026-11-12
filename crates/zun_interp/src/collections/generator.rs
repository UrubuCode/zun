//! Generator and symbol-iterator protocol native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_generator(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "generatorIteratorNew" => {
                if let Some(method_rid) = args.first().and_then(|value| match value {
                    Value::Int(value, _) => Some(*value as u32),
                    Value::Float(value, _) => Some(*value as u32),
                    _ => None,
                }) {
                    return self.create_generator(method_rid, &[]);
                }
                // A string (typed or boxed in `any`) iterates by code point.
                let first_concrete = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .unwrap_or(Value::Void)
                    }
                    other => other.clone(),
                };
                if let Value::Str(text) = &first_concrete {
                    let points: Vec<String> = text.chars().map(|point| point.to_string()).collect();
                    let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                    let roots = [array];
                    let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                    for point in points {
                        zun_gc::with_current_heap_mut(|heap| {
                            let id = heap.intern_string(point.as_str());
                            heap.array_push(roots[0], zun_core::Any::string(id));
                        });
                    }
                    return self.protocol_iterator_new(roots[0]);
                }
                let source = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or_else(|| RunError::Native("TypeError: value is not iterable".into()))?;
                // A generator object drives itself through the existing frame
                // resume machinery; anything else (a built-in iterator object, a
                // raw array/set/map, or a user `[Symbol.iterator]` object) routes
                // through the generic iteration protocol.
                if zun_gc::with_current_heap(|heap| heap.generator_state(source).is_some()) {
                    return Ok(Value::Object(source));
                }
                self.protocol_iterator_new(source)
            }
            "generatorIteratorNewArgs" => {
                let method_rid = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    Value::Float(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                let array =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let values = zun_gc::with_current_heap(|heap| {
                    let length = heap.array_len(array)?;
                    (0..length)
                        .map(|index| heap.array_get(array, index))
                        .collect::<Option<Vec<_>>>()
                })
                .ok_or(RunError::BadToken)?;
                let generator = zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_generator(0, method_rid, values)
                });
                self.finish_generator_creation(method_rid, generator)
            }
            "generatorIteratorDone" => {
                let generator =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.generator_state(generator).is_none()) {
                    return self.protocol_done(generator);
                }
                let needs_prefetch = zun_gc::with_current_heap(|heap| {
                    heap.generator_state(generator)
                        .map(|state| !state.done && !state.has_current)
                })
                .ok_or(RunError::BadToken)?;
                if needs_prefetch {
                    let value = self.call_managed_collection(
                        "generatorIteratorNext",
                        &[Value::Object(generator)],
                    )?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or(zun_core::Any::null());
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(state) = heap.generator_state_mut(generator) {
                            if !state.done {
                                state.current = boxed;
                                state.has_current = true;
                            }
                        }
                    });
                }
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| {
                        heap.generator_state(generator).map(|state| state.done)
                    })
                    .ok_or(RunError::BadToken)?,
                ))
            }
            "symbolIteratorCall" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.generator_state(source).is_some()) {
                    return Ok(Value::Object(source));
                }
                let iterator_id = zun_core::WellKnownSymbol::Iterator.id();
                let builtin = zun_gc::with_current_heap(|heap| {
                    heap.array_iterator_done(source).is_some()
                        || heap.set_iterator_done(source).is_some()
                        || heap.map_iterator_done(source).is_some()
                        || heap.array_len(source).is_some()
                        || heap.set_values(source).is_some()
                        || heap.map_entries(source).is_some()
                });
                if builtin {
                    return Ok(Value::Object(self.resolve_iterator(source)?));
                }
                let handle = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_symbol_function(source, iterator_id)
                })
                .or_else(|| {
                    self.class_symbol_methods
                        .get(&(source.type_rid(), iterator_id.get()))
                        .copied()
                })
                .or_else(|| super::iterator_helpers::symbol_function_in_chain(source, iterator_id));
                let Some(handle) = handle else {
                    return Err(RunError::Native("TypeError: value is not iterable".into()));
                };
                let call_args = self.with_this_arg(handle, source, Vec::new());
                self.invoke_function_value(handle, call_args)
            }
            "generatorIteratorClose" => {
                // IteratorClose: call the iterator's `return()` (if any) when a
                // for-of loop exits early. `throwing` marks a throw completion,
                // whose error wins over anything `return()` raises.
                let wrapper =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let throwing = matches!(args.get(1), Some(Value::Bool(true)));
                if zun_gc::with_current_heap(|heap| heap.generator_state(wrapper).is_some()) {
                    return self.close_generator(wrapper, throwing);
                }
                let it = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get(wrapper, "__zun_it")
                        .and_then(|any| any.as_object())
                })
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                let Some(it) = it else {
                    return Ok(Value::Bool(true));
                };
                let is_builtin = zun_gc::with_current_heap(|heap| {
                    heap.array_iterator_done(it).is_some()
                        || heap.set_iterator_done(it).is_some()
                        || heap.map_iterator_done(it).is_some()
                });
                if is_builtin {
                    return Ok(Value::Bool(true));
                }
                if zun_gc::with_current_heap(|heap| heap.generator_state(it).is_some()) {
                    return self.close_generator(it, throwing);
                }
                let mut method =
                    zun_gc::with_current_heap(|heap| heap.dynamic_get_function(it, "return"));
                // An accessor `return` is read exactly once per close.
                if method.is_none()
                    && zun_gc::with_current_heap(|heap| {
                        heap.dynamic_accessor_getter(it, "return").is_some()
                    })
                {
                    if let Value::Function(handle) = self.dynamic_member_get(it, "return")? {
                        method = Some(handle);
                    }
                }
                let Some(method) = method else {
                    // A present, non-nullish, non-callable `return` is a TypeError.
                    let present = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get(it, "return").is_some_and(|any| {
                            !matches!(
                                Value::unbox_any(any, heap),
                                Some(Value::Void) | Some(Value::Null) | None
                            )
                        })
                    });
                    if present && !throwing {
                        return Err(RunError::Native(
                            "TypeError: iterator.return is not a function".into(),
                        ));
                    }
                    return Ok(Value::Bool(true));
                };
                let roots = [it];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let return_args = self.with_this_arg(method, it, Vec::new());
                match self.invoke_function_value(method, return_args) {
                    Ok(result) => {
                        if !throwing && object(&result).is_none() {
                            return Err(RunError::Native(
                                "TypeError: iterator.return() did not return an object".into(),
                            ));
                        }
                        Ok(Value::Bool(true))
                    }
                    Err(RunError::Thrown(_)) | Err(RunError::Native(_)) if throwing => {
                        Ok(Value::Bool(true))
                    }
                    Err(error) => Err(error),
                }
            }
            "generatorIteratorNext" => {
                let generator =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.generator_state(generator).is_none()) {
                    return self.protocol_next(generator);
                }
                let mut state =
                    zun_gc::with_current_heap_mut(|heap| heap.generator_state_take(generator))
                        .ok_or(RunError::BadToken)?;
                if state.done {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.generator_state_set(generator, state)
                    });
                    return Ok(Value::Void);
                }
                if state.has_current {
                    let value =
                        zun_gc::with_current_heap(|heap| Value::unbox_any(state.current, heap))
                            .ok_or(RunError::BadToken)?;
                    state.has_current = false;
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.generator_state_set(generator, state)
                    });
                    return Ok(value);
                }
                let outcome = if let Some(saved) = state.frame.take() {
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
                    self.resume_suspended(frame)
                } else {
                    let initial_args = zun_gc::with_current_heap(|heap| {
                        state
                            .args
                            .iter()
                            .map(|value| gen_unbox(*value, heap))
                            .collect::<Option<Vec<_>>>()
                    })
                    .ok_or(RunError::BadToken)?;
                    self.dispatch_outcome(state.method_rid, initial_args, None)
                };
                let result = match outcome {
                    ExecutionOutcome::Suspended(frame) => {
                        let value = self.yielded.take().ok_or(RunError::Malformed(
                            "generator suspended without yielding a value".into(),
                        ))?;
                        state.current = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                            .ok_or(RunError::BadToken)?;
                        let locals = zun_gc::with_current_heap_mut(|heap| {
                            frame
                                .locals
                                .iter()
                                .map(|value| gen_box(value, heap))
                                .collect::<Option<Vec<_>>>()
                        })
                        .ok_or(RunError::BadToken)?;
                        let eval_stack = zun_gc::with_current_heap_mut(|heap| {
                            frame
                                .stack
                                .iter()
                                .map(|value| gen_box(value, heap))
                                .collect::<Option<Vec<_>>>()
                        })
                        .ok_or(RunError::BadToken)?;
                        state.frame = Some(zun_gc::GeneratorFrame {
                            method_rid: frame.method_rid,
                            pc: frame.pc,
                            locals,
                            eval_stack,
                        });
                        value
                    }
                    ExecutionOutcome::Complete(value) => {
                        state.done = true;
                        state.current = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                            .ok_or(RunError::BadToken)?;
                        value
                    }
                    ExecutionOutcome::Thrown(value)
                        if state.has_pending_return
                            && crate::unwind::is_generator_return(&value) =>
                    {
                        state.done = true;
                        state.has_pending_return = false;
                        state.current = state.pending_return;
                        zun_gc::with_current_heap(|heap| Value::unbox_any(state.current, heap))
                            .ok_or(RunError::BadToken)?
                    }
                    ExecutionOutcome::Thrown(value) => {
                        state.done = true;
                        state.current = zun_core::Any::void();
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.generator_state_set(generator, state)
                        });
                        return Err(RunError::Thrown(value));
                    }
                    ExecutionOutcome::Fault(error) => return Err(error),
                };
                zun_gc::with_current_heap_mut(|heap| heap.generator_state_set(generator, state));
                Ok(result)
            }
            _ => Err(RunError::BadToken),
        }
    }
}
