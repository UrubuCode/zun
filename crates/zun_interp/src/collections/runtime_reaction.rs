//! Promise reactions, then/finally, and combinator machinery.
//!
//! Methods moved verbatim out of the collection dispatch module.

use super::*;

impl<'a> Interp<'a> {
    /// Attach a reaction to `source` and bridge its control onto the scheduler.
    /// The reaction waits on `source`'s scheduler promise; when that settles
    /// (now or later), the scheduler enqueues this continuation and the
    /// turn-drain runs [`Self::run_reaction`]. An already-settled source yields
    /// `ReadyNow`, which *enqueues* the reaction rather than running it inline —
    /// the async-not-sync guarantee.
    pub(crate) fn attach_reaction(
        &mut self,
        source: zun_gc::Obj,
        target: zun_gc::Obj,
        kind: ReactionKind,
    ) -> ContinuationId {
        let source_pid = self.bridge_pid(source);
        // Bridge the target too (for its side effect: it may reconcile an
        // already-settled target and is the identity a later settle wakes).
        self.bridge_pid(target);
        let cid = ContinuationId(self.async_state.next_cont);
        self.async_state.next_cont += 1;
        self.async_state.reactions.insert(
            cid,
            Reaction {
                source: Value::Object(source),
                target: Value::Object(target),
                kind,
            },
        );
        let state = &mut self.async_state;
        state
            .scheduler
            .park(&mut state.promises, Continuation::new(cid, source_pid));
        cid
    }

