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
            "proxyGet" => {
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.clone(),
                    _ => return Err(RunError::BadToken),
                };
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
                    call_args.push(Value::Str(key.into()));
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
                    call_args.push(Value::Str(key.into()));
                    call_args.push(Value::Any(value));
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
                    call_args.push(Value::Str(key.into()));
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
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    return Ok(Value::Any(boxed));
                }
                // No `apply` trap: invoke the underlying boxed function with the
                // spread arguments. The `thisArg` is not prepended (a callable
                // proxy called directly, `proxy(...args)`, forwards exactly its
                // arguments to the target).
                let handle = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_function(target, "__boxedFn")
                })
                .ok_or(RunError::BadToken)?;
                let _ = &this_arg;
                let spread = self.read_array_values(array)?;
                let result = self.invoke_function_value(handle, spread)?;
                let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                Ok(Value::Any(boxed))
            }
            "proxyConstruct" => {
                // `new proxy(...args)`: `handler.construct(target, argsArray,
                // newTarget)` when present (the descriptor stands in for both
                // `target` and `newTarget`), else default-construct the class.
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let array =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target = zun_gc::with_current_heap(|heap| heap.proxy_target(proxy))
                    .ok_or(RunError::BadToken)?;
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
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
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
