//! Async generator / async function resumption helpers.
//!
//! Methods moved verbatim out of the collection dispatch module.

use super::*;

impl<'a> Interp<'a> {
    /// Settle `target` with a reaction callback's return value, adopting a
    /// returned Promise (chaining, not wrapping). A still-pending returned
    /// Promise is adopted by attaching a pass-through reaction, so `target`
    /// settles exactly when the returned Promise does — the proper async
    /// chaining the old eager path could not express.
    pub(crate) fn settle_target_with(&mut self, target: zun_gc::Obj, value: Value) {
        if let Some(obj) = object(&value) {
            // A promise resolved with itself rejects with a TypeError.
            if obj == target {
                let message = "Chaining cycle detected for promise #<Promise>";
                if let Some(Value::Object(error)) = self.make_error_value("TypeError", message) {
                    let reason = zun_core::Any::object(error.addr() as usize);
                    self.settle_promise_reject_value(
                        target,
                        reason,
                        format!("TypeError: {message}"),
                    );
                }
                return;
            }
            // `state` is consumed only by the intentionally-unreachable match
            // below; silence the dead-code unused_variables warning.
            #[allow(unused_variables)]
            if let Some(state) = zun_gc::with_current_heap(|heap| heap.promise_state(obj)) {
                // ECMAScript NewPromiseResolveThenableJob: adopting a native
                // Promise costs a job (which attaches the pass-through reaction)
                // plus the reaction tick.
                let dummy = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                let roots = [target, obj, dummy];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                zun_gc::with_current_heap_mut(|heap| {
                    heap.promise_resolve(roots[2], zun_core::Any::null())
                });
                self.attach_reaction(
                    roots[2],
                    roots[0],
                    ReactionKind::ThenableJob {
                        thenable: Value::Object(roots[1]),
                        then: Value::Void,
                    },
                );
                return;
                #[allow(unreachable_code)]
                match state {
                    zun_core::ManagedPromiseState::Resolved => {
                        let inner = zun_gc::with_current_heap(|heap| heap.promise_result(obj))
                            .unwrap_or_else(zun_core::Any::null);
                        self.settle_promise_resolve(target, inner);
                    }
                    zun_core::ManagedPromiseState::Rejected => {
                        let error = zun_gc::with_current_heap(|heap| heap.promise_error(obj))
                            .unwrap_or_else(|| "Promise rejected".into());
                        let reason = zun_gc::with_current_heap(|heap| heap.promise_reason(obj));
                        self.notice_promise(obj);
                        match reason {
                            Some(reason) => {
                                self.settle_promise_reject_value(target, reason, error);
                            }
                            None => {
                                self.settle_promise_reject(target, error);
                            }
                        }
                    }
                    // Adopt a pending Promise: a pass-through reaction copies its
                    // settlement to `target` when it settles.
                    zun_core::ManagedPromiseState::Pending => {
                        self.attach_reaction(
                            obj,
                            target,
                            ReactionKind::Then {
                                on_fulfilled: None,
                                on_rejected: None,
                            },
                        );
                    }
                }
                return;
            }
            // Generic thenable: an object with a callable `then` is adopted by
            // calling `then(resolve, reject)` with fresh settlers for `target`
            // (the `new Promise` executor mechanism). Function values do not
            // box into `Any`, so a callable `then` lives in the function slot.
            // The call is scheduled as a microtask job: a pre-resolved dummy
            // promise carries a `ThenableJob` reaction, which the scheduler
            // enqueues (ReadyNow) and the turn-drain runs.
            // A typed class instance with a `then` method is a thenable too; its
            // method rid rides the job as an integer (called with the instance
            // as `this`).
            let typed_then = if zun_gc::with_current_heap(|heap| {
                heap.dynamic_get_function(obj, "then").is_none() && !heap.is_dynamic_object(obj)
            }) {
                self.instance_method_rid(obj, "then")
            } else {
                None
            };
            // A plain object's `then` is read once, through its accessor and
            // prototype chain, and adopted only when callable.
            let dynamic_then = if typed_then.is_none()
                && zun_gc::with_current_heap(|heap| {
                    heap.is_dynamic_object(obj) && heap.array_len(obj).is_none()
                }) {
                match self.dynamic_member_get(obj, "then") {
                    Ok(Value::Function(handle)) => Some(Value::Function(handle)),
                    Ok(Value::Any(any)) => zun_gc::with_current_heap(|heap| {
                        match crate::collections::unbox_function_box(
                            Value::unbox_any(any, heap).unwrap_or(Value::Void),
                            heap,
                        ) {
                            Value::Function(handle) => Some(Value::Function(handle)),
                            _ => None,
                        }
                    }),
                    _ => None,
                }
            } else {
                None
            };
            if let Some(then) = typed_then
                .map(|rid| Value::Int(rid as i64, zun_il::NumKind::U32))
                .or(dynamic_then)
            {
                let dummy = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                let roots = [target, obj, dummy];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                zun_gc::with_current_heap_mut(|heap| {
                    heap.promise_resolve(roots[2], zun_core::Any::null())
                });
                self.attach_reaction(
                    roots[2],
                    roots[0],
                    ReactionKind::ThenableJob {
                        thenable: Value::Object(roots[1]),
                        then,
                    },
                );
                return;
            }
        }
        let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        self.settle_promise_resolve(target, boxed);
    }

