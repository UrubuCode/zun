//! Proxy trap native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_proxy(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "proxyNew" => {
                let target =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let handler =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let proxy = zun_gc::with_current_heap_mut(|heap| heap.alloc_proxy(target, handler));
                Ok(Value::Object(proxy))
            }
            "proxyRevocable" => {
                let target =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let handler =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // Keep-alive: `alloc_proxy`/`alloc_dynamic` allocate while the target and handler
                // are still being read.
                zun_gc::root!(target = target);
                zun_gc::root!(handler = handler);
                let result = zun_gc::with_current_heap_mut(|heap| {
                    let proxy = heap.alloc_proxy(target.get(), handler.get());
                    let result = heap.alloc_dynamic();
                    heap.dynamic_set(
                        result,
                        "proxy",
                        zun_core::Any::object(proxy.addr() as usize),
                    );
                    heap.dynamic_set_function(
                        result,
                        "revoke".to_owned(),
                        zun_core::FunctionHandle::new_builtin_method_with_env(
                            crate::property_ops::BUILTIN_PROXY_REVOKE,
                            proxy.addr() as usize,
                        ),
                    );
                    result
                });
                Ok(Value::Object(result))
            }
            "proxyGet" => {
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.clone(),
                    _ => return Err(RunError::BadToken),
                };
                if zun_gc::with_current_heap(|heap| heap.proxy_is_revoked(proxy)) {
                    return Err(self.revoked_proxy_error("get"));
                }
                let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy))
                else {
                    return Err(RunError::BadToken);
                };
                let trap = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "get"));
                let result = if let Some(callback) = trap {
                    let mut call_args = Vec::with_capacity(3);
                    if let Some(address) = callback.environment() {
                        call_args.push(Value::Object(unsafe {
                            zun_gc::Obj::from_addr(address as *mut u64)
                        }));
                    }
                    call_args.push(Value::Object(target));
                    call_args.push(Value::Str(key));
                    // The trap's `receiver` is the proxy the read was made on.
                    call_args.push(Value::Object(proxy));
                    self.call(callback.method_rid(), call_args, None)?
                } else {
                    zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get(target, &key)
                            .map(Value::Any)
                            .unwrap_or(Value::Void)
                    })
                };
                Ok(Value::Any(any(&result).unwrap_or_else(zun_core::Any::null)))
            }
            "proxySet" => {
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let value =
                    any(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.proxy_is_revoked(proxy)) {
                    return Err(self.revoked_proxy_error("set"));
                }
                let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy))
                else {
                    return Err(RunError::BadToken);
                };
                let trap = zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "set"));
                if let Some(callback) = trap {
                    let mut call_args = Vec::with_capacity(4);
                    if let Some(address) = callback.environment() {
                        call_args.push(Value::Object(unsafe {
                            zun_gc::Obj::from_addr(address as *mut u64)
                        }));
                    }
                    call_args.push(Value::Object(target));
                    call_args.push(Value::Str(key));
                    call_args.push(Value::Any(value));
                    // The trap's `receiver` is the proxy the write was made on.
                    call_args.push(Value::Object(proxy));
                    let result = self.call(callback.method_rid(), call_args, None)?;
                    return Ok(Value::Bool(matches!(result, Value::Bool(true))));
                }
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set(target, key.to_string(), value);
                });
                Ok(Value::Bool(true))
            }
            "proxyHas" | "proxyDelete" => {
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.clone(),
                    _ => return Err(RunError::BadToken),
                };
                if zun_gc::with_current_heap(|heap| heap.proxy_is_revoked(proxy)) {
                    return Err(self.revoked_proxy_error(if name == "proxyHas" { "has" } else { "deleteProperty" }));
                }
                let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy))
                else {
                    return Err(RunError::BadToken);
                };
                let trap_name = if name == "proxyHas" {
                    "has"
                } else {
                    "deleteProperty"
                };
                if let Some(callback) =
                    zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, trap_name))
                {
                    let mut call_args = Vec::with_capacity(3);
                    if let Some(address) = callback.environment() {
                        call_args.push(Value::Object(unsafe {
                            zun_gc::Obj::from_addr(address as *mut u64)
                        }));
                    }
                    call_args.push(Value::Object(target));
                    call_args.push(Value::Str(key));
                    let result = self.call(callback.method_rid(), call_args, None)?;
                    return Ok(Value::Bool(matches!(result, Value::Bool(true))));
                }
                let result = zun_gc::with_current_heap_mut(|heap| {
                    if name == "proxyHas" {
                        heap.dynamic_has(target, &key)
                    } else {
                        heap.dynamic_delete(target, &key)
                    }
                });
                Ok(Value::Bool(result))
            }
            "proxyNewCallable" => {
                // A proxy over a callable: the target is a `functionBox` record
                // (a GC object carrying the function under `__boxedFn`), so the
                // proxy's stored target stays a GC object like every other
                // proxy. The handler is an ordinary dynamic object.
                let target =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let handler =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let proxy = zun_gc::with_current_heap_mut(|heap| heap.alloc_proxy(target, handler));
                Ok(Value::Object(proxy))
            }
            "proxyApply" => {
                // `proxy(...args)` / `Reflect.apply(proxy, thisArg, args)`. With
                // an `apply` trap: `handler.apply(target, thisArg, argsArray)`.
                // Without one: invoke the underlying boxed function with the
                // `this` argument threaded as the leading positional slot (the
                // same convention `functionApply` uses).
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let this_arg = args.get(1).ok_or(RunError::BadToken)?.clone();
                let array =
                    object(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.proxy_is_revoked(proxy)) {
                    return Err(self.revoked_proxy_error("apply"));
                }
                let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy))
                else {
                    return Err(RunError::BadToken);
                };
                if let Some(callback) =
                    zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "apply"))
                {
                    let result = self.invoke_proxy_trap(
                        callback,
                        vec![Value::Object(target), this_arg, Value::Object(array)],
                    )?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any_cell(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    return Ok(Value::Any(boxed));
                }
                // A nested proxy forwards the call to the proxy it wraps.
                if zun_gc::with_current_heap(|heap| heap.proxy_target(target).is_some()) {
                    return self.col_proxy(
                        "proxyApply",
                        &[Value::Object(target), this_arg, Value::Object(array)],
                    );
                }
                // No `apply` trap: invoke the underlying boxed function with the
                // spread arguments. The `thisArg` is not prepended (a callable
                // proxy called directly, `proxy(...args)`, forwards exactly its
                // arguments to the target).
                // A proxy whose target is not callable is a `TypeError`, as it
                // is for a plain object: `new Proxy({}, {})()` answered a bare
                // `BadToken`, which is not a JS error and so could not be
                // caught at all.
                let Some(handle) = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_function(target, "__boxedFn")
                }) else {
                    return Err(RunError::Native(
                        "TypeError: proxy target is not a function".into(),
                    ));
                };
                let _ = &this_arg;
                let spread = self.read_array_values(array)?;
                let result = self.invoke_function_value(handle, spread)?;
                let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any_cell(heap))
                    .unwrap_or_else(zun_core::Any::null);
                Ok(Value::Any(boxed))
            }
            "proxyConstruct" => {
                // `new proxy(...args)`: `handler.construct(target, argsArray,
                // newTarget)` when present (the descriptor stands in for both
                // `target` and `newTarget`), else default-construct the class.
                // A builtin buffer constructor value (`new ctor(...)` with a
                // typed array / ArrayBuffer / DataView descriptor in `ctor`).
                if let Some(token) = args.first().and_then(Self::class_token_of) {
                    if token & zun_natives::USER_CLASS_FLAG == 0 {
                        let array = object(args.get(1).ok_or(RunError::BadToken)?)
                            .ok_or(RunError::BadToken)?;
                        let values = self.read_array_values(array)?;
                        if let Some(result) = self.construct_builtin_buffer(token, &values) {
                            let built = result?;
                            let boxed = zun_gc::with_current_heap_mut(|heap| built.box_any(heap))
                                .unwrap_or_else(zun_core::Any::null);
                            return Ok(Value::Any(boxed));
                        }
                    }
                    // Any other class value held in a dynamic local (`const C: any
                    // = SomeClass; new C()`) constructs exactly as
                    // `Reflect.construct(C, args)` does.
                    return self.col_reflect("reflectConstruct", args);
                }
                // An ordinary function value: `[[Construct]]` through its
                // `prototype` (same path as `Reflect.construct(fn, args)`).
                // A function read out of an array or a property arrives in its
                // `functionBox` wrapper (functions do not box into `Any`).
                let callee = args.first().and_then(|arg| unbox_arg(arg).ok()).map(|value| {
                    zun_gc::with_current_heap(|heap| unbox_function_box(value, heap))
                });
                if let Some(Value::Function(handle)) = callee {
                    let array = object(args.get(1).ok_or(RunError::BadToken)?)
                        .ok_or(RunError::BadToken)?;
                    let values = self.read_array_values(array)?;
                    if self.coerce_ctor_name(handle).is_none()
                        && !self.function_is_constructor(handle)
                    {
                        return Err(RunError::Native(
                            "TypeError: value is not a constructor".into(),
                        ));
                    }
                    // A bound function is a constructor only when its target
                    // is one: binding an arrow (or a method shorthand) does
                    // not make it constructible.
                    //
                    // The unwrap's result used to be discarded (`let (target,
                    // _)`), validating the target and then constructing the
                    // *bound* handle with unprepended arguments. A bound
                    // function's `[[Construct]]` discards the `thisArg` that
                    // `bind` captured and constructs the target with the bound
                    // arguments in front, so `new (C.bind(null, 5))()` has to
                    // build an instance of `C`; before this the target ran with
                    // `this` bound to `null` and the first field store threw
                    // `Cannot set properties of null`. The prototype comes from
                    // the target too.
                    let (handle, values) = if handle.is_bound() {
                        let (target, prepended) = self.unwrap_bound_chain(handle, values)?;
                        if self.is_arrow_closure(target) || self.is_method_closure(target) {
                            return Err(self.proto_type_error("value is not a constructor"));
                        }
                        // `unwrap_bound_chain` serves the *call* path, where the
                        // bound `thisArg` occupies the receiver slot, so it comes
                        // back as the first value. Construction supplies its own
                        // fresh receiver, so that leading entry is dropped --
                        // keeping it made the target read the `thisArg` as its
                        // first parameter (`a` was `null`, not `5`).
                        let mut rest = prepended;
                        if !rest.is_empty() {
                            rest.remove(0);
                        }
                        // A bound **class**: the target is its constructor
                        // method, and that has to be constructed through the
                        // class descriptor so the instance gets the class's
                        // typed layout. Run as an ordinary function it got a
                        // plain dynamic object and the first field store threw.
                        let rid = target.method_rid();
                        let owner = self
                            .module
                            .tables
                            .type_defs
                            .iter()
                            .position(|def| rid >= def.method_start && rid < def.method_end);
                        if let Some(index) = owner {
                            let token =
                                zun_natives::USER_CLASS_FLAG | (index as u32 + 1);
                            return self.construct_class_from_descriptor(token, rest);
                        }
                        (target, rest)
                    } else {
                        (handle, values)
                    };
                    return self.construct_ordinary_function(handle, handle, values);
                }
                let proxy = args.first().and_then(object);
                let proxy = match proxy {
                    Some(p) if zun_gc::with_current_heap(|heap| heap.proxy_target(p).is_some()) => p,
                    _ => {
                        return Err(RunError::Native(
                            "TypeError: value is not a constructor".into(),
                        ))
                    }
                };
                let array =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                if zun_gc::with_current_heap(|heap| heap.proxy_is_revoked(proxy)) {
                    return Err(self.revoked_proxy_error("construct"));
                }
                let target = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy))
                    .ok_or(RunError::BadToken)?;
                // A proxy over another proxy without a `construct` trap forwards.
                if zun_gc::with_current_heap(|heap| {
                    heap.proxy_target(target).is_some() && heap.proxy_trap(proxy, "construct").is_none()
                }) {
                    return self.col_proxy(
                        "proxyConstruct",
                        &[Value::Object(target), Value::Object(array)],
                    );
                }
                let descriptor =
                    zun_gc::with_current_heap(|heap| heap.dynamic_get(target, "__classDesc"))
                        .ok_or(RunError::BadToken)?;
                if let Some(callback) =
                    zun_gc::with_current_heap(|heap| heap.proxy_trap(proxy, "construct"))
                {
                    let result = self.invoke_proxy_trap(
                        callback,
                        vec![
                            Value::Any(descriptor),
                            Value::Object(array),
                            Value::Any(descriptor),
                        ],
                    )?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any_cell(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    return Ok(Value::Any(boxed));
                }
                let token = descriptor.as_class_descriptor().ok_or(RunError::BadToken)?;
                let values = self.read_array_values(array)?;
                self.construct_class_from_descriptor(token, values)
            }
            _ => Err(RunError::BadToken),
        }
    }
}
