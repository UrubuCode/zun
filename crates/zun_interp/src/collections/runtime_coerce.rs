//! Timers, promise settlement, and numeric/relational coercion helpers.
//!
//! Methods moved verbatim out of the collection dispatch module.

// The `to_number_value` / `to_primitive_default` helpers coerce an operand
// value; the `to_` name describes the coercion they perform, not `self`.
#![allow(clippy::wrong_self_convention)]

use super::*;
use core::cmp::Ordering;

impl<'a> Interp<'a> {
    /// The current virtual time (ms), seeded lazily from the real `nowMillis`
    /// clock on first use. The real clock is only the *base*; the loop advances
    /// this virtual clock by fast-forward between macrotasks rather than
    /// sleeping (see [`Self::fire_one_timer`]).
    pub(crate) fn timer_now(&mut self) -> i64 {
        if !self.async_state.timers.seeded {
            let real = real_now_millis(self.output);
            self.async_state.timers.clock = real;
            self.async_state.timers.seeded = true;
        }
        self.async_state.timers.clock
    }

    /// Arm a timer: allocate a scheduler [`TaskId`] (which doubles as the
    /// numeric handle), compute its due time from the current virtual clock,
    /// and push it onto the timer heap. `period` is `Some` for an interval.
    pub(crate) fn timer_arm(&mut self, callback: Value, delay: i64, period: Option<i64>) -> u32 {
        let now = self.timer_now();
        let task = TaskId(self.async_state.next_task);
        self.async_state.next_task += 1;
        let seq = self.async_state.timers.next_seq;
        self.async_state.timers.next_seq += 1;
        self.async_state.timers.entries.push(TimerEntry {
            task,
            due: now.saturating_add(delay.max(0)),
            seq,
            period,
            callback,
            args: Vec::new(),
        });
        task.0
    }

    /// Clear a timer by handle: cancel it on the scheduler's task queue (which
    /// also stops an interval re-arming) and drop any still-pending entry.
    pub(crate) fn timer_clear(&mut self, handle: u32) {
        let task = TaskId(handle);
        self.async_state.scheduler.cancel_task(task);
        self.async_state
            .timers
            .entries
            .retain(|timer| timer.task != task);
    }

