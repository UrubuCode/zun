//! WeakMap / WeakSet / WeakRef / FinalizationRegistry native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_weak(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "weakMapNew" => {
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_weak_map());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "weakMapSet" => {
                let recv = args.first().ok_or(RunError::BadToken)?.clone();
                let map_object = object(&recv).ok_or(RunError::BadToken)?;
                let key = weak_key(args.get(1).ok_or(RunError::BadToken)?).ok_or_else(|| {
                    RunError::Native("TypeError: Invalid value used as weak map key".to_string())
                })?;
                let raw = args.get(2).ok_or(RunError::BadToken)?;
                // A non-object value is boxed into a hidden dynamic object so
                // the ephemeron floor (object -> object) can hold it.
                let value = match raw {
                    Value::Function(_) => weak_key(raw),
                    _ => object(raw),
                }
                .unwrap_or_else(|| {
                    zun_gc::with_current_heap_mut(|heap| {
                        let boxed = raw.box_any(heap).unwrap_or_else(zun_core::Any::void);
                        let cell = heap.alloc_dynamic();
                        heap.dynamic_set(cell, "__wmv", boxed);
                        cell
                    })
                });
                zun_gc::with_current_heap_mut(|heap| {
                    heap.weak_map_set_object(map_object, key, value)
                });
                Ok(recv)
            }
            "weakMapGet" => {
                let map_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let Some(key) = weak_key(args.get(1).ok_or(RunError::BadToken)?) else {
                    return Ok(Value::Void);
                };
                Ok(zun_gc::with_current_heap(|heap| {
                    heap.weak_map_get_value_object(map_object, key)
                        .map(|value| match heap.dynamic_get(value, "__wmv") {
                            Some(inner) => Value::Any(inner),
                            None => Value::Any(zun_core::Any::object(value.addr() as usize)),
                        })
                })
                .unwrap_or(Value::Void))
            }
            "weakMapHas" => {
                let map_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let Some(key) = weak_key(args.get(1).ok_or(RunError::BadToken)?) else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| {
                    heap.weak_map_has_object(map_object, key)
                })))
            }
            "weakMapDelete" => {
                let map_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let Some(key) = weak_key(args.get(1).ok_or(RunError::BadToken)?) else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.weak_map_delete_object(map_object, key)
                })))
            }
            "weakSetNew" => {
                // A WeakSet reuses the WeakMap ephemeron floor; entries are
                // stored value-less by keying each object to itself.
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_weak_map());
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "weakSetAdd" => {
                let recv = args.first().ok_or(RunError::BadToken)?.clone();
                let set_object = object(&recv).ok_or(RunError::BadToken)?;
                let key = weak_key(args.get(1).ok_or(RunError::BadToken)?).ok_or_else(|| {
                    RunError::Native("TypeError: Invalid value used in weak set".to_string())
                })?;
                zun_gc::with_current_heap_mut(|heap| {
                    heap.weak_map_set_object(set_object, key, key)
                });
                Ok(recv)
            }
            "weakSetHas" => {
                let set_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let Some(key) = weak_key(args.get(1).ok_or(RunError::BadToken)?) else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| {
                    heap.weak_map_has_object(set_object, key)
                })))
            }
            "weakSetDelete" => {
                let set_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let Some(key) = weak_key(args.get(1).ok_or(RunError::BadToken)?) else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.weak_map_delete_object(set_object, key)
                })))
            }
            "weakRefNew" => {
                let target =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let object = zun_gc::with_current_heap_mut(|heap| heap.alloc_weak_ref(target));
                Ok(Value::Any(zun_core::Any::object(object.addr() as usize)))
            }
            "weakRefDeref" => {
                let ref_object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(
                    zun_gc::with_current_heap(|heap| heap.weak_ref_get_object(ref_object))
                        .map(|target| Value::Any(zun_core::Any::object(target.addr() as usize)))
                        .unwrap_or(Value::Void),
                )
            }
            "finalizationRegistryNew" => {
                let cleanup = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(callback) => *callback,
                    _ => return Err(RunError::BadToken),
                };
                let object = zun_gc::with_current_heap_mut(|heap| {
                    let registry = heap.alloc(0, 0);
                    heap.finalization_registry_set_cleanup(registry, cleanup);
                    registry
                });
                Ok(Value::Object(object))
            }
            "finalizationRegister" => {
                let registry =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // The held value is an arbitrary immediate `Any` carrier; box a
                // non-`any` operand into one so an object held value is traced.
                let held = zun_gc::with_current_heap_mut(|heap| {
                    args.get(2)
                        .ok_or(RunError::BadToken)
                        .and_then(|value| value.clone().box_any(heap).ok_or(RunError::BadToken))
                })?;
                // The unregister token is an object identity. A non-object
                // value (the surface's sentinel for an omitted token) leaves
                // the registration permanently un-unregisterable, per spec.
                let unregister_token = args
                    .get(3)
                    .and_then(object)
                    .map(|token| token.addr() as usize)
                    .unwrap_or(0);
                zun_gc::with_current_heap_mut(|heap| {
                    let cleanup = heap
                        .finalization_registry_cleanup(registry)
                        .ok_or(RunError::BadToken)?;
                    heap.finalization_register_with_held(target, held, cleanup, unregister_token);
                    Ok::<(), RunError>(())
                })?;
                Ok(Value::Void)
            }
            "finalizationUnregister" => {
                let _registry =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let unregister_token = args
                    .get(1)
                    .and_then(object)
                    .map(|token| token.addr() as usize)
                    .unwrap_or(0);
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    heap.finalization_unregister_by_token(unregister_token)
                })))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
