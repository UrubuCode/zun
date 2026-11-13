//! ES2025 Iterator helpers: `Iterator.from`, the lazy `map`/`filter`/`take`/
//! `drop`/`flatMap` adapters and the eager `reduce`/`toArray`/`forEach`/`some`/
//! `every`/`find` consumers living on `%Iterator.prototype%`.
//!
//! A lazy adapter is a dynamic object whose prototype is the shared
//! `%IteratorHelperPrototype%` (own `next`/`return`, inheriting the helper
//! methods from `%Iterator.prototype%`). Its state lives in hidden
//! (non-enumerable) own slots so the object stays traceable by the GC.

use super::*;

/// `(js name, arity)` of the `%Iterator.prototype%` helper methods; the index
/// is added to [`BUILTIN_ITER_HELPER_BASE`] to form the built-in kind.
pub(crate) const ITER_HELPERS: [(&str, usize); 11] = [
    ("map", 1),
    ("filter", 1),
    ("take", 1),
    ("drop", 1),
    ("flatMap", 1),
    ("reduce", 1),
    ("toArray", 0),
    ("forEach", 1),
    ("some", 1),
    ("every", 1),
    ("find", 1),
];

pub(crate) const BUILTIN_ITER_HELPER_BASE: u32 = 120;
pub(crate) const BUILTIN_ITER_FROM: u32 = 131;
pub(crate) const BUILTIN_ITERH_NEXT: u32 = 132;
pub(crate) const BUILTIN_ITERH_RETURN: u32 = 133;
pub(crate) const BUILTIN_WRAP_NEXT: u32 = 134;
pub(crate) const BUILTIN_WRAP_RETURN: u32 = 135;

const K_MAP: u32 = 0;
const K_FILTER: u32 = 1;
const K_TAKE: u32 = 2;
const K_DROP: u32 = 3;
const K_FLAT_MAP: u32 = 4;
const K_REDUCE: u32 = 5;
const K_TO_ARRAY: u32 = 6;
#[allow(dead_code)]
const K_FOR_EACH: u32 = 7;
const K_SOME: u32 = 8;
const K_EVERY: u32 = 9;
const K_FIND: u32 = 10;

/// The helper-method index of `name`, if it is one of [`ITER_HELPERS`].
pub(crate) fn iter_helper_index(name: &str) -> Option<u32> {
    ITER_HELPERS
        .iter()
        .position(|entry| entry.0 == name)
        .map(|index| index as u32)
}

/// The symbol-keyed method `symbol` found on `obj` or its dynamic prototype
/// chain.
pub(crate) fn symbol_function_in_chain(
    obj: zun_gc::Obj,
    symbol: zun_core::SymbolId,
) -> Option<zun_core::FunctionHandle> {
    let mut current = obj;
    for _ in 0..256 {
        if let Some(handle) =
            zun_gc::with_current_heap(|heap| heap.dynamic_get_symbol_function(current, symbol))
        {
            return Some(handle);
        }
        current = zun_gc::with_current_heap(|heap| {
            heap.dynamic_prototype(current)
                .and_then(|any| any.as_object())
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
        })?;
    }
    None
}

/// Whether `obj` is a built-in iterator or generator (the receivers whose
/// prototype chain reaches `%Iterator.prototype%` without a dynamic link).
pub(crate) fn is_builtin_iterator_object(obj: zun_gc::Obj) -> bool {
    zun_gc::with_current_heap(|heap| {
        heap.generator_state(obj).is_some()
            || heap.array_iterator_done(obj).is_some()
            || heap.set_iterator_done(obj).is_some()
            || heap.map_iterator_done(obj).is_some()
    })
}

fn float_any(value: f64) -> zun_core::Any {
    zun_core::Any::float(value)
}

fn hidden(heap: &mut zun_gc::Heap, obj: zun_gc::Obj, key: &str, value: zun_core::Any) {
    heap.dynamic_define_data(obj, key.to_owned(), value, true, false, true);
}

fn get_f64(obj: zun_gc::Obj, key: &str) -> f64 {
    zun_gc::with_current_heap(|heap| {
        heap.dynamic_get(obj, key)
            .and_then(|any| any.as_float())
            .unwrap_or(0.0)
    })
}