    /// Run one scheduled reaction (a microtask): read the source's settlement
    /// from the heap, run the matching callback (or pass the settlement
    /// through), and settle the reaction's target through the scheduler so its
    /// own reactions/frames wake in turn.
    pub(crate) fn run_reaction(&mut self, cid: ContinuationId) -> Result<(), RunError> {
        let Some(reaction) = self.async_state.reactions.remove(&cid) else {
            return Ok(());
        };
        let source = object(&reaction.source).ok_or(RunError::BadToken)?;
        let target = object(&reaction.target).ok_or(RunError::BadToken)?;
        // The callbacks below re-enter managed code and can collect; the source and the target
        // are read back through guards after every such call.
        zun_gc::root!(source = source);
        zun_gc::root!(target = target);
        match reaction.kind {
            ReactionKind::Then {
                on_fulfilled,
                on_rejected,
            } => match self.promise_settlement_of(source.get())? {
                PromiseSettlement::Resolved(value) => match as_function(&on_fulfilled) {
                    Some(callback) => {
                        self.run_then_callback(callback, Value::Any(value), target)?
                    }
                    None => {
                        self.settle_promise_resolve(target.get(), value);
                    }
                },
                PromiseSettlement::Rejected(error, reason) => match as_function(&on_rejected) {
                    Some(callback) => {
                        // Deliver a first-class reason value (an object such as
                        // an `AggregateError`) as-is; otherwise hand the
                        // callback the interned string message.
                        let arg = match reason {
                            Some(reason) => reason,
                            None => zun_gc::with_current_heap_mut(|heap| {
                                zun_core::Any::string(
                                    heap.intern_string(std::rc::Rc::from(error.as_str())),
                                )
                            }),
                        };
                        self.run_then_callback(callback, Value::Any(arg), target)?;
                    }
                    None => match reason {
                        Some(reason) => {
                            self.settle_promise_reject_value(target.get(), reason, error);
                        }
                        None => {
                            self.settle_promise_reject(target.get(), error);
                        }
                    },
                },
                // A reaction only runs once its source has settled, so a pending
                // read here is a spurious wake; leave the target pending.
                PromiseSettlement::Pending => {}
            },
            ReactionKind::Finally { on_finally } => {
                let settlement = self.promise_settlement_of(source.get())?;
                let cleanup_error = match as_function(&on_finally) {
                    Some(callback) => match self.invoke_callback(callback, vec![]) {
                        Ok(result) => {
                            // A cleanup that returns a Promise or thenable delays the
                            // pass-through until that value settles.
                            let waits = object(&unbox_arg(&result).unwrap_or(Value::Void))
                                .filter(|obj| {
                                    zun_gc::with_current_heap(|heap| {
                                        heap.promise_state(*obj).is_some()
                                            || heap.dynamic_get_function(*obj, "then").is_some()
                                            || (!heap.is_dynamic_object(*obj)
                                                && self.instance_method_rid(*obj, "then").is_some())
                                    })
                                });
                            if let Some(obj) = waits {
                                let wait =
                                    zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                                // Defensive: `settle_target_with` only allocates, and
                                // allocating never collects by itself.
                                zun_gc::root!(wait = wait);
                                zun_gc::root!(waited = obj);
                                self.settle_target_with(wait.get(), Value::Object(waited.get()));
                                self.attach_reaction(
                                    wait.get(),
                                    target.get(),
                                    ReactionKind::FinallyPass {
                                        original: Value::Object(source.get()),
                                    },
                                );
                                return Ok(());
                            }
                            None
                        }
                        Err(RunError::Thrown(value)) => Some(value),
                        Err(other) => return Err(other),
                    },
                    None => None,
                };
                if let Some(error) = cleanup_error {
                    self.reject_with_thrown(target.get(), &error);
                } else {
                    match settlement {
                        PromiseSettlement::Resolved(value) => {
                            self.settle_promise_resolve(target.get(), value);
                        }
                        PromiseSettlement::Rejected(error, Some(reason)) => {
                            self.settle_promise_reject_value(target.get(), reason, error);
                        }
                        PromiseSettlement::Rejected(error, None) => {
                            self.settle_promise_reject(target.get(), error);
                        }
                        PromiseSettlement::Pending => {}
                    }
                }
            }
            ReactionKind::FinallyPass { original } => {
                let waited = self.promise_settlement_of(source.get())?;
                if let PromiseSettlement::Rejected(error, reason) = waited {
                    match reason {
                        Some(reason) => {
                            self.settle_promise_reject_value(target.get(), reason, error);
                        }
                        None => {
                            self.settle_promise_reject(target.get(), error);
                        }
                    }
                } else if let Some(original) = object(&original) {
                    match self.promise_settlement_of(original)? {
                        PromiseSettlement::Resolved(value) => {
                            self.settle_promise_resolve(target.get(), value);
                        }
                        PromiseSettlement::Rejected(error, Some(reason)) => {
                            self.settle_promise_reject_value(target.get(), reason, error);
                        }
                        PromiseSettlement::Rejected(error, None) => {
                            self.settle_promise_reject(target.get(), error);
                        }
                        PromiseSettlement::Pending => {}
                    }
                }
            }
            ReactionKind::Combinator { aggregate, index } => {
                let settlement = self.promise_settlement_of(source.get())?;
                self.combinator_input_settled(aggregate, index, settlement);
            }
            ReactionKind::Microtask { callback } => {
                if let Some(callback) = as_function(&Some(callback)) {
                    match self.invoke_callback(callback, Vec::new()) {
                        Ok(_) | Err(RunError::Thrown(_)) => {}
                        Err(other) => return Err(other),
                    }
                }
            }
            ReactionKind::AsyncStep { generator } => {
                let generator = object(&generator).ok_or(RunError::BadToken)?;
                let input = match crate::settled_await_value(source.get()) {
                    Ok(Some(value)) => Ok(value),
                    Ok(None) => return Ok(()),
                    Err(RunError::Thrown(thrown)) => Err(thrown),
                    Err(other) => return Err(other),
                };
                self.async_step(generator, target.get(), input)?;
            }
            ReactionKind::ThenableJob { thenable, then } => {
                if matches!(then, Value::Void) {
                    // Native Promise adoption: pass its settlement through.
                    if let Some(source) = object(&thenable) {
                        self.attach_reaction(
                            source,
                            target.get(),
                            ReactionKind::Then {
                                on_fulfilled: None,
                                on_rejected: None,
                            },
                        );
                    }
                } else if let Value::Int(rid, _) = then {
                    let resolve = self.make_promise_settler(target.get(), SETTLE_RESOLVE);
                    let reject = self.make_promise_settler(target.get(), SETTLE_REJECT);
                    match self.call(
                        rid as u32,
                        vec![thenable, Value::Function(resolve), Value::Function(reject)],
                        None,
                    ) {
                        Ok(_) => {}
                        Err(RunError::Thrown(thrown)) => {
                            let _ = self.invoke_promise_settler(
                                target.get(),
                                SETTLE_REJECT,
                                vec![thrown],
                            );
                        }
                        Err(_) => {
                            self.settle_promise_reject(target.get(), "thenable.then failed".into());
                        }
                    }
                } else if let Some(then) = as_function(&Some(then)) {
                    let resolve = self.make_promise_settler(target.get(), SETTLE_RESOLVE);
                    let reject = self.make_promise_settler(target.get(), SETTLE_REJECT);
                    let then_args = vec![Value::Function(resolve), Value::Function(reject)];
                    let then_args = match object(&thenable) {
                        Some(receiver) => self.with_this_arg(then, receiver, then_args),
                        None => then_args,
                    };
                    match self.invoke_function_value(then, then_args) {
                        Ok(_) => {}
                        Err(RunError::Thrown(thrown)) => {
                            let _ = self.invoke_promise_settler(
                                target.get(),
                                SETTLE_REJECT,
                                vec![thrown],
                            );
                        }
                        Err(_) => {
                            self.settle_promise_reject(target.get(), "thenable.then failed".into());
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Invoke a `then`/`catch` callback and settle the reaction's target from
    /// its result (adopting a returned Promise) or reject it on a throw.
    /// `target` is re-read after the re-entry, which may have relocated it.
    pub(crate) fn run_then_callback(
        &mut self,
        callback: zun_core::FunctionHandle,
        arg: Value,
        target: std::pin::Pin<&zun_gc::Rooted<zun_gc::Obj>>,
    ) -> Result<(), RunError> {
        match self.invoke_callback(callback, vec![arg]) {
            Ok(result) => {
                self.settle_target_with(target.get(), result);
                Ok(())
            }
            Err(RunError::Thrown(value)) => {
                self.reject_with_thrown(target.get(), &value);
                Ok(())
            }
            Err(other) => Err(other),
        }
    }

    /// Reject `target` with a thrown value, keeping an object reason
    /// first-class (a plain string keeps the string surface).
    pub(crate) fn reject_with_thrown(&mut self, target: zun_gc::Obj, value: &Value) {
        match value {
            Value::Str(message) => {
                self.settle_promise_reject(target, message.to_string());
            }
            other => {
                let reason = zun_gc::with_current_heap_mut(|heap| other.clone().box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                let message = value_to_error_string(other);
                self.settle_promise_reject_value(target, reason, message);
            }
        }
    }

    /// `promise.then(onFulfilled?, onRejected?)` / `catch`: create a NEW promise
    /// and attach a reaction to the source. When the source settles (now or
    /// later), the reaction runs the matching callback as a microtask and
    /// settles the new promise. A `then` on a pending source now works — the
    /// behaviour the old eager-at-settle path could not provide.
    pub(crate) fn promise_then(
        &mut self,
        source: zun_gc::Obj,
        on_fulfilled: Option<zun_core::FunctionHandle>,
        on_rejected: Option<zun_core::FunctionHandle>,
    ) -> Result<Value, RunError> {
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        // Keep-alive: `attach_reaction` allocates while the source and the target are still read.
        zun_gc::root!(source = source);
        zun_gc::root!(target = target);
        self.attach_reaction(
            source.get(),
            target.get(),
            ReactionKind::Then {
                on_fulfilled: on_fulfilled.map(Value::Function),
                on_rejected: on_rejected.map(Value::Function),
            },
        );
        Ok(Value::Object(target.get()))
    }

    /// `promise.finally(onFinally)`: create a NEW promise and attach a reaction
    /// that runs the cleanup on settlement and passes the source settlement
    /// through, unless the cleanup itself throws.
    pub(crate) fn promise_finally(
        &mut self,
        source: zun_gc::Obj,
        on_finally: Option<zun_core::FunctionHandle>,
    ) -> Result<Value, RunError> {
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        // Keep-alive: `attach_reaction` allocates while the source and the target are still read.
        zun_gc::root!(source = source);
        zun_gc::root!(target = target);
        self.attach_reaction(
            source.get(),
            target.get(),
            ReactionKind::Finally {
                on_finally: on_finally.map(Value::Function),
            },
        );
        Ok(Value::Object(target.get()))
    }

    /// Normalize a combinator input to a Promise object: an existing Promise is
    /// used as-is, any other value is wrapped in an already-resolved Promise
    /// (`Promise.resolve` semantics) so every input drives a reaction.
    pub(crate) fn as_input_promise(&mut self, value: &Value) -> zun_gc::Obj {
        if let Some(obj) = object(value) {
            if zun_gc::with_current_heap(|heap| heap.promise_state(obj)).is_some() {
                return obj;
            }
        }
        // A thenable object is adopted (its `then` runs as a microtask job).
        if let Some(obj) = object(value) {
            let thenable = zun_gc::with_current_heap(|heap| {
                heap.dynamic_get_function(obj, "then").is_some()
                    || (!heap.is_dynamic_object(obj)
                        && self.instance_method_rid(obj, "then").is_some())
            });
            if thenable {
                let promise = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
                // Defensive: `settle_target_with` only allocates and attaches a reaction, and
                // allocating never collects by itself, so the promise cannot move here yet.
                zun_gc::root!(promise = promise);
                self.settle_target_with(promise.get(), Value::Object(obj));
                return promise.get();
            }
        }
        let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        zun_gc::with_current_heap_mut(|heap| {
            let promise = heap.alloc_promise();
            heap.promise_resolve(promise, boxed);
            promise
        })
    }

    /// `Promise.all/allSettled/race/any`: create a NEW promise and an aggregate,
    /// then attach a reaction to each input promise. The aggregate is settled
    /// per spec as the inputs settle through the scheduler, so pending inputs
    /// are handled (the eager path could only read already-settled inputs).
    pub(crate) fn promise_combinator(
        &mut self,
        name: &str,
        array: zun_gc::Obj,
    ) -> Result<Value, RunError> {
        let elements = self.read_array_values(array)?;
        let kind = match name {
            "promiseAll" => CombinatorKind::All,
            "promiseAllSettled" => CombinatorKind::AllSettled,
            "promiseRace" => CombinatorKind::Race,
            "promiseAny" => CombinatorKind::Any,
            _ => return Err(RunError::BadToken),
        };
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        // `all`/`allSettled` collect an ordered result array; `any` collects the
        // per-input rejection reasons for its `AggregateError`; `race` keeps no
        // array (first settlement wins).
        let holder = match kind {
            CombinatorKind::All | CombinatorKind::AllSettled | CombinatorKind::Any => {
                Value::Object(zun_gc::with_current_heap_mut(|heap| {
                    heap.alloc_array(elements.len())
                }))
            }
            CombinatorKind::Race => Value::Void,
        };
        let aggregate = self.async_state.aggregates.len();
        self.async_state.aggregates.push(Aggregate {
            kind,
            target: Value::Object(target),
            holder,
            remaining: elements.len(),
            settled: false,
        });
        // An empty input settles the aggregate immediately per spec.
        if elements.is_empty() {
            self.conclude_empty_combinator(aggregate);
            return Ok(Value::Object(target));
        }
        for (index, element) in elements.iter().enumerate() {
            let source = self.as_input_promise(element);
            self.attach_reaction(
                source,
                target,
                ReactionKind::Combinator { aggregate, index },
            );
        }
        Ok(Value::Object(target))
    }

    /// Settle an empty-input combinator: `all`/`allSettled` resolve to an empty
    /// array, `race` stays pending forever (spec), `any` rejects (no fulfilment).
    pub(crate) fn conclude_empty_combinator(&mut self, aggregate: usize) {
        let (kind, target, holder) = {
            let agg = &self.async_state.aggregates[aggregate];
            (agg.kind, agg.target.clone(), agg.holder.clone())
        };
        let target = object(&target).expect("aggregate target is an object");
        match kind {
            CombinatorKind::All | CombinatorKind::AllSettled => {
                let array = object(&holder).expect("all/allSettled holder is an array");
                let boxed = zun_core::Any::object(array.addr() as usize);
                self.settle_promise_resolve(target, boxed);
            }
            CombinatorKind::Race => {}
            CombinatorKind::Any => {
                let errors = object(&holder).expect("any holder is an array");
                let reason = self.build_aggregate_error(errors, "All promises were rejected");
                self.settle_promise_reject_value(
                    target,
                    reason,
                    "All promises were rejected".into(),
                );
            }
        }
        self.async_state.aggregates[aggregate].settled = true;
    }

    /// Build an `AggregateError`-shaped dynamic object carrying `errors` (the
    /// collected rejection reasons), `message`, and `name`. A dynamic object
    /// rather than the typed `AggregateError` class: the class is not
    /// guaranteed to be present in the running module (it ships in
    /// `builtin/error.zun`, compiled in only when a program imports it), so the
    /// combinator cannot depend on it. The shape matches the class surface, so
    /// `reason.errors` / `reason.message` / `reason.name` read the same.
    pub(crate) fn build_aggregate_error(
        &mut self,
        errors: zun_gc::Obj,
        message: &str,
    ) -> zun_core::Any {
        // Keep-alive: the aggregate object, its name and its message are allocated while the
        // errors array is still read.
        zun_gc::root!(errors = errors);
        zun_gc::with_current_heap_mut(|heap| {
            let object = heap.alloc_dynamic();
            let name = Value::Str(zun_core::ZStr::from("AggregateError"))
                .box_any(heap)
                .unwrap_or_else(zun_core::Any::null);
            heap.dynamic_set(object, "name", name);
            let message = Value::Str(zun_core::ZStr::from(message))
                .box_any(heap)
                .unwrap_or_else(zun_core::Any::null);
            heap.dynamic_set(object, "message", message);
            heap.dynamic_set(
                object,
                "errors",
                zun_core::Any::object(errors.get().addr() as usize),
            );
            zun_core::Any::object(object.addr() as usize)
        })
    }

    /// Fold one settled input into its combinator aggregate and settle the
    /// aggregate target when the per-spec condition is met.
    pub(crate) fn combinator_input_settled(
        &mut self,
        aggregate: usize,
        index: usize,
        settlement: PromiseSettlement,
    ) {
        if self.async_state.aggregates[aggregate].settled {
            return;
        }
        let kind = self.async_state.aggregates[aggregate].kind;
        match kind {
            CombinatorKind::All => match settlement {
                PromiseSettlement::Resolved(value) => {
                    let holder =
                        object(&self.async_state.aggregates[aggregate].holder).unwrap();
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(holder, index, value));
                    let remaining = {
                        let agg = &mut self.async_state.aggregates[aggregate];
                        agg.remaining -= 1;
                        agg.remaining
                    };
                    if remaining == 0 {
                        let (target, holder) = {
                            let agg = &self.async_state.aggregates[aggregate];
                            (
                                object(&agg.target).unwrap(),
                                object(&agg.holder).unwrap(),
                            )
                        };
                        let boxed = zun_core::Any::object(holder.addr() as usize);
                        self.settle_promise_resolve(target, boxed);
                        self.async_state.aggregates[aggregate].settled = true;
                    }
                }
                PromiseSettlement::Rejected(error, reason) => {
                    let target =
                        object(&self.async_state.aggregates[aggregate].target).unwrap();
                    match reason {
                        Some(reason) => {
                            self.settle_promise_reject_value(target, reason, error);
                        }
                        None => {
                            self.settle_promise_reject(target, error);
                        }
                    }
                    self.async_state.aggregates[aggregate].settled = true;
                }
                PromiseSettlement::Pending => {}
            },
            CombinatorKind::AllSettled => {
                let descriptor = match settlement {
                    PromiseSettlement::Resolved(value) => zun_gc::with_current_heap_mut(|heap| {
                        let object = heap.alloc_dynamic();
                        let status = Value::Str(zun_core::ZStr::from("fulfilled"))
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        heap.dynamic_set(object, "status", status);
                        heap.dynamic_set(object, "value", value);
                        zun_core::Any::object(object.addr() as usize)
                    }),
                    PromiseSettlement::Rejected(error, reason) => {
                        zun_gc::with_current_heap_mut(|heap| {
                            let object = heap.alloc_dynamic();
                            let status = Value::Str(zun_core::ZStr::from("rejected"))
                                .box_any(heap)
                                .unwrap_or_else(zun_core::Any::null);
                            heap.dynamic_set(object, "status", status);
                            // Carry the first-class reason when present, else the
                            // string message.
                            let reason = reason.unwrap_or_else(|| {
                                Value::Str(zun_core::ZStr::from(error.as_str()))
                                    .box_any(heap)
                                    .unwrap_or_else(zun_core::Any::null)
                            });
                            heap.dynamic_set(object, "reason", reason);
                            zun_core::Any::object(object.addr() as usize)
                        })
                    }
                    PromiseSettlement::Pending => return,
                };
                let holder = object(&self.async_state.aggregates[aggregate].holder).unwrap();
                zun_gc::with_current_heap_mut(|heap| heap.array_set(holder, index, descriptor));
                let remaining = {
                    let agg = &mut self.async_state.aggregates[aggregate];
                    agg.remaining -= 1;
                    agg.remaining
                };
                if remaining == 0 {
                    let (target, holder) = {
                        let agg = &self.async_state.aggregates[aggregate];
                        (
                            object(&agg.target).unwrap(),
                            object(&agg.holder).unwrap(),
                        )
                    };
                    let boxed = zun_core::Any::object(holder.addr() as usize);
                    self.settle_promise_resolve(target, boxed);
                    self.async_state.aggregates[aggregate].settled = true;
                }
            }
            CombinatorKind::Race => {
                let target = object(&self.async_state.aggregates[aggregate].target).unwrap();
                match settlement {
                    PromiseSettlement::Resolved(value) => {
                        self.settle_promise_resolve(target, value);
                    }
                    PromiseSettlement::Rejected(error, Some(reason)) => {
                        self.settle_promise_reject_value(target, reason, error);
                    }
                    PromiseSettlement::Rejected(error, None) => {
                        self.settle_promise_reject(target, error);
                    }
                    PromiseSettlement::Pending => return,
                }
                self.async_state.aggregates[aggregate].settled = true;
            }
            CombinatorKind::Any => match settlement {
                PromiseSettlement::Resolved(value) => {
                    let target =
                        object(&self.async_state.aggregates[aggregate].target).unwrap();
                    self.settle_promise_resolve(target, value);
                    self.async_state.aggregates[aggregate].settled = true;
                }
                PromiseSettlement::Rejected(error, reason) => {
                    // Collect this input's rejection reason into the ordered
                    // errors array (a first-class reason as-is, else the string
                    // message boxed).
                    let holder =
                        object(&self.async_state.aggregates[aggregate].holder).unwrap();
                    let reason = reason.unwrap_or_else(|| {
                        zun_gc::with_current_heap_mut(|heap| {
                            Value::Str(zun_core::ZStr::from(error.as_str()))
                                .box_any(heap)
                                .unwrap_or_else(zun_core::Any::null)
                        })
                    });
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(holder, index, reason));
                    let remaining = {
                        let agg = &mut self.async_state.aggregates[aggregate];
                        agg.remaining -= 1;
                        agg.remaining
                    };
                    if remaining == 0 {
                        let (target, holder) = {
                            let agg = &self.async_state.aggregates[aggregate];
                            (
                                object(&agg.target).unwrap(),
                                object(&agg.holder).unwrap(),
                            )
                        };
                        // All inputs rejected: reject with an `AggregateError`
                        // carrying the collected reasons (per spec).
                        let aggregate_error =
                            self.build_aggregate_error(holder, "All promises were rejected");
                        self.settle_promise_reject_value(
                            target,
                            aggregate_error,
                            "All promises were rejected".into(),
                        );
                        self.async_state.aggregates[aggregate].settled = true;
                    }
                }
                PromiseSettlement::Pending => {}
            },
        }
    }
}
