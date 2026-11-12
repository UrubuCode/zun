//! Promise and timer native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_promise(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "promiseNew" => {
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                Ok(Value::Object(promise))
            }
            "promiseWithResolvers" => {
                // `Promise.withResolvers()` → `{ promise, resolve, reject }`.
                // `promise` is an ordinary dynamic property; `resolve`/`reject`
                // are stored in the dynamic-FUNCTION slot (function values do
                // not box into `Any`), reached from source via a dynamic method
                // call (`r.resolve(v)`). Each is a native "settler" callable: a
                // bound-function handle over a record naming the promise and the
                // settle mode (see `settler_of` / the settler interception in
                // `invoke_function_value` and `CallIndirect`).
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                let result = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                let roots = [promise, result];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let resolve = self.make_promise_settler(roots[0], SETTLE_RESOLVE);
                let reject = self.make_promise_settler(roots[0], SETTLE_REJECT);
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set(
                        roots[1],
                        "promise",
                        zun_core::Any::object(roots[0].addr() as usize),
                    );
                    heap.dynamic_set_function(roots[1], "resolve", resolve);
                    heap.dynamic_set_function(roots[1], "reject", reject);
                });
                Ok(Value::Object(roots[1]))
            }
            "promiseNewExecutor" => {
                // `new Promise(executor)`: build a pending promise plus its
                // resolve/reject settlers (the exact `withResolvers` mechanism —
                // see `make_promise_settler`/`settler_of`), then synchronously
                // invoke the executor with them. An executor throw rejects the
                // promise with the thrown value and is swallowed (ECMAScript).
                let executor = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                let roots = [promise];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let resolve = self.make_promise_settler(roots[0], SETTLE_RESOLVE);
                let reject = self.make_promise_settler(roots[0], SETTLE_REJECT);
                match self.invoke_function_value(
                    executor,
                    vec![Value::Function(resolve), Value::Function(reject)],
                ) {
                    Ok(_) => {}
                    Err(RunError::Thrown(value)) => {
                        // Carry an object reason first-class; keep a string
                        // reason's message surface — the same split the reject
                        // settler uses (see `invoke_promise_settler`).
                        match &value {
                            Value::Str(message) => {
                                self.settle_promise_reject(roots[0], message.to_string());
                            }
                            other => {
                                let reason = zun_gc::with_current_heap_mut(|heap| {
                                    other.clone().box_any(heap)
                                })
                                .unwrap_or_else(zun_core::Any::null);
                                let message = value_to_error_string(other);
                                self.settle_promise_reject_value(roots[0], reason, message);
                            }
                        }
                    }
                    Err(other) => return Err(other),
                }
                Ok(Value::Object(roots[0]))
            }
            "promiseResolve" => {
                let promise =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // Settle through the scheduler directly: the heap cell holds the
                // value, and bridged reactions/frames wake via the scheduler.
                Ok(Value::Bool(self.settle_promise_resolve(promise, value)))
            }
            "promiseReject" => {
                let promise =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let error = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(error) => error.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Bool(self.settle_promise_reject(promise, error)))
            }
            "promiseState" => {
                let promise =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let state = zun_gc::with_current_heap(|heap| heap.promise_state(promise))
                    .ok_or(RunError::BadToken)?;
                let state = match state {
                    zun_core::ManagedPromiseState::Pending => 0,
                    zun_core::ManagedPromiseState::Resolved => 1,
                    zun_core::ManagedPromiseState::Rejected => 2,
                };
                Ok(Value::Int(state, zun_il::NumKind::U8))
            }
            "promiseResult" => {
                let promise =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let value = zun_gc::with_current_heap(|heap| heap.promise_result(promise))
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Any(value))
            }
            "promiseIsReady" => {
                let promise =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(
                    zun_gc::with_current_heap(|heap| heap.promise_is_ready(promise))
                        .ok_or(RunError::BadToken)?,
                ))
            }
            "promiseThen" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let on_fulfilled = callback_arg(args.get(1));
                let on_rejected = callback_arg(args.get(2));
                self.promise_then(source, on_fulfilled, on_rejected)
            }
            "promiseCatch" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let on_rejected = callback_arg(args.get(1));
                self.promise_then(source, None, on_rejected)
            }
            "promiseFinally" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let on_finally = callback_arg(args.get(1));
                self.promise_finally(source, on_finally)
            }
            "promiseResolveStatic" => {
                let value = args.first().ok_or(RunError::BadToken)?;
                // A value that is already a Promise is returned unchanged.
                if let Some(existing) = object(value) {
                    if zun_gc::with_current_heap(|heap| heap.promise_state(existing)).is_some() {
                        return Ok(Value::Object(existing));
                    }
                }
                // A thenable object is adopted (its `then` runs as a job).
                if let Some(obj) = object(value) {
                    let thenable = zun_gc::with_current_heap(|heap| {
                        (heap.is_dynamic_object(obj) && heap.array_len(obj).is_none())
                            || (!heap.is_dynamic_object(obj)
                                && self.instance_method_rid(obj, "then").is_some())
                    });
                    if thenable {
                        let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                        let roots = [promise];
                        let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                        self.settle_target_with(roots[0], Value::Object(obj));
                        return Ok(Value::Object(roots[0]));
                    }
                }
                let boxed = any(value).ok_or(RunError::BadToken)?;
                let promise = zun_gc::with_current_heap_mut(|heap| {
                    let promise = heap.alloc_promise();
                    heap.promise_resolve(promise, boxed);
                    promise
                });
                Ok(Value::Object(promise))
            }
            "promiseRejectStatic" => {
                let value = args.first().ok_or(RunError::BadToken)?;
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                // Settle through the scheduler so a detached rejection (nothing
                // awaiting or `.catch`-ing it) is tracked for turn-end
                // unhandled-rejection reporting. An object reason (e.g. an
                // `Error`) is carried first-class — the same split as the
                // executor `reject(..)` path — so `await`/`catch (e: Error)`
                // sees the object, not a stringified "[object]".
                match value {
                    Value::Str(message) => {
                        self.settle_promise_reject(promise, message.to_string());
                    }
                    other => {
                        let reason =
                            zun_gc::with_current_heap_mut(|heap| other.clone().box_any(heap))
                                .unwrap_or_else(zun_core::Any::null);
                        let message = value_to_error_string(other);
                        self.settle_promise_reject_value(promise, reason, message);
                    }
                }
                Ok(Value::Object(promise))
            }
            "promiseAll" | "promiseAllSettled" | "promiseRace" | "promiseAny" => {
                let array =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                self.promise_combinator(name, array)
            }
            "setTimeout" | "setInterval" | "setImmediate" => {
                // The callback is required and must be a function value; a
                // missing/non-function callback is a no-op that still returns a
                // (dead) handle, matching the lenient host surface.
                let callback = match args.first() {
                    Some(value @ Value::Function(_)) => value.clone(),
                    _ => return Ok(Value::Float(f64::NAN, zun_il::NumKind::F64)),
                };
                // `setImmediate` is a 0-delay macrotask; the others read the
                // (checker-defaulted) delay argument.
                let delay = if name == "setImmediate" {
                    0
                } else {
                    match args.get(1) {
                        Some(Value::Float(ms, _)) => *ms as i64,
                        Some(Value::Int(ms, _)) => *ms,
                        _ => 0,
                    }
                };
                let delay = delay.max(0);
                let period = if name == "setInterval" {
                    Some(delay.max(0))
                } else {
                    None
                };
                let handle = self.timer_arm(callback, delay, period);
                Ok(Value::Float(handle as f64, zun_il::NumKind::F64))
            }
            "setTimeoutArgs" | "setIntervalArgs" => {
                let callback = match args.first() {
                    Some(value @ Value::Function(_)) => value.clone(),
                    _ => return Ok(Value::Float(f64::NAN, zun_il::NumKind::F64)),
                };
                let delay = match args.get(1) {
                    Some(Value::Float(ms, _)) => *ms as i64,
                    Some(Value::Int(ms, _)) => *ms,
                    _ => 0,
                }
                .max(0);
                let period = if name == "setIntervalArgs" {
                    Some(delay)
                } else {
                    None
                };
                let extra = match args.get(2).and_then(object) {
                    Some(array) => self.read_array_values(array)?,
                    None => Vec::new(),
                };
                let handle = self.timer_arm(callback, delay, period);
                let task = TaskId(handle);
                if let Some(entry) = self
                    .async_state
                    .timers
                    .entries
                    .iter_mut()
                    .find(|entry| entry.task == task)
                {
                    entry.args = extra;
                }
                Ok(Value::Float(handle as f64, zun_il::NumKind::F64))
            }
            "clearTimeout" | "clearInterval" => {
                let handle = match args.first() {
                    Some(Value::Float(handle, _)) => *handle as i64,
                    Some(Value::Int(handle, _)) => *handle,
                    // A nullish/absent handle clears nothing (JS is lenient).
                    _ => return Ok(Value::Void),
                };
                if handle >= 0 {
                    self.timer_clear(handle as u32);
                }
                Ok(Value::Void)
            }
            "queueMicrotask" => {
                let callback = match args.first() {
                    Some(Value::Function(handle)) => *handle,
                    _ => return Ok(Value::Void),
                };
                self.queue_microtask(callback)?;
                Ok(Value::Void)
            }
            _ => Err(RunError::BadToken),
        }
    }
}