    /// `asyncGenResume(gen, mode, value)`: advance an async-generator coroutine
    /// (mode 0 = next, 1 = return, 2 = throw) and answer `[kind, value]` where
    /// kind 0 = the body awaits `value`, 1 = it yielded `value`, 2 = it finished
    /// with `value`. A body exception propagates as a thrown value.
    pub(crate) fn async_gen_resume(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let generator = object(&unbox_arg(args.first().ok_or(RunError::BadToken)?)?)
            .ok_or(RunError::BadToken)?;
        let mode = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
            Value::Int(n, _) => n,
            Value::Float(n, _) => n as i64,
            _ => 0,
        };
        let value = unbox_arg(args.get(2).unwrap_or(&Value::Void))?;
        let roots = [generator];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        let (done, started) = zun_gc::with_current_heap(|heap| {
            heap.generator_state(roots[0])
                .map(|state| (state.done, state.frame.is_some()))
                .unwrap_or((true, false))
        });
        let finish = |kind: i64, value: Value| -> Result<Value, RunError> {
            zun_gc::with_current_heap_mut(|heap| {
                let array = heap.alloc_array(2);
                let kind_any = Value::Float(kind as f64, zun_il::NumKind::F64).box_any(heap)?;
                let value_any = value.box_any(heap)?;
                heap.array_set(array, 0, kind_any);
                heap.array_set(array, 1, value_any);
                Some(Value::Any(zun_core::Any::object(array.addr() as usize)))
            })
            .ok_or(RunError::BadToken)
        };
        match mode {
            1 => {
                if done || !started {
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(state) = heap.generator_state_mut(roots[0]) {
                            state.done = true;
                            state.frame = None;
                        }
                    });
                    return finish(2, value);
                }
                crate::unwind::ASYNC_AWAIT.with(|flag| flag.set(false));
                let result = self.generator_return(roots[0], value)?;
                let flagged = crate::unwind::ASYNC_AWAIT.with(|flag| flag.replace(false));
                let result_obj = object(&unbox_arg(&result)?).ok_or(RunError::BadToken)?;
                let (inner, finished) = zun_gc::with_current_heap(|heap| {
                    let inner = heap
                        .dynamic_get(result_obj, "value")
                        .and_then(|any| Value::unbox_any(any, heap))
                        .unwrap_or(Value::Void);
                    let finished = heap
                        .dynamic_get(result_obj, "done")
                        .and_then(|any| any.as_bool())
                        .unwrap_or(true);
                    (inner, finished)
                });
                return finish(
                    if finished {
                        2
                    } else if flagged {
                        0
                    } else {
                        1
                    },
                    inner,
                );
            }
            2 => {
                if done || !started {
                    zun_gc::with_current_heap_mut(|heap| {
                        if let Some(state) = heap.generator_state_mut(roots[0]) {
                            state.done = true;
                            state.frame = None;
                        }
                    });
                    return Err(RunError::Thrown(value));
                }
                crate::unwind::GENERATOR_THROW_PENDING
                    .with(|slot| *slot.borrow_mut() = Some(value));
            }
            _ => {
                if done {
                    return finish(2, Value::Void);
                }
                crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = Some(value));
            }
        }
        crate::unwind::ASYNC_AWAIT.with(|flag| flag.set(false));
        let result =
            self.call_managed_collection("generatorIteratorNext", &[Value::Object(roots[0])]);
        crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = None);
        crate::unwind::GENERATOR_THROW_PENDING.with(|slot| *slot.borrow_mut() = None);
        let awaited = crate::unwind::ASYNC_AWAIT.with(|flag| flag.replace(false));
        let value = result?;
        let finished = zun_gc::with_current_heap(|heap| {
            heap.generator_state(roots[0])
                .map(|state| state.done)
                .unwrap_or(true)
        });
        finish(
            if finished {
                2
            } else if awaited {
                0
            } else {
                1
            },
            value,
        )
    }

    /// Finish creating a generator object: an `async function*` hands the raw
    /// coroutine to the prelude's `__zunMakeAsyncGen`, which wraps it in an
    /// async-generator object (queueing, promise-returning `next`/`return`/`throw`).
    pub(crate) fn finish_generator_creation(
        &mut self,
        method_rid: u32,
        generator: zun_gc::Obj,
    ) -> Result<Value, RunError> {
        let is_async_gen = self
            .module
            .tables
            .method_defs
            .get((method_rid as usize).wrapping_sub(1))
            .and_then(|method| read_body(&self.module.code, method.body_offset).ok())
            .map(|body| {
                matches!(decode_one(body.code), Ok((first, _)) if first.opcode == Opcode::AsyncGenStart)
            })
            .unwrap_or(false);
        if !is_async_gen {
            return Ok(Value::Object(generator));
        }
        let maker = self
            .free_function_rid("__zunMakeAsyncGen")
            .ok_or_else(|| RunError::Native("async generator support is unavailable".into()))?;
        let roots = [generator];
        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
        self.call(maker, vec![Value::Object(roots[0])], None)
    }

    /// Entry of an `async` function: allocate its result Promise and a
    /// generator-style coroutine over the body, run the body to its first
    /// `await` (synchronously, as the spec requires) and hand the Promise back.
    pub(crate) fn async_start(
        &mut self,
        method_rid: u32,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        let boxed = zun_gc::with_current_heap_mut(|heap| {
            args.iter()
                .map(|value| gen_box(value, heap))
                .collect::<Option<Vec<_>>>()
        })
        .ok_or(RunError::BadToken)?;
        let (promise, generator) = zun_gc::with_current_heap_mut(|heap| {
            let promise = heap.alloc_promise();
            let generator = heap.alloc_generator(0, method_rid, boxed);
            (promise, generator)
        });
        // `async_step` runs the body, which allocates and can evacuate the promise;
        // the result below must be the relocated address.
        zun_gc::root!(promise = promise);
        zun_gc::root!(generator = generator);
        self.async_step(generator.get(), promise.get(), Ok(Value::Void))?;
        // Closures are called through function values, whose results are
        // dynamically typed (`any`); named functions and methods return the
        // Promise object itself.
        let dynamic_result = self
            .module
            .tables
            .method_defs
            .get(method_rid as usize - 1)
            .map(|method| {
                self.module
                    .strings
                    .get(method.name)
                    .starts_with("<closure@")
            })
            .unwrap_or(false);
        if dynamic_result {
            Ok(Value::Any(zun_core::Any::object(
                promise.get().addr() as usize
            )))
        } else {
            Ok(Value::Object(promise.get()))
        }
    }

    /// Advance an async coroutine one step: resume it with `input` (a value, or
    /// an exception raised at the `await`), then either settle `promise` (body
    /// finished or threw) or park the coroutine on the awaited value.
    pub(crate) fn async_step(
        &mut self,
        generator: zun_gc::Obj,
        promise: zun_gc::Obj,
        input: Result<Value, Value>,
    ) -> Result<(), RunError> {
        // `generatorIteratorNext` runs the coroutine body and can evacuate both objects;
        // the state lookup and every settle below go through the relocated addresses.
        zun_gc::root!(generator = generator);
        zun_gc::root!(promise = promise);
        match input {
            Ok(value) => {
                crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = Some(value))
            }
            Err(thrown) => crate::unwind::GENERATOR_THROW_PENDING
                .with(|slot| *slot.borrow_mut() = Some(thrown)),
        }
        crate::unwind::ASYNC_BYPASS.with(|flag| flag.set(true));
        let result = self
            .call_managed_collection("generatorIteratorNext", &[Value::Object(generator.get())]);
        crate::unwind::ASYNC_BYPASS.with(|flag| flag.set(false));
        crate::unwind::GENERATOR_SENT.with(|sent| *sent.borrow_mut() = None);
        crate::unwind::GENERATOR_THROW_PENDING.with(|slot| *slot.borrow_mut() = None);
        let done = zun_gc::with_current_heap(|heap| {
            heap.generator_state(generator.get())
                .map(|state| state.done)
                .unwrap_or(true)
        });
        match result {
            Err(RunError::Thrown(thrown)) => {
                let message = value_to_error_string(&thrown);
                let boxed = zun_gc::with_current_heap_mut(|heap| thrown.box_any(heap));
                match boxed {
                    Some(reason) => {
                        self.settle_promise_reject_value(promise.get(), reason, message);
                    }
                    None => {
                        self.settle_promise_reject(promise.get(), message);
                    }
                }
                Ok(())
            }
            Err(other) => Err(other),
            Ok(value) if done => {
                self.settle_target_with(promise.get(), value);
                Ok(())
            }
            Ok(value) => {
                // `await value`: a Promise is awaited directly; anything else
                // (a plain value or a thenable) is first resolved into a fresh
                // Promise so it takes the same one-tick path.
                let awaited = match object(&value) {
                    Some(obj)
                        if zun_gc::with_current_heap(|heap| heap.promise_state(obj)).is_some() =>
                    {
                        obj
                    }
                    _ => {
                        let fresh = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                        let inner = [fresh];
                        let _inner_scope = unsafe { zun_gc::NativeRootScope::enter(&inner) };
                        self.settle_target_with(inner[0], value);
                        inner[0]
                    }
                };
                let awaited_roots = [awaited];
                let _awaited_scope = unsafe { zun_gc::NativeRootScope::enter(&awaited_roots) };
                self.notice_promise(awaited_roots[0]);
                self.attach_reaction(
                    awaited_roots[0],
                    promise.get(),
                    ReactionKind::AsyncStep {
                        generator: Value::Object(generator.get()),
                    },
                );
                Ok(())
            }
        }
    }
}