fn get_obj(obj: zun_gc::Obj, key: &str) -> Option<zun_gc::Obj> {
    zun_gc::with_current_heap(|heap| {
        heap.dynamic_get(obj, key)
            .and_then(|any| any.as_object())
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
    })
}

fn unboxed(value: &Value) -> Value {
    match value {
        Value::Any(any) => {
            zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)).unwrap_or(Value::Void)
        }
        other => other.clone(),
    }
}

fn value_object(value: &Value) -> Option<zun_gc::Obj> {
    match unboxed(value) {
        Value::Object(obj) => {
            if zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some()) {
                None
            } else {
                Some(obj)
            }
        }
        _ => None,
    }
}

fn callable(value: &Value) -> Option<zun_core::FunctionHandle> {
    match unboxed(value) {
        Value::Function(handle) => Some(handle),
        _ => None,
    }
}

fn state_set(obj: zun_gc::Obj, value: &Value, key: &str) {
    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
        .unwrap_or_else(zun_core::Any::void);
    zun_gc::with_current_heap_mut(|heap| hidden(heap, obj, key, boxed));
}

fn state_get(obj: zun_gc::Obj, key: &str) -> Value {
    zun_gc::with_current_heap(|heap| {
        heap.dynamic_get(obj, key)
            .and_then(|any| Value::unbox_any(any, heap))
    })
    .unwrap_or(Value::Void)
}

impl<'a> Interp<'a> {
    /// Populate the `Iterator` namespace object registered on the realm global:
    /// `Iterator.from`, `Iterator.prototype` (the shared `%Iterator.prototype%`)
    /// and the prototype's back-link `constructor`.
    pub(crate) fn install_iterator_global(namespace: zun_gc::Obj) {
        let proto = Self::iterator_prototype_object(0);
        // Keep-alive: `dynamic_set_function`/`dynamic_define_data` allocate while both objects
        // are still read.
        zun_gc::root!(namespace = namespace);
        zun_gc::root!(proto = proto);
        zun_gc::with_current_heap_mut(|heap| {
            heap.dynamic_set_function(
                namespace.get(),
                "from".to_owned(),
                zun_core::FunctionHandle::new_builtin_method(BUILTIN_ITER_FROM),
            );
            heap.dynamic_define_data(
                namespace.get(),
                "prototype".to_owned(),
                zun_core::Any::object(proto.get().addr() as usize),
                false,
                false,
                false,
            );
            heap.dynamic_define_data(
                proto.get(),
                "constructor".to_owned(),
                zun_core::Any::object(namespace.get().addr() as usize),
                true,
                false,
                true,
            );
        });
    }

    fn iter_error(&mut self, class: &str, message: &str) -> RunError {
        match self.new_error_object(class, message) {
            Some(object) => RunError::Thrown(Value::Object(object)),
            None => RunError::Native(format!("{class}: {message}")),
        }
    }

    /// Call `handle` as a user callback with `this` unbound.
    fn iter_call(
        &mut self,
        handle: zun_core::FunctionHandle,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        if handle.builtin_method().is_some() {
            return self.invoke_function_value(handle, args);
        }
        self.invoke_callback(handle, args)
    }

    /// Whether `obj` inherits `%Iterator.prototype%`.
    fn iter_inherits_proto(&mut self, obj: zun_gc::Obj) -> bool {
        if is_builtin_iterator_object(obj) {
            return true;
        }
        let target = Self::iterator_prototype_object(0).addr();
        let mut current = obj;
        for _ in 0..256 {
            let Some(next) = zun_gc::with_current_heap(|heap| {
                heap.dynamic_prototype(current)
                    .and_then(|any| any.as_object())
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
            }) else {
                return false;
            };
            if next.addr() == target {
                return true;
            }
            current = next;
        }
        false
    }

