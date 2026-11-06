//! Object/global/symbol enumeration, freeze, prototype, and realm native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_object_core(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "objectForInKeys" => {
                // `for (k in obj)`: own enumerable keys first, then enumerable
                // keys inherited along the explicit prototype chain; a key
                // already seen (even a non-enumerable own one) shadows.
                // A primitive receiver: a string enumerates its character
                // indices; every other primitive (and null/undefined) has none.
                let obj = match unbox_arg(args.first().ok_or(RunError::BadToken)?)? {
                    Value::Object(obj) => obj,
                    other => {
                        let count = match &other {
                            Value::Str(text) => zun_core::wtf16::len16(&text),
                            _ => 0,
                        };
                        let array = zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(count);
                            for index in 0..count {
                                let item = Value::Str(index.to_string().into())
                                    .box_any(heap)
                                    .unwrap_or_else(zun_core::Any::null);
                                let _ = heap.array_set(array, index, item);
                            }
                            array
                        });
                        return Ok(Value::Object(array));
                    }
                };
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj) && heap.array_len(obj).is_none() && !heap.is_instance_bag(obj));
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut keys: Vec<String> = Vec::new();
                if is_dynamic {
                    let mut current = obj;
                    for _ in 0..256 {
                        let own = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_keys(current).unwrap_or_default().into_iter()
                                .map(|key| {
                                    let enumerable = heap.dynamic_is_enumerable(current, &key);
                                    (key, enumerable)
                                })
                                .collect::<Vec<_>>()
                        });
                        for (key, enumerable) in own {
                            if seen.insert(key.clone()) && enumerable {
                                keys.push(key);
                            }
                        }
                        let next = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_prototype(current)
                                .and_then(|any| any.as_object())
                                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                        });
                        match next {
                            Some(next) => current = next,
                            None => break,
                        }
                    }
                } else {
                    keys = self.object_own_entries(obj)?.into_iter().map(|(key, _)| key).collect();
                }
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(keys.len());
                    for (index, key) in keys.iter().enumerate() {
                        let item = Value::Str(key.clone().into())
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        let _ = heap.array_set(array, index, item);
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectKeys" => {
                // Primitives: a string exposes its characters, other
                // primitives have no own properties, null/undefined throw.
                match unbox_arg(args.first().ok_or(RunError::BadToken)?)? {
                    Value::Str(text) => {
                        let count = zun_core::wtf16::len16(&text);
                        let array = zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(count);
                            for index in 0..count {
                                let item = Value::Str(index.to_string().into())
                                    .box_any(heap)
                                    .unwrap_or_else(zun_core::Any::null);
                                let _ = heap.array_set(array, index, item);
                            }
                            array
                        });
                        return Ok(Value::Object(array));
                    }
                    Value::Int(..) | Value::Float(..) | Value::Bool(_) => {
                        let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                        return Ok(Value::Object(array));
                    }
                    Value::Null | Value::Void => {
                        return Err(RunError::Native(
                            "TypeError: Cannot convert undefined or null to object".into(),
                        ));
                    }
                    _ => {}
                }
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = self.object_own_entries_impl(obj, false)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(entries.len());
                    for (index, (key, _)) in entries.iter().enumerate() {
                        let item = Value::Str(key.clone().into())
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        let _ = heap.array_set(array, index, item);
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectValues" => {
                match unbox_arg(args.first().ok_or(RunError::BadToken)?)? {
                    Value::Str(text) => {
                        let chars: Vec<String> = text.chars().map(|ch| ch.to_string()).collect();
                        let array = zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(chars.len());
                            for (index, ch) in chars.iter().enumerate() {
                                let value_any = Value::Str(ch.clone().into())
                                    .box_any(heap)
                                    .unwrap_or_else(zun_core::Any::null);
                                let item = if name == "objectEntries" {
                                    let key_any = Value::Str(index.to_string().into())
                                        .box_any(heap)
                                        .unwrap_or_else(zun_core::Any::null);
                                    let pair = heap.alloc_array(2);
                                    let _ = heap.array_set(pair, 0, key_any);
                                    let _ = heap.array_set(pair, 1, value_any);
                                    zun_core::Any::object(pair.addr() as usize)
                                } else {
                                    value_any
                                };
                                let _ = heap.array_set(array, index, item);
                            }
                            array
                        });
                        return Ok(Value::Object(array));
                    }
                    Value::Int(..) | Value::Float(..) | Value::Bool(_) => {
                        let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                        return Ok(Value::Object(array));
                    }
                    Value::Null | Value::Void => {
                        return Err(RunError::Native(
                            "TypeError: Cannot convert undefined or null to object".into(),
                        ));
                    }
                    _ => {}
                }
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = self.object_own_entries(obj)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(entries.len());
                    for (index, (_, value)) in entries.iter().enumerate() {
                        let item = value.box_any(heap).unwrap_or_else(zun_core::Any::null);
                        let _ = heap.array_set(array, index, item);
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectEntries" => {
                match unbox_arg(args.first().ok_or(RunError::BadToken)?)? {
                    Value::Str(text) => {
                        let chars: Vec<String> = text.chars().map(|ch| ch.to_string()).collect();
                        let array = zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(chars.len());
                            for (index, ch) in chars.iter().enumerate() {
                                let value_any = Value::Str(ch.clone().into())
                                    .box_any(heap)
                                    .unwrap_or_else(zun_core::Any::null);
                                let item = if name == "objectEntries" {
                                    let key_any = Value::Str(index.to_string().into())
                                        .box_any(heap)
                                        .unwrap_or_else(zun_core::Any::null);
                                    let pair = heap.alloc_array(2);
                                    let _ = heap.array_set(pair, 0, key_any);
                                    let _ = heap.array_set(pair, 1, value_any);
                                    zun_core::Any::object(pair.addr() as usize)
                                } else {
                                    value_any
                                };
                                let _ = heap.array_set(array, index, item);
                            }
                            array
                        });
                        return Ok(Value::Object(array));
                    }
                    Value::Int(..) | Value::Float(..) | Value::Bool(_) => {
                        let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                        return Ok(Value::Object(array));
                    }
                    Value::Null | Value::Void => {
                        return Err(RunError::Native(
                            "TypeError: Cannot convert undefined or null to object".into(),
                        ));
                    }
                    _ => {}
                }
                let obj = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let entries = self.object_own_entries(obj)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(entries.len());
                    for (index, (key, value)) in entries.iter().enumerate() {
                        let key_any = Value::Str(key.clone().into())
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        let value_any = value.box_any(heap).unwrap_or_else(zun_core::Any::null);
                        let pair = heap.alloc_array(2);
                        let _ = heap.array_set(pair, 0, key_any);
                        let _ = heap.array_set(pair, 1, value_any);
                        let _ = heap.array_set(array, index, zun_core::Any::object(pair.addr() as usize));
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectAssign" => {
                let Some(target) = object(args.first().ok_or(RunError::BadToken)?) else {
                    return Err(RunError::Native(
                        "TypeError: Cannot convert undefined or null to object".into(),
                    ));
                };
                // A Map, Set or RegExp target gains own expando properties.
                zun_gc::with_current_heap_mut(|heap| {
                    if !heap.is_dynamic_object(target)
                        && (heap.map_len(target).is_some() || heap.set_len(target).is_some() || heap.regex_source(target).is_some())
                    {
                        heap.ensure_dynamic_entry(target);
                    }
                });
                // A string source contributes its indexed characters.
                let string_source = match args.get(1) {
                    Some(Value::Str(text)) => Some(text.to_string()),
                    Some(Value::Any(a)) => zun_gc::with_current_heap(|heap| {
                        a.as_string().and_then(|h| heap.try_string(h).map(|s| s.to_string()))
                    }),
                    _ => None,
                };
                if let Some(text) = string_source {
                    for (index, ch) in text.chars().enumerate() {
                        let boxed = zun_gc::with_current_heap_mut(|heap| {
                            Value::Str(ch.to_string().into())
                                .box_any(heap)
                                .unwrap_or_else(zun_core::Any::null)
                        });
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set(target, index.to_string(), boxed);
                        });
                    }
                    return Ok(Value::Object(target));
                }
                // A nullish (or other primitive) source contributes nothing.
                let Some(source) = object(args.get(1).ok_or(RunError::BadToken)?) else {
                    return Ok(Value::Object(target));
                };
                let dynamic_source =
                    zun_gc::with_current_heap(|heap| heap.is_dynamic_object(source));
                if dynamic_source {
                    // Enumerable own keys in order; source getters and target
                    // setters run; a refused write is a TypeError (strict mode).
                    // Methods live in the function slots: copy those first (a data key
                    // of the same name below still wins).
                    let target_is_dynamic =
                        zun_gc::with_current_heap(|heap| heap.is_dynamic_object(target));
                    if target_is_dynamic {
                        let (named, symbolic) =
                            zun_gc::with_current_heap(|heap| heap.dynamic_function_entries(source));
                        zun_gc::with_current_heap_mut(|heap| {
                            for (name, handle) in named {
                                heap.dynamic_set_function(target, name, handle);
                            }
                            for (symbol, handle) in symbolic {
                                heap.dynamic_set_symbol_function(target, symbol, handle);
                            }
                        });
                    }
                    let keys = zun_gc::with_current_heap(|heap| heap.dynamic_keys(source))
                        .unwrap_or_default();
                    for key in keys {
                        let (enumerable, getter, is_function) = zun_gc::with_current_heap(|heap| {
                            (
                                heap.dynamic_is_enumerable(source, &key),
                                heap.dynamic_accessor_getter(source, &key),
                                heap.dynamic_get_function(source, &key).is_some(),
                            )
                        });
                        // Function-slot keys were copied above.
                        if !enumerable || is_function {
                            continue;
                        }
                        let boxed = if let Some(getter) = getter {
                            let got = self.invoke_function_value(getter, self.with_this_arg(getter, source, Vec::new()))?;
                            zun_gc::with_current_heap_mut(|heap| got.box_any(heap))
                                .unwrap_or_else(zun_core::Any::null)
                        } else {
                            zun_gc::with_current_heap(|heap| heap.dynamic_get(source, &key))
                                .unwrap_or_else(zun_core::Any::null)
                        };
                        let target_dynamic =
                            zun_gc::with_current_heap(|heap| heap.is_dynamic_object(target));
                        if !target_dynamic {
                            // A typed instance target: a declared field takes the
                            // value, any other key becomes an expando.
                            let typed = zun_gc::with_current_heap(|heap| {
                                heap.array_len(target).is_none() && target.type_rid() >= 1
                            });
                            if typed
                                && !self.instance_member_set(target, &key, &Value::Any(boxed))
                            {
                                zun_gc::with_current_heap_mut(|heap| {
                                    heap.ensure_instance_bag(target);
                                    heap.dynamic_set(target, key, boxed);
                                });
                            }
                            continue;
                        }
                        let (is_accessor, setter, has, writable, extensible) =
                            zun_gc::with_current_heap(|heap| {
                                (
                                    heap.dynamic_is_accessor(target, &key),
                                    heap.dynamic_accessor_setter(target, &key),
                                    heap.dynamic_has(target, &key),
                                    heap.dynamic_is_writable(target, &key),
                                    heap.dynamic_is_extensible(target),
                                )
                            });
                        if is_accessor {
                            match setter {
                                Some(setter) => {
                                    self.invoke_function_value(setter, self.with_this_arg(setter, target, vec![Value::Any(boxed)]))?;
                                }
                                None => {
                                    return Err(RunError::Native(format!(
                                        "TypeError: Cannot set property {key} which has only a getter"
                                    )));
                                }
                            }
                        } else if has && !writable {
                            return Err(RunError::Native(format!(
                                "TypeError: Cannot assign to read only property '{key}'"
                            )));
                        } else if !has && !extensible {
                            return Err(RunError::Native(format!(
                                "TypeError: Cannot add property {key}, object is not extensible"
                            )));
                        } else {
                            zun_gc::with_current_heap_mut(|heap| {
                                heap.dynamic_set(target, key, boxed);
                            });
                        }
                    }
                    // Own enumerable symbol-keyed properties.
                    let symbols = zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(source));
                    for symbol in symbols {
                        if !zun_gc::with_current_heap(|heap| heap.dynamic_is_symbol_enumerable(source, symbol)) {
                            continue;
                        }
                        let value = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get_symbol(source, symbol)
                        });
                        if let Some(value) = value {
                            zun_gc::with_current_heap_mut(|heap| {
                                heap.dynamic_set_symbol(target, symbol, value);
                            });
                        }
                    }
                    return Ok(Value::Object(target));
                }
                // An array source contributes its indexed elements.
                if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(source)) {
                    for index in 0..len {
                        zun_gc::with_current_heap_mut(|heap| {
                            if let Some(element) = heap.array_get(source, index) {
                                heap.dynamic_set(target, index.to_string(), element);
                            }
                        });
                    }
                    return Ok(Value::Object(target));
                }
                let entries = self.object_own_entries(source)?;
                for (key, value) in entries {
                    let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(target, key, boxed);
                    });
                }
                Ok(Value::Object(target))
            }
            "globalSet" => {
                // `global.set(name, value, readonly)` — register `name -> value`
                // on the realm global singleton, non-writable when `readonly`.
                let name = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                // Function values do not box to `Any`; they ride the global
                // object's function slot.
                if let Some(Value::Function(handle)) = args.get(1) {
                    let handle = *handle;
                    zun_gc::with_current_heap_mut(|heap| {
                        let global = heap.global_object();
                        heap.dynamic_set_function(global, name, handle);
                    });
                    return Ok(Value::Function(handle));
                }
                let value = any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let readonly = matches!(args.get(2), Some(Value::Bool(true)));
                zun_gc::with_current_heap_mut(|heap| {
                    let global = heap.global_object();
                    heap.dynamic_define(global, name, value, !readonly);
                });
                // An assignment expression evaluates to the assigned value.
                Ok(Value::Any(value))
            }
            "globalGet" => {
                // Read a module-level binding from the realm global singleton.
                let name = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let value = zun_gc::with_current_heap_mut(|heap| {
                    let global = heap.global_object();
                    heap.dynamic_get(global, &name)
                });
                if value.is_none() {
                    let function = zun_gc::with_current_heap_mut(|heap| {
                        { let global = heap.global_object(); heap.dynamic_get_function(global, &name) }
                    });
                    if let Some(handle) = function {
                        return Ok(Value::Function(handle));
                    }
                }
                Ok(value.map(Value::Any).unwrap_or(Value::Void))
            }
            "globalRead" => {
                let name = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let found = zun_gc::with_current_heap_mut(|heap| {
                    let global = heap.global_object();
                    if let Some(any) = heap.dynamic_get(global, &name) {
                        Some(Value::Any(any))
                    } else {
                        heap.dynamic_get_function(global, &name).map(Value::Function)
                    }
                });
                match found {
                    Some(value) => Ok(value),
                    None => Err(RunError::Native(format!("ReferenceError: {name} is not defined"))),
                }
            }
            "realmDescriptor" => {
                // Box a base-type metadata token into an opaque class-descriptor
                // `Any` (PLAN §3.1). The token arrives as a `number`.
                let token = match args.first().ok_or(RunError::BadToken)? {
                    Value::Float(value, _) => *value as u32,
                    Value::Int(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                Ok(Value::Any(zun_core::Any::class_descriptor(token)))
            }
            "classDescriptorToken" => {
                // Read a class descriptor's token back, or `-1` for anything that
                // is not a descriptor.
                let token = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_class_descriptor().map(|t| t as f64).unwrap_or(-1.0),
                    _ => -1.0,
                };
                Ok(Value::Float(token, zun_il::NumKind::F64))
            }
            "symbolNew" => {
                // `Symbol(description?)` mints a fresh, unique symbol. `Void`
                // (undefined) or `Null` means no description.
                let description = match args.first() {
                    Some(Value::Str(text)) => Some(text.to_string()),
                    Some(Value::Any(any)) => zun_gc::with_current_heap(|heap| {
                        any.as_string().and_then(|handle| heap.try_string(handle).map(|s| s.to_string()))
                    }),
                    _ => None,
                };
                let id = zun_gc::with_current_heap_mut(|heap| heap.symbol_create(description));
                Ok(Value::Any(zun_core::Any::symbol(id)))
            }
            "symbolFor" => {
                // `Symbol.for(key)` — the same symbol for the same key.
                let key = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(text) => text.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let id = zun_gc::with_current_heap_mut(|heap| heap.symbol_for(key));
                Ok(Value::Any(zun_core::Any::symbol(id)))
            }
            "symbolKeyFor" => {
                // `Symbol.keyFor(symbol)` — the registry key, or `undefined`.
                let id = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_symbol().ok_or(RunError::BadToken)?,
                    _ => return Err(RunError::BadToken),
                };
                Ok(zun_gc::with_current_heap(|heap| {
                    heap.symbol_key_for(id).map(|key| Value::Str(key.into()))
                })
                .unwrap_or(Value::Void))
            }
            "symbolWellKnown" => {
                // Materialize a well-known symbol value by its stable index.
                let index = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u64,
                    Value::Float(value, _) => *value as u64,
                    _ => return Err(RunError::BadToken),
                };
                let id = zun_core::SymbolId::from_raw(index);
                Ok(Value::Any(zun_core::Any::symbol(id)))
            }
            "symbolDescription" => {
                // `symbol.description` — the creation description, or `undefined`.
                let id = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_symbol().ok_or(RunError::BadToken)?,
                    _ => return Err(RunError::BadToken),
                };
                Ok(zun_gc::with_current_heap(|heap| {
                    heap.symbol_description(id).map(|text| Value::Str(text.into()))
                })
                .unwrap_or(Value::Void))
            }
            "objectFreeze" => {
                // `Object.freeze(primitive)` is the identity (non-objects have
                // no properties to freeze), returning the argument unchanged.
                let arg = args.first().ok_or(RunError::BadToken)?;
                let Some(obj) = object(arg) else {
                    return Ok(arg.clone());
                };
                if self.apply_nondynamic_integrity(obj, 3)? {
                    return Ok(Value::Object(obj));
                }
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_freeze(obj));
                Ok(Value::Object(obj))
            }
            "objectIsFrozen" => {
                // A primitive is always "frozen" (ECMAScript); otherwise a
                // dynamic object is frozen only when `Object.freeze` ran on it,
                // and a fixed-layout typed object has an immutable shape.
                let arg = args.first().ok_or(RunError::BadToken)?;
                let Some(obj) = object(arg) else {
                    return Ok(Value::Bool(true));
                };
                if let Some(level_len) = zun_gc::with_current_heap(|heap| {
                    heap.array_len(obj).map(|len| (heap.array_integrity(obj), len))
                }) {
                    return Ok(Value::Bool(level_len.0 >= 3));
                }
                if let Some(len) = crate::typed_arrays::element_len(obj) {
                    return Ok(Value::Bool(len == 0));
                }
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let frozen = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_frozen(obj))
                } else {
                    true
                };
                Ok(Value::Bool(frozen))
            }
            "objectCreate" => {
                let prototype = Self::normalize_prototype(
                    prototype_from_arg(args.first().ok_or(RunError::BadToken)?),
                    args.first().ok_or(RunError::BadToken)?,
                );
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_set_prototype(obj, prototype));
                Ok(Value::Object(obj))
            }
            "objectGetPrototypeOf" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let obj = match self.proxy_route(proxy, "getPrototypeOf") {
                    ProxyRoute::Trap(callback, target) => {
                        let result =
                            self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::null);
                        return Ok(Value::Any(boxed));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                if let Some(class_proto) = self.instance_class_prototype(obj) {
                    return Ok(Value::Any(zun_core::Any::object(class_proto.addr() as usize)));
                }
                let prototype = zun_gc::with_current_heap(|heap| heap.dynamic_prototype(obj));
                if prototype.is_none()
                    && zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some())
                {
                    let array_class = zun_gc::with_current_heap_mut(|heap| {
                        let global = heap.global_object();
                        heap.dynamic_get(global, "Array")
                    });
                    if let Some(descriptor) = array_class {
                        return Ok(Value::Any(descriptor));
                    }
                }
                // A dynamic object with no explicit link inherits from the
                // shared `Object.prototype`; an explicit `null` link
                // (`Object.create(null)`) stays null.
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                // `Object.prototype` itself is the end of every chain.
                if prototype.is_none() && is_dynamic && obj.addr() == Self::object_prototype_singleton().addr() {
                    return Ok(Value::Any(zun_core::Any::null()));
                }
                if prototype.is_none() && is_dynamic {
                    let root = Self::object_prototype_singleton();
                    return Ok(Value::Any(zun_core::Any::object(root.addr() as usize)));
                }
                // A `class X extends Promise` chain ends at the prelude wrapper
                // `__BuiltinPromise`; its prototype stands for the built-in's own.
                if let Some(proto_addr) = prototype.and_then(|p| p.as_object()) {
                    let count = self.module.tables.type_defs.len();
                    for rid in 1..=count as u32 {
                        let Some(def) = self.module.tables.type_defs.get(rid as usize - 1) else { continue };
                        let Some(builtin) = self.module.strings.get(def.name).strip_prefix("__Builtin").map(str::to_owned) else {
                            continue;
                        };
                        if self.class_prototype_object(rid).addr() as usize == proto_addr {
                            let descriptor = zun_gc::with_current_heap_mut(|heap| {
                                let global = heap.global_object();
                                heap.dynamic_get(global, &builtin)
                            });
                            if let Some(descriptor) = descriptor {
                                return Ok(Value::Any(descriptor));
                            }
                        }
                    }
                }
                Ok(Value::Any(prototype.unwrap_or_else(zun_core::Any::null)))
            }
            "objectJsonStringify" => self.json_stringify(args),
            "objectPrototype" => {
                let root = Self::object_prototype_singleton();
                Ok(Value::Any(zun_core::Any::object(root.addr() as usize)))
            }
            "regexpPrototype" => {
                let root = Self::regexp_prototype_singleton();
                Ok(Value::Any(zun_core::Any::object(root.addr() as usize)))
            }
            "objectSetPrototypeOf" => {
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let obj = match self.proxy_route(proxy, "setPrototypeOf") {
                    ProxyRoute::Trap(callback, target) => {
                        let prototype = args.get(1).ok_or(RunError::BadToken)?.clone();
                        self.invoke_proxy_trap(
                            callback,
                            vec![Value::Object(target), prototype],
                        )?;
                        return Ok(Value::Object(proxy));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                let prototype = Self::normalize_prototype(
                    prototype_from_arg(args.get(1).ok_or(RunError::BadToken)?),
                    args.get(1).ok_or(RunError::BadToken)?,
                );
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_set_prototype(obj, prototype));
                Ok(Value::Object(obj))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