    /// `queueMicrotask(cb)`: enqueue a callback directly on the reaction
    /// (microtask) queue, so it runs before any macrotask on the next drain.
    /// Modelled as a `then` on an already-resolved promise — the same
    /// enqueue-not-run-inline discipline every reaction follows.
    pub(crate) fn queue_microtask(
        &mut self,
        callback: zun_core::FunctionHandle,
    ) -> Result<(), RunError> {
        let promise = zun_gc::with_current_heap_mut(|heap| {
            let promise = heap.alloc_promise();
            heap.promise_resolve(promise, zun_core::Any::null());
            promise
        });
        let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_promise());
        // Keep-alive: `attach_reaction` allocates while both promises are still read.
        zun_gc::root!(promise = promise);
        zun_gc::root!(target = target);
        self.attach_reaction(
            promise.get(),
            target.get(),
            ReactionKind::Microtask {
                callback: Value::Function(callback),
            },
        );
        Ok(())
    }

    /// Index of the earliest-due pending timer (min by `(due, seq)`), or `None`
    /// when the heap is empty. Cleared timers are already purged from the heap.
    pub(crate) fn earliest_timer_index(&self) -> Option<usize> {
        let entries = &self.async_state.timers.entries;
        if entries.is_empty() {
            return None;
        }
        let mut best = 0;
        for index in 1..entries.len() {
            let candidate = &entries[index];
            let current = &entries[best];
            if (candidate.due, candidate.seq) < (current.due, current.seq) {
                best = index;
            }
        }
        Some(best)
    }

    /// Fire the single earliest-due ready timer as one macrotask, returning
    /// whether a timer was fired (`false` when none remain). The earliest timer
    /// is always fired regardless of real wall-clock time: if it is not yet due
    /// the virtual clock fast-forwards to its due time (no real sleeping), which
    /// is what makes a timed program terminate deterministically. A due timer is
    /// scheduled on the scheduler's task tier and consumed through it, so the
    /// scheduler stays the authority on micro-before-macro ordering. An interval
    /// re-arms itself (at `now + period`) unless it was cleared meanwhile.
    pub(crate) fn fire_one_timer(&mut self) -> Result<bool, RunError> {
        let Some(index) = self.earliest_timer_index() else {
            return Ok(false);
        };
        let entry = self.async_state.timers.entries.remove(index);
        if entry.due > self.async_state.timers.clock {
            self.async_state.timers.clock = entry.due;
        }
        self.async_state.scheduler.schedule_task(entry.task);
        let ran = matches!(
            self.async_state.scheduler.next(),
            Some(zun_sched::Ran::Task(task)) if task == entry.task
        );
        if ran {
            if let Value::Function(handle) = &entry.callback {
                // A throw from a timer callback is swallowed (detached-task
                // semantics, as in the microtask drain); a non-throw fault
                // still aborts the turn.
                match self.invoke_callback(*handle, entry.args.clone()) {
                    Ok(_) | Err(RunError::Thrown(_)) => {}
                    Err(other) => return Err(other),
                }
            }
            if let Some(period) = entry.period {
                if !self.async_state.scheduler.is_task_cancelled(entry.task) {
                    let now = self.async_state.timers.clock;
                    let seq = self.async_state.timers.next_seq;
                    self.async_state.timers.next_seq += 1;
                    self.async_state.timers.entries.push(TimerEntry {
                        task: entry.task,
                        due: now.saturating_add(period.max(0)),
                        seq,
                        period: Some(period),
                        callback: entry.callback.clone(),
                        args: entry.args,
                    });
                }
            }
        }
        Ok(true)
    }

    /// Read the settlement of a managed Promise. Errors if the object is not a
    /// Promise cell.
    pub(crate) fn promise_settlement_of(
        &self,
        promise: zun_gc::Obj,
    ) -> Result<PromiseSettlement, RunError> {
        zun_gc::with_current_heap(|heap| match heap.promise_state(promise) {
            Some(zun_core::ManagedPromiseState::Resolved) => Ok(PromiseSettlement::Resolved(
                heap.promise_result(promise)
                    .unwrap_or_else(zun_core::Any::null),
            )),
            Some(zun_core::ManagedPromiseState::Rejected) => Ok(PromiseSettlement::Rejected(
                heap.promise_error(promise)
                    .unwrap_or_else(|| "Promise rejected".into()),
                heap.promise_reason(promise),
            )),
            Some(zun_core::ManagedPromiseState::Pending) => Ok(PromiseSettlement::Pending),
            None => Err(RunError::BadToken),
        })
    }

    /// Resolve a managed Promise through the scheduler: the settled value lives
    /// in the heap cell, and the scheduler is told so every reaction and parked
    /// `await` frame bridged to this promise wakes. Returns whether this call
    /// was the settling one (a second settlement is a no-op).
    pub(crate) fn settle_promise_resolve(
        &mut self,
        promise: zun_gc::Obj,
        value: zun_core::Any,
    ) -> bool {
        let settled = zun_gc::with_current_heap_mut(|heap| heap.promise_resolve(promise, value))
            .unwrap_or(false);
        if settled {
            self.wake_scheduler(promise, Settlement::Fulfilled);
        }
        settled
    }

    /// Reject a managed Promise through the scheduler (see
    /// [`Self::settle_promise_resolve`]). The promise is bridged first so the
    /// scheduler always learns of the rejection — this is what lets a detached
    /// rejection (nothing awaiting/`.catch`-ing it) be tracked as unhandled.
    pub(crate) fn settle_promise_reject(&mut self, promise: zun_gc::Obj, error: String) -> bool {
        self.bridge_pid(promise);
        let settled = zun_gc::with_current_heap_mut(|heap| heap.promise_reject(promise, error))
            .unwrap_or(false);
        if settled {
            self.wake_scheduler(promise, Settlement::Rejected);
        }
        settled
    }

    /// Reject a managed Promise with a first-class reason value (an object such
    /// as an `AggregateError`, or any boxed value) plus its display message.
    /// The value is delivered to `catch`/`await`; the message keeps the string
    /// surface working. Bridged first, exactly like [`Self::settle_promise_reject`].
    pub(crate) fn settle_promise_reject_value(
        &mut self,
        promise: zun_gc::Obj,
        reason: zun_core::Any,
        message: String,
    ) -> bool {
        self.bridge_pid(promise);
        let settled = zun_gc::with_current_heap_mut(|heap| {
            heap.promise_reject_value(promise, reason, message)
        })
        .unwrap_or(false);
        if settled {
            self.wake_scheduler(promise, Settlement::Rejected);
        }
        settled
    }

    /// OrdinaryToPrimitive for an object operand. Consults
    /// `[Symbol.toPrimitive]` (dynamic symbol slot, or the typed-class
    /// symbol-method table for an instance), then the ordered
    /// `valueOf`/`toString` dynamic function slots (hint "string" tries
    /// `toString` first). Returns the first primitive, or `None`.
    /// A typed instance's `valueOf`/`toString` come from `class_named_methods`
    /// (registered by `classSetMethod`) and receive the instance as `this`.
    /// ECMAScript IsLooselyEqual (`==`).
    pub(crate) fn loose_equals(&mut self, left: Value, right: Value) -> Result<bool, RunError> {
        let left = unbox_arg(&left)?;
        let right = unbox_arg(&right)?;
        let nullish = |value: &Value| matches!(value, Value::Void | Value::Null);
        let number = |value: &Value| match value {
            Value::Int(n, _) => Some(*n as f64),
            Value::Float(n, _) => Some(*n),
            _ => None,
        };
        if nullish(&left) || nullish(&right) {
            return Ok(nullish(&left) && nullish(&right));
        }
        if let (Some(a), Some(b)) = (number(&left), number(&right)) {
            return Ok(a == b);
        }
        // BigInt operands (heap objects) compare by mathematical value.
        let big_text = |value: &Value| -> Option<String> {
            match value {
                Value::Object(obj) => {
                    zun_gc::with_current_heap(|heap| heap.bigint_value(*obj).map(|v| v.to_string()))
                }
                _ => None,
            }
        };
        let normalize_digits = |text: &str| -> Option<String> {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Some("0".to_string());
            }
            let (negative, digits) = match trimmed.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
            };
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let stripped = digits.trim_start_matches('0');
            if stripped.is_empty() {
                Some("0".to_string())
            } else if negative {
                Some(format!("-{stripped}"))
            } else {
                Some(stripped.to_string())
            }
        };
        match (big_text(&left), big_text(&right)) {
            (Some(a), Some(b)) => return Ok(a == b),
            (Some(a), None) | (None, Some(a)) => {
                let other = if big_text(&left).is_some() {
                    &right
                } else {
                    &left
                };
                match other {
                    Value::Str(text) => return Ok(normalize_digits(text).is_some_and(|n| n == a)),
                    Value::Int(..) | Value::Float(..) => {
                        let n = number(other).unwrap_or(f64::NAN);
                        return Ok(n.is_finite()
                            && n.fract() == 0.0
                            && (n.abs() < 1e38)
                            && format!("{}", n as i128) == a);
                    }
                    Value::Bool(flag) => return Ok(a == if *flag { "1" } else { "0" }),
                    Value::Object(obj) => {
                        let primitive = self.to_primitive_default(*obj)?;
                        let (l, r) = if big_text(&left).is_some() {
                            (left.clone(), primitive)
                        } else {
                            (primitive, right.clone())
                        };
                        return self.loose_equals(l, r);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        match (&left, &right) {
            (Value::Str(a), Value::Str(b)) => return Ok(a == b),
            (Value::Bool(a), Value::Bool(b)) => return Ok(a == b),
            (Value::Object(a), Value::Object(b)) => return Ok(a.addr() == b.addr()),
            (Value::Function(a), Value::Function(b)) => return Ok(a == b),
            _ => {}
        }
        // Booleans become numbers first.
        if let Value::Bool(flag) = &left {
            let as_number = Value::Float(if *flag { 1.0 } else { 0.0 }, zun_il::NumKind::F64);
            return self.loose_equals(as_number, right);
        }
        if let Value::Bool(flag) = &right {
            let as_number = Value::Float(if *flag { 1.0 } else { 0.0 }, zun_il::NumKind::F64);
            return self.loose_equals(left, as_number);
        }
        // Number against string: the string is converted.
        if let (Some(a), Value::Str(text)) = (number(&left), &right) {
            return Ok(a == js_string_to_number(text));
        }
        if let (Value::Str(text), Some(b)) = (&left, number(&right)) {
            return Ok(js_string_to_number(text) == b);
        }
        // An object against a primitive: ToPrimitive(object), then compare again.
        if let Value::Object(obj) = &left {
            if big_text(&left).is_none() {
                let primitive = self.to_primitive_default(*obj)?;
                return self.loose_equals(primitive, right);
            }
        }
        if let Value::Object(obj) = &right {
            if big_text(&right).is_none() {
                let primitive = self.to_primitive_default(*obj)?;
                return self.loose_equals(left, primitive);
            }
        }
        Ok(false)
    }

    /// ToPrimitive with the default hint: arrays join, objects run their
    /// `valueOf`/`toString` hooks, a bare object is `"[object Object]"`.
    pub(crate) fn to_primitive_default(&mut self, obj: zun_gc::Obj) -> Result<Value, RunError> {
        if let Some(primitive) = self.array_own_primitive(obj, "default")? {
            return Ok(primitive);
        }
        if let Some(text) = array_join_text(obj) {
            return Ok(Value::Str(text.into()));
        }
        if let Some(text) = crate::node_buffer::buffer_text(obj) {
            return Ok(Value::Str(text.into()));
        }
        Ok(match self.ordinary_to_primitive(obj, "default")? {
            Some(primitive) => primitive,
            None => Value::Str("[object Object]".into()),
        })
    }

    /// The primitive an array that took an own `toString`, `valueOf` or `join`
    /// (an expando property, e.g. `arr.toString = fn`) converts to under `hint`,
    /// or `None` when it has no such override and the built-in join applies.
    /// `Array.prototype.toString` calls `join`, so an own `join` counts too. The
    /// one place every array-to-primitive conversion consults for an override.
    pub(crate) fn array_own_primitive(
        &mut self,
        obj: zun_gc::Obj,
        hint: &str,
    ) -> Result<Option<Value>, RunError> {
        let overridable = zun_gc::with_current_heap(|heap| {
            heap.array_len(obj).is_some() && heap.is_dynamic_object(obj)
        });
        if !overridable {
            return Ok(None);
        }
        zun_gc::root!(held = obj);
        if let Some(primitive) = self.ordinary_to_primitive(obj, hint)? {
            return Ok(Some(primitive));
        }
        let own_join =
            zun_gc::with_current_heap(|heap| heap.dynamic_get_function(held.get(), "join"));
        let Some(join) = own_join else {
            // A non-callable own `join` makes `Array.prototype.toString` fall
            // back to `Object.prototype.toString`.
            let shadowed = zun_gc::with_current_heap(|heap| {
                heap.dynamic_has(held.get(), "join") && !heap.dynamic_is_accessor(held.get(), "join")
            });
            // A `join` the array inherits from a class prototype (an `Array`
            // subclass overriding it) is the one `Array.prototype.toString`
            // calls; the ordinary member call resolves and invokes it.
            if !shadowed && Self::native_chain_has(held.get(), "join") {
                let result = self.call_managed_collection(
                    "dynamicCall0",
                    &[Value::Object(held.get()), Value::Str("join".into())],
                )?;
                let result = unbox_arg(&result)?;
                return Ok(
                    (!matches!(result, Value::Object(_) | Value::Function(_))).then_some(result)
                );
            }
            return Ok(shadowed.then(|| Value::Str("[object Array]".into())));
        };
        let call_args = self.with_this_arg(join, held.get(), Vec::new());
        let result = unbox_arg(&self.invoke_function_value(join, call_args)?)?;
        // An object answer is not a primitive; the built-in join applies.
        Ok((!matches!(result, Value::Object(_) | Value::Function(_))).then_some(result))
    }

    /// ECMAScript ToNumber of an (already unboxed) value, running object
    /// coercion hooks (`valueOf` / `toString` / `[Symbol.toPrimitive]`).
    /// Relational-comparison operand preparation (the two topmost stack values):
    /// non-BigInt objects become primitives (hint "number"); when the result is
    /// not string-versus-string, both sides become numbers. Plain numeric pairs
    /// are left untouched.
    /// `Array.prototype.join` over an array object: object elements run their
    /// own ToString (`toString` / `Symbol.toPrimitive`), nested arrays join with
    /// commas, and `null`/`undefined`/holes render empty. `None` for a non-array.
    pub(crate) fn array_join_dyn(
        &mut self,
        obj: zun_gc::Obj,
        separator: &str,
    ) -> Result<Option<String>, RunError> {
        // The arrays being joined, outermost first: an array reachable from
        // itself contributes an empty string instead of recursing forever. The
        // registry owns the slots so a collection inside an element's `toString`
        // keeps them current.
        let stack = zun_gc::RootArray::new(0);
        self.array_join_guarded(obj, separator, &stack, 0)
    }

    fn array_join_guarded(
        &mut self,
        obj: zun_gc::Obj,
        separator: &str,
        stack: &zun_gc::RootArray,
        depth: usize,
    ) -> Result<Option<String>, RunError> {
        if zun_gc::with_current_heap(|heap| heap.array_len(obj)).is_none() {
            return Ok(None);
        }
        // A cycle, or nesting too deep to be anything else, joins as empty.
        if depth > 256 || (0..depth).any(|level| stack.get(level) == Some(obj)) {
            return Ok(Some(String::new()));
        }
        stack.set(depth, obj);
        let joined = self.array_join_elements(obj, separator, stack, depth);
        stack.clear(depth);
        joined.map(Some)
    }

    fn array_join_elements(
        &mut self,
        obj: zun_gc::Obj,
        separator: &str,
        stack: &zun_gc::RootArray,
        depth: usize,
    ) -> Result<String, RunError> {
        let mut parts: Vec<String> = Vec::new();
        let mut index = 0usize;
        loop {
            // An element's `toString` can run managed code that moves the array.
            let obj = stack.get(depth).unwrap_or(obj);
            let length = zun_gc::with_current_heap(|heap| heap.array_len(obj)).unwrap_or(0);
            if index >= length {
                break;
            }
            let element = zun_gc::with_current_heap(|heap| heap.array_get(obj, index))
                .unwrap_or_else(zun_core::Any::void);
            index += 1;
            let concrete = zun_gc::with_current_heap(|heap| Value::unbox_any(element, heap));
            let text = match concrete {
                Some(Value::Object(inner))
                    if zun_gc::with_current_heap(|heap| heap.bigint_value(inner).is_none()) =>
                {
                    if zun_gc::with_current_heap(|heap| heap.array_len(inner).is_some()) {
                        // A nested array that took an own `toString` / `join` answers it.
                        match self.array_own_primitive(inner, "string")? {
                            Some(primitive) => primitive.display(),
                            None => self
                                .array_join_guarded(inner, ",", stack, depth + 1)?
                                .unwrap_or_default(),
                        }
                    } else {
                        match self
                            .call_managed_collection("stringCoerce", &[Value::Object(inner)])?
                        {
                            Value::Str(text) => text.to_string(),
                            other => other.display(),
                        }
                    }
                }
                _ => zun_gc::with_current_heap(|heap| stringify_any(element, heap))
                    .unwrap_or_default(),
            };
            parts.push(text);
        }
        Ok(parts.join(separator))
    }

    #[allow(clippy::ptr_arg)] // operates on the interpreter operand-stack buffer
    pub(crate) fn coerce_relational_operands(
        &mut self,
        stack: &mut Vec<Value>,
    ) -> Result<(), RunError> {
        let len = stack.len();
        if len < 2 {
            return Ok(());
        }
        let unbox = |v: &Value| -> Value {
            match v {
                Value::Any(any) if any.as_symbol().is_none() => {
                    zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                        .unwrap_or_else(|| v.clone())
                }
                other => other.clone(),
            }
        };
        let (a, b) = (unbox(&stack[len - 2]), unbox(&stack[len - 1]));
        let is_plain_object = |v: &Value| match v {
            Value::Object(obj) => {
                zun_gc::with_current_heap(|heap| heap.bigint_value(*obj).is_none())
            }
            _ => false,
        };
        let bigint_of = |v: &Value| -> Option<zun_core::BigInt> {
            match v {
                Value::Object(obj) => {
                    zun_gc::with_current_heap(|heap| heap.bigint_value(*obj).cloned())
                }
                _ => None,
            }
        };
        // A BigInt against a non-BigInt primitive compares by exact
        // mathematical value, never through a rounded `number`.
        match (bigint_of(&a), bigint_of(&b)) {
            (Some(x), None) if !is_plain_object(&b) || matches!(b, Value::Str(_)) => {
                let order = self.bigint_mixed_order(&x, &b)?;
                Self::store_relational_order(stack, order);
                return Ok(());
            }
            (None, Some(y)) if !is_plain_object(&a) || matches!(a, Value::Str(_)) => {
                let order = self.bigint_mixed_order(&y, &a)?.map(Ordering::reverse);
                Self::store_relational_order(stack, order);
                return Ok(());
            }
            _ => {}
        }
        let numeric = |v: &Value| matches!(v, Value::Int(..) | Value::Float(..));
        let both_bigint = bigint_of(&a).is_some() && bigint_of(&b).is_some();
        let needs = !both_bigint
            && !(numeric(&a) && numeric(&b))
            && !(matches!(a, Value::Str(_)) && matches!(b, Value::Str(_)));
        if !needs {
            return Ok(());
        }
        let mut prims = Vec::with_capacity(2);
        for value in [a, b] {
            let prim = match value {
                Value::Object(obj) if crate::node_buffer::buffer_text(obj).is_some() => {
                    Value::Str(crate::node_buffer::buffer_text(obj).unwrap_or_default().into())
                }
                Value::Object(obj) if is_plain_object(&Value::Object(obj)) => {
                    let overridden = self.array_own_primitive(obj, "number")?;
                    if let Some(primitive) = overridden {
                        primitive
                    } else if let Some(text) = self.array_join_dyn(obj, ",")? {
                        Value::Str(text.into())
                    } else {
                        match self.ordinary_to_primitive(obj, "number")? {
                            Some(prim) => prim,
                            None => Value::Str("[object Object]".into()),
                        }
                    }
                }
                other => other,
            };
            prims.push(prim);
        }
        // ToPrimitive can itself answer a BigInt (`{ valueOf() { return 1n } }`):
        // two BigInts compare as they are, and one against another primitive
        // compares exactly, as above.
        match (bigint_of(&prims[0]), bigint_of(&prims[1])) {
            (Some(_), Some(_)) => {}
            (Some(x), None) => {
                let order = self.bigint_mixed_order(&x, &prims[1])?;
                Self::store_relational_order(stack, order);
                return Ok(());
            }
            (None, Some(y)) => {
                let order = self.bigint_mixed_order(&y, &prims[0])?.map(Ordering::reverse);
                Self::store_relational_order(stack, order);
                return Ok(());
            }
            (None, None) => {
                if !prims.iter().all(|p| matches!(p, Value::Str(_))) {
                    for prim in prims.iter_mut() {
                        let number = self.to_number_value(prim.clone())?;
                        *prim = Value::Float(number, zun_il::NumKind::F64);
                    }
                }
            }
        }
        let second = prims.pop().ok_or(RunError::BadToken)?;
        let first = prims.pop().ok_or(RunError::BadToken)?;
        stack[len - 2] = first;
        stack[len - 1] = second;
        Ok(())
    }

    /// The order of `big` against the non-BigInt primitive `other` (ECMA-262
    /// IsLessThan with one BigInt operand). A string is read with
    /// StringToBigInt and compared as a BigInt; anything else is converted
    /// with ToNumber and compared exactly. `None` means "undefined": an
    /// unparsable string or a `NaN`, for which every relation is false.
    fn bigint_mixed_order(
        &mut self,
        big: &zun_core::BigInt,
        other: &Value,
    ) -> Result<Option<Ordering>, RunError> {
        match other {
            Value::Str(text) => Ok(zun_core::BigInt::parse_js_string(text)
                .ok()
                .map(|parsed| big.cmp(&parsed))),
            Value::Any(any) if any.as_symbol().is_some() => Err(RunError::Native(
                "TypeError: Cannot convert a Symbol value to a number".into(),
            )),
            other => Ok(big.cmp_f64(self.to_number_value(other.clone())?)),
        }
    }

    /// Replace the two operands on top of `stack` with a pair of numbers in
    /// the given order, so the comparison opcode that follows answers it; an
    /// undefined order becomes two `NaN`s, which every relation rejects.
    fn store_relational_order(stack: &mut [Value], order: Option<Ordering>) {
        let (first, second) = match order {
            None => (f64::NAN, f64::NAN),
            Some(Ordering::Less) => (0.0, 1.0),
            Some(Ordering::Equal) => (0.0, 0.0),
            Some(Ordering::Greater) => (1.0, 0.0),
        };
        let len = stack.len();
        stack[len - 2] = Value::Float(first, zun_il::NumKind::F64);
        stack[len - 1] = Value::Float(second, zun_il::NumKind::F64);
    }

    /// Owned-value spelling of [`Interp::dyn_to_number`], the one ToNumber.
    #[inline]
    pub(crate) fn to_number_value(&mut self, value: Value) -> Result<f64, RunError> {
        self.dyn_to_number(&value)
    }

    pub(crate) fn ordinary_to_primitive(
        &mut self,
        obj: zun_gc::Obj,
        hint: &str,
    ) -> Result<Option<Value>, RunError> {
        // A BigInt is a primitive: its string form is its decimal digits (the
        // number hint parses them back). Checked first so the heap object's
        // reserved type id never reaches the class-method tables.
        if let Some(text) =
            zun_gc::with_current_heap(|heap| heap.bigint_value(obj).map(|v| v.to_string()))
        {
            return Ok(Some(Value::Str(text.into())));
        }
        // A Node `Buffer` converts to its utf8 text.
        if let Some(text) = crate::node_buffer::buffer_text(obj) {
            return Ok(Some(Value::Str(text.into())));
        }
        if zun_gc::with_current_heap(|heap| heap.proxy_target(obj).is_some()) {
            return self.proxy_ordinary_to_primitive(obj, hint);
        }
        let tp = zun_core::WellKnownSymbol::ToPrimitive.id();
        let handle = zun_gc::with_current_heap(|heap| heap.dynamic_get_symbol_function(obj, tp))
            .or_else(|| {
                self.class_symbol_methods
                    .get(&(obj.type_rid(), tp.get()))
                    .copied()
            });
        if let Some(handle) = handle {
            let hint_value = zun_gc::with_current_heap_mut(|heap| {
                Value::Any(zun_core::Any::string(heap.intern_string(hint)))
            });
            let call_args = self.with_this_arg(handle, obj, vec![hint_value]);
            let result = self.invoke_function_value(handle, call_args)?;
            let result = unbox_arg(&result)?;
            if matches!(result, Value::Object(_) | Value::Function(_)) {
                return Err(RunError::Native(
                    "TypeError: Cannot convert object to primitive value".into(),
                ));
            }
            return Ok(Some(result));
        }
        // A `[Symbol.toPrimitive]` the object inherits through its linked
        // prototype chain.
        if let Some(handle) = Self::linked_chain_function(obj, |heap, holder| {
            heap.dynamic_get_symbol_function(holder, tp)
        }) {
            let hint_value = zun_gc::with_current_heap_mut(|heap| {
                Value::Any(zun_core::Any::string(heap.intern_string(hint)))
            });
            let call_args = self.inherited_method_args(handle, obj, vec![hint_value]);
            let result = self.invoke_function_value(handle, call_args)?;
            let result = unbox_arg(&result)?;
            if matches!(result, Value::Object(_) | Value::Function(_)) {
                return Err(RunError::Native(
                    "TypeError: Cannot convert object to primitive value".into(),
                ));
            }
            return Ok(Some(result));
        }
        let mut rejected = false;
        let order: [&str; 2] = if hint == "string" {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        for name in order {
            // A typed class instance's own `valueOf`/`toString` method is bound
            // with the instance as the leading `this` argument; a dynamic
            // object's function slot is invoked as a plain closure.
            let typed = self
                .type_rid_describes_object(obj)
                .then(|| {
                    self.class_named_methods
                        .get(&obj.type_rid())
                        .and_then(|methods| methods.get(name))
                        .copied()
                })
                .flatten();
            let method = match typed {
                Some(method) => Some((method, vec![Value::Object(obj)])),
                // A built-in method value (`Number.prototype.valueOf`) is a
                // generic function: it reads its receiver from the leading argument.
                None => zun_gc::with_current_heap(|heap| heap.dynamic_get_function(obj, name))
                    .map(|method| {
                        // A closure that reads `this` takes the receiver as its
                        // hidden first argument, like a built-in method value.
                        let call_args = self.with_this_arg(method, obj, Vec::new());
                        (method, call_args)
                    })
                    // Not an own method: the one the object inherits through
                    // the prototype chain it was linked to (an instance of a
                    // constructor function or of a run-time class).
                    .or_else(|| {
                        let method = Self::linked_chain_function(obj, |heap, holder| {
                            heap.dynamic_get_function(holder, name)
                        })?;
                        Some((method, self.inherited_method_args(method, obj, Vec::new())))
                    }),
            };
            // String hint: `Object.prototype.toString` always exists and
            // returns a primitive, so a missing own `toString` ends the
            // search (it never falls through to `valueOf`).
            if hint == "string" && name == "toString" && method.is_none() {
                return Ok(None);
            }
            if let Some((method, call_args)) = method {
                let result = self.invoke_function_value(method, call_args)?;
                let result = unbox_arg(&result)?;
                if !matches!(result, Value::Object(_) | Value::Function(_)) {
                    return Ok(Some(result));
                }
                rejected = true;
            }
        }
        if rejected {
            return Err(RunError::Native(
                "TypeError: Cannot convert object to primitive value".into(),
            ));
        }
        Ok(None)
    }

    /// ToPrimitive over a Proxy: `[Symbol.toPrimitive]`, then `valueOf` /
    /// `toString`, are each read through the handler's `get` trap (symbol keys
    /// reach the trap as symbol values), and the chosen method is called with
    /// the proxy as `this`. A trap that throws propagates.
    fn proxy_ordinary_to_primitive(
        &mut self,
        proxy: zun_gc::Obj,
        hint: &str,
    ) -> Result<Option<Value>, RunError> {
        // Every step below runs user code that can relocate the proxy.
        zun_gc::root!(proxy = proxy);
        let tp = zun_core::WellKnownSymbol::ToPrimitive.id();
        let method = self.proxy_get_symbol(proxy.get(), tp)?;
        match method {
            Value::Void | Value::Null => {}
            Value::Function(handle) => {
                let hint_value = zun_gc::with_current_heap_mut(|heap| {
                    Value::Any(zun_core::Any::string(heap.intern_string(hint)))
                });
                let call_args = self.with_this_arg(handle, proxy.get(), vec![hint_value]);
                let result = self.invoke_function_value(handle, call_args)?;
                let result = unbox_arg(&result)?;
                if matches!(result, Value::Object(_) | Value::Function(_)) {
                    return Err(RunError::Native(
                        "TypeError: Cannot convert object to primitive value".into(),
                    ));
                }
                return Ok(Some(result));
            }
            _ => {
                return Err(RunError::Native(
                    "TypeError: Symbol(Symbol.toPrimitive) is not a function".into(),
                ));
            }
        }
        let order: [&str; 2] = if hint == "string" {
            ["toString", "valueOf"]
        } else {
            ["valueOf", "toString"]
        };
        let mut rejected = false;
        for name in order {
            let method = self.proxy_dynamic_get(proxy.get(), name)?;
            if let Value::Function(handle) = unbox_arg(&method)? {
                let call_args = self.with_this_arg(handle, proxy.get(), Vec::new());
                let result = self.invoke_function_value(handle, call_args)?;
                let result = unbox_arg(&result)?;
                if !matches!(result, Value::Object(_) | Value::Function(_)) {
                    return Ok(Some(result));
                }
                rejected = true;
            } else if name == "toString" {
                // No own or target `toString`: the inherited one decides. An array
                // target joins its elements (`Array.prototype.toString`); anything
                // else is `Object.prototype.toString`, which always yields a
                // primitive and so ends the search (it never falls to `valueOf`).
                let mut base = proxy.get();
                while let Some(inner) = zun_gc::with_current_heap(|heap| heap.proxy_target(base)) {
                    base = inner;
                }
                if let Some(text) = self.array_join_dyn(base, ",")? {
                    return Ok(Some(Value::Str(text.into())));
                }
                return Ok(None);
            }
        }
        if rejected {
            return Err(RunError::Native(
                "TypeError: Cannot convert object to primitive value".into(),
            ));
        }
        Ok(None)
    }

    /// `proxy[symbol]`: the handler's `get` trap called as `(target, symbol,
    /// receiver)`, or the target's own symbol-keyed slot when there is no trap.
    /// Answers the unboxed value (`Void` when absent).
    fn proxy_get_symbol(
        &mut self,
        proxy: zun_gc::Obj,
        symbol: zun_core::SymbolId,
    ) -> Result<Value, RunError> {
        if zun_gc::with_current_heap(|heap| heap.proxy_is_revoked(proxy)) {
            return Err(self.revoked_proxy_error("get"));
        }
        let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy)) else {
            return Err(RunError::BadToken);
        };
        if let Some(callback) = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "get")) {
            let handler =
                zun_gc::with_current_heap(|heap| heap.proxy_handler(proxy)).unwrap_or(target);
            let args = vec![
                Value::Object(target),
                Value::Any(zun_core::Any::symbol(symbol)),
                Value::Object(proxy),
            ];
            let result =
                self.invoke_function_value(callback, self.with_this_arg(callback, handler, args))?;
            return unbox_arg(&result);
        }
        if zun_gc::with_current_heap(|heap| heap.proxy_target(target).is_some()) {
            return self.proxy_get_symbol(target, symbol);
        }
        if let Some(handle) =
            zun_gc::with_current_heap(|heap| heap.dynamic_get_symbol_function(target, symbol))
                .or_else(|| {
                    self.class_symbol_methods
                        .get(&(target.type_rid(), symbol.get()))
                        .copied()
                })
        {
            return Ok(Value::Function(handle));
        }
        match zun_gc::with_current_heap(|heap| heap.dynamic_get_symbol(target, symbol)) {
            Some(any) => unbox_arg(&Value::Any(any)),
            None => Ok(Value::Void),
        }
    }
}