    /// Read `name` off an iterator object as a callable, if it is one.
    pub(crate) fn iter_method(
        &mut self,
        it: zun_gc::Obj,
        name: &str,
    ) -> Result<Option<zun_core::FunctionHandle>, RunError> {
        let found = self.dynamic_member_get(it, name)?;
        Ok(match found {
            Value::Function(handle) => Some(handle),
            Value::Any(any) => zun_gc::with_current_heap(|heap| {
                let concrete = Value::unbox_any(any, heap)?;
                match concrete {
                    Value::Function(handle) => Some(handle),
                    Value::Object(cell) => heap.function_of_cell(cell),
                    _ => None,
                }
            }),
            _ => None,
        })
    }

    /// Advance an arbitrary iterator once: `(value, done)`.
    pub(crate) fn iter_step(&mut self, it: zun_gc::Obj) -> Result<(Value, bool), RunError> {
        if is_builtin_iterator_object(it) {
            return self.protocol_step(it);
        }
        if zun_gc::with_current_heap(|heap| heap.dynamic_has(it, "$ih")) {
            return self.helper_next(it);
        }
        let result = self.iter_raw_next(it)?;
        let Some(result_obj) = value_object(&result) else {
            let text = "Iterator result is not an object";
            return Err(self.iter_error("TypeError", text));
        };
        // `done` and `value` are read through `dynamic_member_get`, which runs an
        // accessor if the result object defines one; that can evacuate it between
        // the two reads.
        zun_gc::root!(result_obj = result_obj);
        let done = {
            let raw = self.dynamic_member_get(result_obj.get(), "done")?;
            zun_gc::with_current_heap(|heap| is_truthy(&raw, heap))
        };
        if done {
            return Ok((Value::Void, true));
        }
        let value = self.dynamic_member_get(result_obj.get(), "value")?;
        Ok((unboxed(&value), false))
    }

    /// `iterator.next()` returning the raw result value.
    fn iter_raw_next(&mut self, it: zun_gc::Obj) -> Result<Value, RunError> {
        // `iter_method` can run a getter and evacuate `it` before it is `this`.
        zun_gc::root!(it = it);
        let Some(next) = self.iter_method(it.get(), "next")? else {
            return Err(self.iter_error("TypeError", "iterator.next is not a function"));
        };
        let call_args = self.with_this_arg(next, it.get(), Vec::new());
        self.invoke_function_value(next, call_args)
    }

    /// IteratorClose. With `throwing` an error from `return()` is swallowed.
    pub(crate) fn iter_close(&mut self, it: zun_gc::Obj, throwing: bool) -> Result<(), RunError> {
        // Keep-alive: closing re-enters user `return()` code; `it` is read back only for the
        // call.
        zun_gc::root!(it = it);
        let outcome = self.iter_close_inner(it.get());
        match outcome {
            Err(_) if throwing => Ok(()),
            other => other,
        }
    }

    fn iter_close_inner(&mut self, it: zun_gc::Obj) -> Result<(), RunError> {
        let (is_gen, is_builtin) = zun_gc::with_current_heap(|heap| {
            (
                heap.generator_state(it).is_some(),
                heap.array_iterator_done(it).is_some()
                    || heap.set_iterator_done(it).is_some()
                    || heap.map_iterator_done(it).is_some(),
            )
        });
        if is_gen {
            return self.close_generator(it, false).map(|_| ());
        }
        if is_builtin {
            return Ok(());
        }
        if zun_gc::with_current_heap(|heap| heap.dynamic_has(it, "$ih")) {
            return self.helper_close(it);
        }
        // `iter_method` can run a getter and evacuate `it` before it is `this`.
        zun_gc::root!(it = it);
        let Some(method) = self.iter_method(it.get(), "return")? else {
            return Ok(());
        };
        let call_args = self.with_this_arg(method, it.get(), Vec::new());
        let result = self.invoke_function_value(method, call_args)?;
        if value_object(&result).is_none() {
            return Err(self.iter_error("TypeError", "iterator.return() did not return an object"));
        }
        Ok(())
    }

    // ---- lazy helper objects ----

    fn helper_alloc(
        &mut self,
        kind: u32,
        src: zun_gc::Obj,
        callback: Option<zun_core::FunctionHandle>,
        remaining: f64,
    ) -> Value {
        let proto = Self::iterator_prototype_object(3);
        // Defensive: allocating never collects by itself (only the dispatch loop polls the
        // GC budget), so this guards the two addresses against a re-entry added here later.
        zun_gc::root!(src = src);
        zun_gc::root!(proto = proto);
        let helper = zun_gc::with_current_heap_mut(|heap| {
            let helper = heap.alloc_dynamic();
            heap.dynamic_set_prototype(
                helper,
                Some(zun_core::Any::object(proto.get().addr() as usize)),
            );
            hidden(heap, helper, "$ih", float_any(kind as f64));
            hidden(
                heap,
                helper,
                "$src",
                zun_core::Any::object(src.get().addr() as usize),
            );
            hidden(heap, helper, "$i", float_any(0.0));
            hidden(heap, helper, "$rem", float_any(remaining));
            hidden(heap, helper, "$done", zun_core::Any::bool(false));
            if let Some(callback) = callback {
                heap.dynamic_set_function(helper, "$fn", callback);
            }
            helper
        });
        Value::Any(zun_core::Any::object(helper.addr() as usize))
    }

    fn helper_mark_done(helper: zun_gc::Obj) {
        zun_gc::with_current_heap_mut(|heap| {
            hidden(heap, helper, "$done", zun_core::Any::bool(true));
        });
    }

    fn helper_is_done(helper: zun_gc::Obj) -> bool {
        zun_gc::with_current_heap(|heap| {
            heap.dynamic_get(helper, "$done")
                .and_then(|any| any.as_bool())
                .unwrap_or(false)
        })
    }

    /// `helper.return()` semantics: close the flattened inner iterator, then
    /// the source, and finish the helper.
    fn helper_close(&mut self, helper: zun_gc::Obj) -> Result<(), RunError> {
        if Self::helper_is_done(helper) {
            return Ok(());
        }
        Self::helper_mark_done(helper);
        // `iter_close(inner)` runs `return()` and can evacuate the helper, whose
        // `$src` slot is read back afterwards.
        zun_gc::root!(helper = helper);
        if let Some(inner) = get_obj(helper.get(), "$inner") {
            zun_gc::with_current_heap_mut(|heap| {
                heap.dynamic_delete(helper.get(), "$inner");
            });
            if let Err(error) = self.iter_close(inner, false) {
                if let Some(src) = get_obj(helper.get(), "$src") {
                    let _ = self.iter_close(src, true);
                }
                return Err(error);
            }
        }
        let Some(src) = get_obj(helper.get(), "$src") else {
            return Ok(());
        };
        self.iter_close(src, false)
    }

    /// Close the source after a callback/validation failure and rethrow.
    fn helper_abort(&mut self, helper: zun_gc::Obj, error: RunError) -> RunError {
        Self::helper_mark_done(helper);
        // Closing the inner iterator runs `return()` and can evacuate the helper
        // before `$src` is read.
        zun_gc::root!(helper = helper);
        if let Some(inner) = get_obj(helper.get(), "$inner") {
            let _ = self.iter_close(inner, true);
        }
        if let Some(src) = get_obj(helper.get(), "$src") {
            let _ = self.iter_close(src, true);
        }
        error
    }

    /// One pull of a lazy helper: `(value, done)`.
    fn helper_next(&mut self, helper: zun_gc::Obj) -> Result<(Value, bool), RunError> {
        if Self::helper_is_done(helper) {
            return Ok((Value::Void, true));
        }
        // Everything below re-enters managed code (`iter_step`, the callback, a
        // flattened inner iterator) and can evacuate the helper and its source,
        // so neither is held in a plain local across those calls; the callback
        // is re-read from the helper slot at each use for the same reason.
        zun_gc::root!(helper = helper);
        let kind = get_f64(helper.get(), "$ih") as u32;
        let Some(src) = get_obj(helper.get(), "$src") else {
            return Ok((Value::Void, true));
        };
        zun_gc::root!(src = src);
        match kind {
            K_MAP | K_FILTER => loop {
                let (value, done) = match self.iter_step(src.get()) {
                    Ok(pair) => pair,
                    Err(error) => {
                        Self::helper_mark_done(helper.get());
                        return Err(error);
                    }
                };
                if done {
                    Self::helper_mark_done(helper.get());
                    return Ok((Value::Void, true));
                }
                let index = get_f64(helper.get(), "$i");
                zun_gc::with_current_heap_mut(|heap| {
                    hidden(heap, helper.get(), "$i", float_any(index + 1.0))
                });
                state_set(helper.get(), &value, "$cur");
                let callback = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_function(helper.get(), "$fn")
                })
                .ok_or(RunError::BadToken)?;
                let args = vec![
                    zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .map(Value::Any)
                        .unwrap_or(Value::Void),
                    Value::Any(float_any(index)),
                ];
                let produced = match self.iter_call(callback, args) {
                    Ok(produced) => produced,
                    Err(error) => return Err(self.helper_abort(helper.get(), error)),
                };
                if kind == K_MAP {
                    return Ok((unboxed(&produced), false));
                }
                let keep = zun_gc::with_current_heap(|heap| is_truthy(&produced, heap));
                if keep {
                    return Ok((state_get(helper.get(), "$cur"), false));
                }
            },
            K_TAKE => {
                let remaining = get_f64(helper.get(), "$rem");
                if remaining <= 0.0 {
                    Self::helper_mark_done(helper.get());
                    self.iter_close(src.get(), false)?;
                    return Ok((Value::Void, true));
                }
                if remaining.is_finite() {
                    zun_gc::with_current_heap_mut(|heap| {
                        hidden(heap, helper.get(), "$rem", float_any(remaining - 1.0))
                    });
                }
                match self.iter_step(src.get()) {
                    Ok((value, done)) => {
                        if done {
                            Self::helper_mark_done(helper.get());
                        }
                        Ok((value, done))
                    }
                    Err(error) => {
                        Self::helper_mark_done(helper.get());
                        Err(error)
                    }
                }
            }
            K_DROP => {
                loop {
                    let remaining = get_f64(helper.get(), "$rem");
                    if remaining <= 0.0 {
                        break;
                    }
                    if remaining.is_finite() {
                        zun_gc::with_current_heap_mut(|heap| {
                            hidden(heap, helper.get(), "$rem", float_any(remaining - 1.0))
                        });
                    }
                    match self.iter_step(src.get()) {
                        Ok((_, true)) => {
                            Self::helper_mark_done(helper.get());
                            return Ok((Value::Void, true));
                        }
                        Ok(_) => {}
                        Err(error) => {
                            Self::helper_mark_done(helper.get());
                            return Err(error);
                        }
                    }
                }
                match self.iter_step(src.get()) {
                    Ok((value, done)) => {
                        if done {
                            Self::helper_mark_done(helper.get());
                        }
                        Ok((value, done))
                    }
                    Err(error) => {
                        Self::helper_mark_done(helper.get());
                        Err(error)
                    }
                }
            }
            K_FLAT_MAP => loop {
                if let Some(inner) = get_obj(helper.get(), "$inner") {
                    match self.iter_step(inner) {
                        Ok((value, false)) => return Ok((value, false)),
                        Ok((_, true)) => {
                            zun_gc::with_current_heap_mut(|heap| {
                                heap.dynamic_delete(helper.get(), "$inner");
                            });
                            continue;
                        }
                        Err(error) => return Err(self.helper_abort(helper.get(), error)),
                    }
                }
                let (value, done) = match self.iter_step(src.get()) {
                    Ok(pair) => pair,
                    Err(error) => {
                        Self::helper_mark_done(helper.get());
                        return Err(error);
                    }
                };
                if done {
                    Self::helper_mark_done(helper.get());
                    return Ok((Value::Void, true));
                }
                let index = get_f64(helper.get(), "$i");
                zun_gc::with_current_heap_mut(|heap| {
                    hidden(heap, helper.get(), "$i", float_any(index + 1.0))
                });
                let callback = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_function(helper.get(), "$fn")
                })
                .ok_or(RunError::BadToken)?;
                let args = vec![
                    zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .map(Value::Any)
                        .unwrap_or(Value::Void),
                    Value::Any(float_any(index)),
                ];
                let mapped = match self.iter_call(callback, args) {
                    Ok(mapped) => mapped,
                    Err(error) => return Err(self.helper_abort(helper.get(), error)),
                };
                // The inner iterator must be kept alive while it is resolved.
                let inner = match self.iter_flattenable(&mapped, false) {
                    Ok(inner) => inner,
                    Err(error) => return Err(self.helper_abort(helper.get(), error)),
                };
                zun_gc::with_current_heap_mut(|heap| {
                    hidden(
                        heap,
                        helper.get(),
                        "$inner",
                        zun_core::Any::object(inner.addr() as usize),
                    )
                });
            },
            _ => Err(RunError::BadToken),
        }
    }

    /// `GetIteratorFlattenable`: the iterator of `value` (an object, or a
    /// string when `allow_string`); an object without `@@iterator` is its own
    /// iterator.
    fn iter_flattenable(
        &mut self,
        value: &Value,
        allow_string: bool,
    ) -> Result<zun_gc::Obj, RunError> {
        let concrete = unboxed(value);
        let obj = match &concrete {
            Value::Str(_) if allow_string => {
                let made = self.call_managed_collection("stringIteratorNew", &[concrete.clone()])?;
                return value_object(&made).ok_or(RunError::BadToken);
            }
            Value::Function(handle) => crate::function_expando_bag(*handle),
            other => match value_object(other) {
                Some(obj) => obj,
                None => {
                    return Err(self.iter_error("TypeError", "value is not an object"));
                }
            },
        };
        // `resolve_iterator` may call a user `[Symbol.iterator]()` that evacuates `obj`;
        // the fallback below (an object that is its own iterator) needs the new address.
        zun_gc::root!(obj = obj);
        match self.resolve_iterator(obj.get()) {
            Ok(iterator) => Ok(iterator),
            Err(RunError::Native(message)) if message.contains("not iterable") => Ok(obj.get()),
            Err(error) => Err(error),
        }
    }

    // ---- built-in method dispatch ----

    /// Run an Iterator-helper built-in (`kind` in the 120..=135 range).
    pub(crate) fn iterator_builtin_call(
        &mut self,
        kind: u32,
        this: Value,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        match kind {
            BUILTIN_ITER_FROM => self.iterator_from(args.first()),
            BUILTIN_ITERH_NEXT => {
                let Some(helper) = value_object(&this)
                    .filter(|o| zun_gc::with_current_heap(|heap| heap.dynamic_has(*o, "$ih")))
                else {
                    return Err(self.iter_error(
                        "TypeError",
                        "Method Iterator Helper.prototype.next called on incompatible receiver",
                    ));
                };
                let (value, done) = self.helper_next(helper)?;
                Ok(self.make_iter_result(if done { Value::Void } else { value }, done))
            }
            BUILTIN_ITERH_RETURN => {
                let Some(helper) = value_object(&this)
                    .filter(|o| zun_gc::with_current_heap(|heap| heap.dynamic_has(*o, "$ih")))
                else {
                    return Err(self.iter_error(
                        "TypeError",
                        "Method Iterator Helper.prototype.return called on incompatible receiver",
                    ));
                };
                self.helper_close(helper)?;
                Ok(self.make_iter_result(Value::Void, true))
            }
            BUILTIN_WRAP_NEXT | BUILTIN_WRAP_RETURN => {
                let Some(wrapper) = value_object(&this)
                    .filter(|o| zun_gc::with_current_heap(|heap| heap.dynamic_has(*o, "$w")))
                else {
                    return Err(self.iter_error(
                        "TypeError",
                        "Method WrapForValidIteratorPrototype called on incompatible receiver",
                    ));
                };
                let under = get_obj(wrapper, "$w").ok_or(RunError::BadToken)?;
                if kind == BUILTIN_WRAP_NEXT {
                    if is_builtin_iterator_object(under) {
                        let (value, done) = self.protocol_step(under)?;
                        return Ok(self.make_iter_result(if done { Value::Void } else { value }, done));
                    }
                    return self.iter_raw_next(under);
                }
                if is_builtin_iterator_object(under) {
                    self.iter_close(under, false)?;
                    return Ok(self.make_iter_result(Value::Void, true));
                }
                let Some(method) = self.iter_method(under, "return")? else {
                    return Ok(self.make_iter_result(Value::Void, true));
                };
                let call_args = self.with_this_arg(method, under, Vec::new());
                self.invoke_function_value(method, call_args)
            }
            _ => {
                let index = kind - BUILTIN_ITER_HELPER_BASE;
                self.iterator_helper_method(index, this, args)
            }
        }
    }

    fn iterator_from(&mut self, arg: Option<&Value>) -> Result<Value, RunError> {
        let value = arg.cloned().unwrap_or(Value::Void);
        let iterator = self.iter_flattenable(&value, true)?;
        // Defensive: `iter_inherits_proto` only reads the heap and allocating never collects
        // by itself, so nothing here moves the iterator today; the guard is for the day one
        // of these steps reaches user code.
        zun_gc::root!(iterator = iterator);
        if self.iter_inherits_proto(iterator.get()) {
            return Ok(Value::Any(zun_core::Any::object(iterator.get().addr() as usize)));
        }
        let proto = Self::iterator_prototype_object(4);
        zun_gc::root!(proto = proto);
        let wrapper = zun_gc::with_current_heap_mut(|heap| {
            let wrapper = heap.alloc_dynamic();
            heap.dynamic_set_prototype(
                wrapper,
                Some(zun_core::Any::object(proto.get().addr() as usize)),
            );
            hidden(
                heap,
                wrapper,
                "$w",
                zun_core::Any::object(iterator.get().addr() as usize),
            );
            wrapper
        });
        Ok(Value::Any(zun_core::Any::object(wrapper.addr() as usize)))
    }

    fn iterator_helper_method(
        &mut self,
        index: u32,
        this: Value,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        let name = ITER_HELPERS[index as usize].0;
        let Some(it) = value_object(&this) else {
            let message = format!("Iterator.prototype.{name} called on non-object");
            return Err(self.iter_error("TypeError", &message));
        };
        // `to_number_value` can run user `valueOf`, which allocates and can evacuate the
        // iterator before it is closed or wrapped; read it back through the guard.
        zun_gc::root!(it = it);
        let first = args.first().cloned().unwrap_or(Value::Void);
        match index {
            K_TAKE | K_DROP => {
                let limit = match self.to_number_value(first) {
                    Ok(number) => number,
                    Err(error) => {
                        let _ = self.iter_close(it.get(), true);
                        return Err(error);
                    }
                };
                if limit.is_nan() {
                    let _ = self.iter_close(it.get(), true);
                    return Err(self.iter_error("RangeError", "limit must be a number >= 0"));
                }
                let limit = limit.trunc();
                if limit < 0.0 {
                    let _ = self.iter_close(it.get(), true);
                    return Err(self.iter_error("RangeError", "limit must be a number >= 0"));
                }
                Ok(self.helper_alloc(index, it.get(), None, limit))
            }
            K_REDUCE => {
                let Some(reducer) = callable(&first) else {
                    let _ = self.iter_close(it.get(), true);
                    return Err(self.iter_error("TypeError", "reducer is not a function"));
                };
                self.iter_reduce(it.get(), reducer, args.get(1).cloned())
            }
            K_TO_ARRAY => self.iter_consume(index, it.get(), None),
            _ => {
                let Some(callback) = callable(&first) else {
                    let _ = self.iter_close(it.get(), true);
                    let message = format!("{name} callback is not a function");
                    return Err(self.iter_error("TypeError", &message));
                };
                match index {
                    K_MAP | K_FILTER | K_FLAT_MAP => {
                        Ok(self.helper_alloc(index, it.get(), Some(callback), 0.0))
                    }
                    _ => self.iter_consume(index, it.get(), Some(callback)),
                }
            }
        }
    }

    fn iter_reduce(
        &mut self,
        it: zun_gc::Obj,
        reducer: zun_core::FunctionHandle,
        initial: Option<Value>,
    ) -> Result<Value, RunError> {
        let scratch = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
        // Every `iter_step` and the reducer re-enter managed code and can evacuate
        // all three: the reducer is a closure, so its environment is a managed
        // address like the other two.
        zun_gc::root!(it = it);
        zun_gc::root!(scratch = scratch);
        zun_gc::root!(reducer = reducer);
        let mut counter = 0.0;
        match initial {
            Some(value) => state_set(scratch.get(), &unboxed(&value), "acc"),
            None => {
                let (value, done) = self.iter_step(it.get())?;
                if done {
                    return Err(self.iter_error(
                        "TypeError",
                        "Reduce of empty iterator with no initial value",
                    ));
                }
                state_set(scratch.get(), &value, "acc");
                counter = 1.0;
            }
        }
        loop {
            let (value, done) = self.iter_step(it.get())?;
            if done {
                return Ok(state_get(scratch.get(), "acc"));
            }
            state_set(scratch.get(), &value, "cur");
            let boxed = |v: &Value| {
                zun_gc::with_current_heap_mut(|heap| v.box_any(heap))
                    .map(Value::Any)
                    .unwrap_or(Value::Void)
            };
            let args = vec![
                boxed(&state_get(scratch.get(), "acc")),
                boxed(&value),
                Value::Any(float_any(counter)),
            ];
            match self.iter_call(reducer.get(), args) {
                Ok(next) => state_set(scratch.get(), &unboxed(&next), "acc"),
                Err(error) => {
                    let _ = self.iter_close(it.get(), true);
                    return Err(error);
                }
            }
            counter += 1.0;
        }
    }

    /// `toArray` / `forEach` / `some` / `every` / `find`.
    fn iter_consume(
        &mut self,
        index: u32,
        it: zun_gc::Obj,
        callback: Option<zun_core::FunctionHandle>,
    ) -> Result<Value, RunError> {
        let scratch = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
        let out = if index == K_TO_ARRAY {
            Some(zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0)))
        } else {
            None
        };
        // Every `iter_step` and the callback re-enter managed code and can evacuate
        // all of these: the callback is a closure, so its environment is a managed
        // address like the objects are. `K_TO_ARRAY` is the one kind without a
        // callback, and a `None` guard simply has no address to rewrite.
        zun_gc::root!(it = it);
        zun_gc::root!(scratch = scratch);
        zun_gc::root!(out = out.unwrap_or(scratch.get()));
        zun_gc::root!(callback = callback);
        let mut counter = 0.0;
        loop {
            let (value, done) = self.iter_step(it.get())?;
            if done {
                return Ok(match index {
                    K_TO_ARRAY => Value::Any(zun_core::Any::object(out.get().addr() as usize)),
                    K_EVERY => Value::Bool(true),
                    K_SOME => Value::Bool(false),
                    _ => Value::Void,
                });
            }
            state_set(scratch.get(), &value, "cur");
            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .unwrap_or_else(zun_core::Any::void);
            if index == K_TO_ARRAY {
                zun_gc::with_current_heap_mut(|heap| heap.array_push(out.get(), boxed));
                continue;
            }
            let callback = callback.get().ok_or(RunError::BadToken)?;
            let args = vec![Value::Any(boxed), Value::Any(float_any(counter))];
            counter += 1.0;
            let produced = match self.iter_call(callback, args) {
                Ok(produced) => produced,
                Err(error) => {
                    let _ = self.iter_close(it.get(), true);
                    return Err(error);
                }
            };
            let truthy = zun_gc::with_current_heap(|heap| is_truthy(&produced, heap));
            match index {
                K_SOME if truthy => {
                    self.iter_close(it.get(), false)?;
                    return Ok(Value::Bool(true));
                }
                K_EVERY if !truthy => {
                    self.iter_close(it.get(), false)?;
                    return Ok(Value::Bool(false));
                }
                K_FIND if truthy => {
                    self.iter_close(it.get(), false)?;
                    return Ok(state_get(scratch.get(), "cur"));
                }
                _ => {}
            }
        }
    }
}
