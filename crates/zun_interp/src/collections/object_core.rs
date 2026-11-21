//! Object/global/symbol enumeration, freeze, prototype, and realm native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_object_core(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RunError> {
        if let Some(answer) = self.object_primitive_receiver(name, args) {
            return answer;
        }
        match name {
            "objectForInKeys" => {
                // `for (k in obj)`: own enumerable keys first, then enumerable
                // keys inherited along the explicit prototype chain; a key
                // already seen (even a non-enumerable own one) shadows.
                // A primitive receiver: a string enumerates its character
                // indices; every other primitive (and null/undefined) has none.
                // A class value enumerates its enumerable statics, then the
                // properties scripts stored on it.
                if let Some(entries) =
                    self.class_value_entries(args.first().ok_or(RunError::BadToken)?, false)?
                {
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
                    return Ok(Value::Object(array));
                }
                let holder = match unbox_arg(args.first().ok_or(RunError::BadToken)?)? {
                    // A function or class value enumerates the properties
                    // scripts stored on it, kept in its expando bag.
                    Value::Function(handle) => Value::Object(crate::function_expando_bag(handle)),
                    Value::Any(any) if any.as_class_descriptor().is_some() => {
                        match any.as_class_descriptor().and_then(Self::class_expando_peek) {
                            Some(bag) => Value::Object(bag),
                            None => Value::Any(any),
                        }
                    }
                    other => other,
                };
                let obj = match holder {
                    Value::Object(obj) => obj,
                    other => {
                        let count = match &other {
                            Value::Str(text) => zun_core::wtf16::len16(text),
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
                let is_dynamic = zun_gc::with_current_heap(|heap| {
                    heap.is_dynamic_object(obj)
                        && heap.array_len(obj).is_none()
                        && !heap.is_instance_bag(obj)
                });
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                let mut keys: Vec<String> = Vec::new();
                if is_dynamic {
                    let mut current = obj;
                    for _ in 0..256 {
                        let own = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_keys(current)
                                .unwrap_or_default()
                                .into_iter()
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
                                .map(|address| unsafe {
                                    zun_gc::Obj::from_addr(address as *mut u64)
                                })
                        });
                        match next {
                            Some(next) => current = next,
                            None => break,
                        }
                    }
                } else {
                    keys = self
                        .object_own_entries_impl(obj, false)?
                        .into_iter()
                        .map(|(key, _)| key)
                        .collect();
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
                // A class value lists the enumerable properties scripts defined on it.
                let class_bag = args.first().and_then(Self::class_token_of);
                let function_bag = match args.first().map(unbox_arg) {
                    Some(Ok(Value::Function(handle))) => Some(crate::function_expando_bag(handle)),
                    _ => None,
                };
                let obj = match (class_bag, function_bag) {
                    (Some(token), _) => Self::class_expando_bag(token),
                    (None, Some(bag)) => bag,
                    (None, None) => object(args.first().ok_or(RunError::BadToken)?)
                        .ok_or(RunError::BadToken)?,
                };
                let entries = match self
                    .class_value_entries(args.first().ok_or(RunError::BadToken)?, false)?
                {
                    Some(entries) => entries,
                    None => self.object_own_entries_impl(obj, false)?,
                };
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
                // A function or class value lists the properties scripts
                // stored on it, kept in its expando bag.
                let obj = property_holder(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let entries = match self
                    .class_value_entries(args.first().ok_or(RunError::BadToken)?, true)?
                {
                    Some(entries) => entries,
                    None => self.object_own_entries(obj)?,
                };
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(entries.len());
                    for (index, (_, value)) in entries.iter().enumerate() {
                        // A function-valued property travels as its cell.
                        let item = value.box_any_cell(heap).unwrap_or_else(zun_core::Any::null);
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
                // A function or class value lists the properties scripts
                // stored on it, kept in its expando bag.
                let obj = property_holder(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let entries = match self
                    .class_value_entries(args.first().ok_or(RunError::BadToken)?, true)?
                {
                    Some(entries) => entries,
                    None => self.object_own_entries(obj)?,
                };
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(entries.len());
                    for (index, (key, value)) in entries.iter().enumerate() {
                        let key_any = Value::Str(key.clone().into())
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        let value_any =
                            value.box_any_cell(heap).unwrap_or_else(zun_core::Any::null);
                        let pair = heap.alloc_array(2);
                        let _ = heap.array_set(pair, 0, key_any);
                        let _ = heap.array_set(pair, 1, value_any);
                        let _ = heap.array_set(
                            array,
                            index,
                            zun_core::Any::object(pair.addr() as usize),
                        );
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectAssign" => {
                // ToObject on a primitive target: it receives the properties on its
                // wrapper object (`null` and `undefined` throw below).
                let first = args.first().ok_or(RunError::BadToken)?;
                let wrapped = self.primitive_wrapper_object(first)?;
                let Some(target) = wrapped.or_else(|| object(first)) else {
                    return Err(RunError::Native(
                        "TypeError: Cannot convert undefined or null to object".into(),
                    ));
                };
                // A Map, Set or RegExp target gains own expando properties.
                zun_gc::with_current_heap_mut(|heap| {
                    if !heap.is_dynamic_object(target)
                        && (heap.map_len(target).is_some()
                            || heap.set_len(target).is_some()
                            || heap.regex_source(target).is_some())
                    {
                        heap.ensure_dynamic_entry(target);
                    }
                });
                // A string source contributes its indexed characters.
                let string_source = match args.get(1) {
                    Some(Value::Str(text)) => Some(text.to_string()),
                    Some(Value::Any(a)) => zun_gc::with_current_heap(|heap| {
                        a.as_string()
                            .and_then(|h| heap.try_string(h).map(|s| s.to_string()))
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
                // A function or class value contributes the own enumerable
                // properties scripts stored on it (kept in its expando bag).
                let source_arg = args.get(1).ok_or(RunError::BadToken)?;
                let source_bag = match unbox_arg(source_arg) {
                    Ok(Value::Function(handle)) => Some(crate::function_expando_bag(handle)),
                    Ok(Value::Any(any)) => any.as_class_descriptor().map(Self::class_expando_bag),
                    _ => None,
                };
                // A class value: its enumerable statics, then what scripts
                // stored on it.
                if let Some(entries) = self.class_value_entries(source_arg, true)? {
                    for (key, value) in entries {
                        self.assign_store(target, key, value)?;
                    }
                    return Ok(Value::Object(target));
                }
                // A nullish (or other primitive) source contributes nothing.
                let Some(source) = source_bag.or_else(|| object(source_arg)) else {
                    return Ok(Value::Object(target));
                };
                // An array or a typed class instance that took an expando has
                // a property table too, but its elements and declared fields
                // live outside it: those sources are enumerated whole below.
                let dynamic_source = zun_gc::with_current_heap(|heap| {
                    heap.is_dynamic_object(source)
                        && heap.array_len(source).is_none()
                        && !heap.is_instance_bag(source)
                });
                if dynamic_source {
                    // Enumerable own keys in order; source getters and target
                    // setters run; a refused write is a TypeError (strict mode).
                    // String-keyed methods live in the function slots and are
                    // copied in key order by the loop below, so the target lists
                    // them where the source does. Symbol-keyed ones go first.
                    let target_is_dynamic =
                        zun_gc::with_current_heap(|heap| heap.is_dynamic_object(target));
                    if target_is_dynamic {
                        let (_, symbolic) =
                            zun_gc::with_current_heap(|heap| heap.dynamic_function_entries(source));
                        zun_gc::with_current_heap_mut(|heap| {
                            for (symbol, handle) in symbolic {
                                // Only an enumerable own symbol key is copied.
                                if heap.dynamic_is_symbol_enumerable(source, symbol) {
                                    heap.dynamic_set_symbol_function(target, symbol, handle);
                                }
                            }
                        });
                    }
                    let keys = zun_gc::with_current_heap(|heap| heap.dynamic_keys(source))
                        .unwrap_or_default();
                    for key in keys {
                        let (enumerable, getter, function) = zun_gc::with_current_heap(|heap| {
                            (
                                heap.dynamic_is_enumerable(source, &key),
                                heap.dynamic_accessor_getter(source, &key),
                                heap.dynamic_get_function(source, &key),
                            )
                        });
                        if !enumerable {
                            continue;
                        }
                        if let Some(function) = function {
                            self.assign_store(target, key, Value::Function(function))?;
                            continue;
                        }
                        let boxed = if let Some(getter) = getter {
                            let got = self.invoke_function_value(
                                getter,
                                self.with_this_arg(getter, source, Vec::new()),
                            )?;
                            zun_gc::with_current_heap_mut(|heap| got.box_any(heap))
                                .unwrap_or_else(zun_core::Any::null)
                        } else {
                            zun_gc::with_current_heap(|heap| heap.dynamic_get(source, &key))
                                .unwrap_or_else(zun_core::Any::null)
                        };
                        // A target whose own properties do not all live in a
                        // plain property table (a typed class instance, an
                        // array, a Map ...) takes the copy through the shared
                        // store: declared field, element, or expando.
                        let plain_target = zun_gc::with_current_heap(|heap| {
                            heap.is_dynamic_object(target)
                                && !heap.is_instance_bag(target)
                                && heap.array_len(target).is_none()
                        });
                        if !plain_target {
                            self.assign_store(target, key, Value::Any(boxed))?;
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
                                    self.invoke_function_value(
                                        setter,
                                        self.with_this_arg(setter, target, vec![Value::Any(boxed)]),
                                    )?;
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
                    let symbols =
                        zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(source));
                    for symbol in symbols {
                        if !zun_gc::with_current_heap(|heap| {
                            heap.dynamic_is_symbol_enumerable(source, symbol)
                        }) {
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
                // Any other source (an array, a typed class instance, a Map
                // with expandos ...) contributes its own enumerable entries: an
                // array's present elements, then its named properties.
                let entries = self.object_own_entries(source)?;
                for (key, value) in entries {
                    self.assign_store(target, key, value)?;
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
                // A name the realm does not hold yet is a module-level lexical
                // binding being created: generated global code must not see it.
                let fresh = zun_gc::with_current_heap_mut(|heap| {
                    let global = heap.global_object();
                    heap.dynamic_descriptor(global, &name).is_none()
                        && heap.dynamic_get_function(global, &name).is_none()
                });
                if fresh {
                    self.global_code.module_bindings.insert(name.clone());
                }
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
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let readonly = matches!(args.get(2), Some(Value::Bool(true)));
                zun_gc::with_current_heap_mut(|heap| {
                    let global = heap.global_object();
                    // Storing to a binding the realm already holds (a built-in
                    // global the program mirrors, such as a prelude function
                    // it reassigns) is an assignment: the property keeps its
                    // enumerable and configurable attributes rather than being
                    // redefined as an ordinary enumerable one.
                    match heap.dynamic_descriptor(global, &name) {
                        Some(zun_core::PropertyDescriptor::Data {
                            enumerable,
                            configurable,
                            ..
                        }) => {
                            heap.dynamic_define_data(
                                global,
                                name.clone(),
                                value,
                                !readonly,
                                enumerable,
                                configurable,
                            );
                        }
                        _ => {
                            heap.dynamic_define(global, name.clone(), value, !readonly);
                        }
                    }
                });
                if name == "Iterator" {
                    if let Some(address) = value.as_object() {
                        let namespace = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
                        Self::install_iterator_global(namespace);
                    }
                }
                // An assignment expression evaluates to the assigned value.
                Ok(Value::Any(value))
            }
            "globalDefineBuiltin" => {
                // A built-in global property: non-enumerable unless the fourth
                // argument says otherwise, and either an ordinary
                // writable/configurable binding or (`constant`) a frozen value
                // property such as `undefined`.
                let name = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let constant = matches!(args.get(2), Some(Value::Bool(true)));
                let enumerable = matches!(args.get(3), Some(Value::Bool(true)));
                // A function value rides the global object's function slot (as
                // in `globalSet`); it is writable and configurable like any
                // other built-in global, and enumerable only where Node says so.
                if let Some(Value::Function(handle)) = args.get(1) {
                    let handle = *handle;
                    zun_gc::with_current_heap_mut(|heap| {
                        let global = heap.global_object();
                        if enumerable {
                            heap.dynamic_set_function(global, name, handle);
                        } else {
                            heap.dynamic_set_method(global, name, handle);
                        }
                    });
                    return Ok(Value::Void);
                }
                let value =
                    any(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                zun_gc::with_current_heap_mut(|heap| {
                    let global = heap.global_object();
                    heap.dynamic_define_data(
                        global,
                        name.clone(),
                        value,
                        !constant,
                        enumerable,
                        !constant,
                    );
                });
                if name == "Iterator" {
                    if let Some(address) = value.as_object() {
                        let namespace = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
                        Self::install_iterator_global(namespace);
                    }
                }
                // `Object.prototype.constructor` is the realm's `Object` (a class
                // descriptor value): writable, configurable, not enumerable.
                if name == "Object" && value.as_class_descriptor().is_some() {
                    let proto = Self::object_prototype_singleton();
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_define_data(proto, "constructor", value, true, false, true);
                    });
                }
                // `Math` and `Reflect` carry their members as function values.
                if matches!(name.as_str(), "Math" | "Reflect") {
                    if let Some(address) = value.as_object() {
                        let namespace = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
                        zun_gc::with_current_heap_mut(|heap| {
                            crate::intrinsic_meta::install_namespace_members(heap, namespace, &name);
                        });
                    }
                }
                // An ECMAScript namespace the prelude models as a **class**
                // (`JSON`, `Atomics`) answered the right
                // `Object.prototype.toString` tag -- it is synthesized from the
                // class name -- but owned no `Symbol.toStringTag` property, so
                // `getOwnPropertyDescriptor` answered `undefined` where `Math`
                // and `Reflect`, which are plain objects, carry a real one.
                // Writable false, enumerable false, configurable true, as in the
                // specification.
                if matches!(name.as_str(), "JSON" | "Atomics") {
                    if let Some(token) = value.as_class_descriptor() {
                        let bag = Self::class_expando_bag(token);
                        let symbol = zun_core::WellKnownSymbol::ToStringTag.id();
                        zun_gc::with_current_heap_mut(|heap| {
                            let tag = zun_core::Any::string(heap.intern_string(&name));
                            heap.dynamic_define_symbol(bag, symbol, tag, false);
                            heap.dynamic_set_symbol_attributes(bag, symbol, Some(false), Some(true));
                        });
                    }
                }
                Ok(Value::Void)
            }
            "globalGet" => {
                // Read a module-level binding from the realm global singleton.
                let name = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                // `typeof name` in generated global code: a module binding is
                // out of scope there, so it reads as undeclared.
                if self.hidden_from_global_code(&name) {
                    return Ok(Value::Void);
                }
                let value = zun_gc::with_current_heap_mut(|heap| {
                    let global = heap.global_object();
                    heap.dynamic_get(global, &name)
                });
                if value.is_none() {
                    let function = zun_gc::with_current_heap_mut(|heap| {
                        let global = heap.global_object();
                        heap.dynamic_get_function(global, &name)
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
                if self.hidden_from_global_code(&name) {
                    return Err(RunError::Native(format!(
                        "ReferenceError: {name} is not defined"
                    )));
                }
                let found = zun_gc::with_current_heap_mut(|heap| {
                    let global = heap.global_object();
                    if let Some(any) = heap.dynamic_get(global, &name) {
                        Some(Value::Any(any))
                    } else {
                        heap.dynamic_get_function(global, &name)
                            .map(Value::Function)
                    }
                });
                match found {
                    Some(value) => Ok(value),
                    None => Err(RunError::Native(format!(
                        "ReferenceError: {name} is not defined"
                    ))),
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
                // `Symbol(description?)` mints a fresh, unique symbol. An
                // omitted or `undefined` description means none; anything else
                // is converted with ToString (`Symbol(42).description` is
                // `"42"`, `Symbol(null)` is `"null"`, and a Symbol throws).
                let description = match args.first().map(unbox_arg).transpose()? {
                    None | Some(Value::Void) => None,
                    Some(Value::Str(text)) => Some(text.to_string()),
                    Some(other) => Some(self.dyn_to_string(&other)?),
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
                    Value::Any(any) if any.as_symbol().is_some() => {
                        any.as_symbol().ok_or(RunError::BadToken)?
                    }
                    other => {
                        // Anything that is not a symbol is a TypeError
                        // (`Symbol.keyFor("x")`, `Symbol.keyFor()`).
                        let shown = match unbox_arg(other)? {
                            Value::Str(text) => text.to_string(),
                            Value::Int(number, _) => number.to_string(),
                            Value::Float(number, _) if number.fract() == 0.0 => {
                                (number as i64).to_string()
                            }
                            Value::Float(number, _) => number.to_string(),
                            Value::Bool(flag) => flag.to_string(),
                            Value::Null => "null".to_string(),
                            Value::Void => "undefined".to_string(),
                            _ => "#<Object>".to_string(),
                        };
                        return Err(RunError::Native(format!("TypeError: {shown} is not a symbol")));
                    }
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
                    heap.symbol_description(id)
                        .map(|text| Value::Str(text.into()))
                })
                .unwrap_or(Value::Void))
            }
            "objectFreeze" => {
                // `Object.freeze(primitive)` is the identity (non-objects have
                // no properties to freeze), returning the argument unchanged.
                //
                // A function is **not** a primitive: freezing one has to
                // record the level, and its expando bag is where
                // `objectIsExtensible` reads it from, so route to the bag and
                // return the function itself -- `Object.freeze(f) === f`.
                // Before this, freezing a function silently did nothing and
                // `Object.isExtensible(f)` still answered `true`.
                let raw = args.first().ok_or(RunError::BadToken)?;
                let callable = match unbox_arg(raw)? {
                    Value::Function(handle) => Some(handle),
                    _ => None,
                };
                let routed = match callable {
                    Some(handle) => Value::Object(crate::function_expando_bag(handle)),
                    None => raw.clone(),
                };
                let answer = |obj: zun_gc::Obj| match callable {
                    Some(handle) => Value::Function(handle),
                    None => Value::Object(obj),
                };
                let Some(obj) = object(&routed) else {
                    return Ok(raw.clone());
                };
                if self.apply_nondynamic_integrity(obj, 3)? {
                    return Ok(answer(obj));
                }
                // A typed class instance, Map, Set ... has no property table
                // until something is stored on it: give it one so the level
                // is recorded and later stores are refused.
                zun_gc::with_current_heap_mut(|heap| {
                    crate::integrity::ensure_property_table(heap, obj);
                    heap.dynamic_freeze(obj)
                });
                Ok(answer(obj))
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
                    heap.array_len(obj)
                        .map(|len| (heap.array_integrity(obj), len))
                }) {
                    return Ok(Value::Bool(level_len.0 >= 3));
                }
                if let Some(len) = crate::typed_arrays::element_len(obj) {
                    return Ok(Value::Bool(len == 0));
                }
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                // An object with no property table was never frozen (freezing
                // gives it one), and it can still gain own properties. A proxy
                // keeps its historical answer: its integrity queries are not
                // routed through the target here.
                let frozen = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_frozen(obj))
                } else {
                    zun_gc::with_current_heap(|heap| heap.proxy_target(obj).is_some())
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
                // A user class value: its prototype is the base class value.
                if let Ok(Value::Any(descriptor)) = unbox_arg(args.first().ok_or(RunError::BadToken)?)
                {
                    if let Some(rid) = descriptor
                        .as_class_descriptor()
                        .and_then(|t| self.user_class_rid(t))
                    {
                        let base = self
                            .module
                            .tables
                            .type_defs
                            .get((rid as usize).wrapping_sub(1))
                            .and_then(|def| {
                                (def.base.table() == Some(zun_metadata::TableKind::TypeDef)
                                    && !def.base.is_null())
                                .then(|| def.base.rid())
                            });
                        if let Some(base) = base {
                            // A class extending a built-in derives from the
                            // prelude `__Builtin<Name>` root, which stands for
                            // the built-in constructor itself.
                            let builtin = self
                                .module
                                .tables
                                .type_defs
                                .get(base as usize - 1)
                                .and_then(|def| {
                                    self.module.strings.get(def.name).strip_prefix("__Builtin")
                                })
                                .and_then(|name| {
                                    zun_gc::with_current_heap_mut(|heap| {
                                        let global = heap.global_object();
                                        heap.dynamic_get(global, name)
                                    })
                                });
                            if let Some(descriptor) = builtin {
                                return Ok(Value::Any(descriptor));
                            }
                            if self.user_class_rid(zun_natives::USER_CLASS_FLAG | base).is_some() {
                                return Ok(Value::Any(self.canonical_class_descriptor(base)));
                            }
                        }
                        // A base-less class constructor inherits `Function.prototype`.
                        let proto = Self::function_prototype_singleton();
                        return Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)));
                    }
                }
                // A function value or a built-in constructor inherits from
                // `Function.prototype`.
                if let Ok(first) = unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    let is_builtin_ctor = matches!(&first, Value::Any(any)
                        if any.as_class_descriptor().is_some_and(|t| self.user_class_rid(t).is_none()));
                    if let Value::Any(any) = &first {
                        if let Some(token) = any.as_class_descriptor() {
                            if self.user_class_rid(token).is_none()
                                && self.typed_ctor_kind(token).is_some()
                            {
                                let ctor = Self::typed_array_intrinsic_singleton();
                                return Ok(Value::Any(zun_core::Any::object(ctor.addr() as usize)));
                            }
                        }
                    }
                    // A built-in `Error` subclass constructor inherits `Error`.
                    if let Value::Any(any) = &first {
                        if let Some(token) = any.as_class_descriptor() {
                            let error_sub = zun_gc::with_current_heap_mut(|heap| {
                                crate::intrinsic_meta::builtin_class_name_in(heap, token)
                            })
                            .filter(|name| {
                                name != "Error" && crate::intrinsic_meta::is_error_family(name)
                            })
                            .and_then(|_| self.type_rid_of("Error"));
                            if let Some(error_rid) = error_sub {
                                return Ok(Value::Any(self.canonical_class_descriptor(error_rid)));
                            }
                        }
                    }
                    // A function relinked by `Object.setPrototypeOf`.
                    if let Value::Function(handle) = &first {
                        if let Some(link) = crate::function_prototype_override(*handle) {
                            return Ok(Value::Any(link));
                        }
                    }
                    // A generator, async or async-generator function inherits
                    // its own intrinsic (%GeneratorFunction.prototype% and
                    // siblings), which carries the brand, not
                    // `Function.prototype` directly.
                    if let Value::Function(handle) = &first {
                        if let Some(ctor_kind) = self.function_kind_ctor(*handle) {
                            let proto = self.function_kind_prototype(ctor_kind);
                            return Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)));
                        }
                    }
                    if is_builtin_ctor || matches!(first, Value::Function(_)) {
                        let proto = Self::function_prototype_singleton();
                        return Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)));
                    }
                }
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let obj = match self.proxy_route(proxy, "getPrototypeOf")? {
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
                    return Ok(Value::Any(zun_core::Any::object(
                        class_proto.addr() as usize
                    )));
                }
                if let Some(mode) = zun_gc::with_current_heap(|heap| heap.array_iterator_mode(obj)) {
                    let proto = Self::iterator_prototype_object(if mode == 3 { 2 } else { 1 });
                    return Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)));
                }
                // A generator object is linked to its function's `prototype`
                // object, which is where its chain starts; the shared
                // %GeneratorPrototype% is one level further up. The brand
                // below used to shortcut past that level.
                if let Some(address) = zun_gc::with_current_heap(|heap| {
                    heap.generator_state(obj)
                        .is_some()
                        .then(|| heap.dynamic_prototype(obj).and_then(|any| any.as_object()))
                        .flatten()
                }) {
                    return Ok(Value::Any(zun_core::Any::object(address)));
                }
                let setmap_kind = zun_gc::with_current_heap(|heap| {
                    if heap.generator_state(obj).is_some() {
                        Some(7u8)
                    } else if heap.set_iterator_done(obj).is_some() {
                        Some(5u8)
                    } else if heap.map_iterator_done(obj).is_some() {
                        Some(6u8)
                    } else {
                        None
                    }
                });
                if let Some(kind) = setmap_kind {
                    let proto = Self::iterator_prototype_object(kind);
                    return Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)));
                }
                let prototype = zun_gc::with_current_heap(|heap| heap.dynamic_prototype(obj));
                // A link to a function's expando bag stands for that function
                // (a function relinked onto a parent function).
                if let Some(parent) = prototype
                    .and_then(|link| link.as_object())
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
                    .and_then(crate::function_of_linked_bag)
                {
                    return Ok(Value::Any(parent));
                }
                if prototype.is_none() {
                    if let Some(kind) =
                        zun_gc::with_current_heap(|heap| heap.typed_array_element(obj))
                    {
                        if let Some(proto) = self.builtin_prototype_by_name(kind.js_name()) {
                            return Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)));
                        }
                    }
                    let builtin =zun_gc::with_current_heap(|heap| {
                        if heap.array_len(obj).is_some() {
                            Some("Array")
                        } else if heap.map_entries(obj).is_some() {
                            Some("Map")
                        } else if heap.set_values(obj).is_some() {
                            Some("Set")
                        } else if heap.is_weak_collection(obj) {
                            if heap.is_weak_set(obj) {
                                Some("WeakSet")
                            } else {
                                Some("WeakMap")
                            }
                        } else if heap.is_weak_ref(obj) {
                            Some("WeakRef")
                        // A DataView and the two buffer kinds are native storage
                        // too, with no link to walk: their prototype is the
                        // builtin's. (They answered something else entirely, so
                        // `Object.getPrototypeOf(view) === DataView.prototype`
                        // was false.)
                        } else if heap.data_view_buffer(obj).is_some() {
                            Some("DataView")
                        } else if heap.buffer_len(obj).is_some() {
                            if heap.buffer_shared(obj) == Some(true) {
                                Some("SharedArrayBuffer")
                            } else {
                                Some("ArrayBuffer")
                            }
                        } else {
                            None
                        }
                    });
                    if let Some(name) = builtin {
                        if let Some(proto) = self.builtin_prototype_by_name(name) {
                            return Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)));
                        }
                    }
                }
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
                if prototype.is_none()
                    && is_dynamic
                    && obj.addr() == Self::object_prototype_singleton().addr()
                {
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
                        let Some(def) = self.module.tables.type_defs.get(rid as usize - 1) else {
                            continue;
                        };
                        let Some(builtin) = self
                            .module
                            .strings
                            .get(def.name)
                            .strip_prefix("__Builtin")
                            .map(str::to_owned)
                        else {
                            continue;
                        };
                        if self.class_prototype_object(rid).addr() as usize == proto_addr {
                            // `RegExp.prototype` is its own singleton object.
                            if builtin == "RegExp" {
                                let root = Self::regexp_prototype_singleton();
                                return Ok(Value::Any(zun_core::Any::object(root.addr() as usize)));
                            }
                            if let Some(proto) = self.builtin_prototype_by_name(&builtin) {
                                return Ok(Value::Any(zun_core::Any::object(
                                    proto.addr() as usize,
                                )));
                            }
                        }
                    }
                }
                Ok(Value::Any(prototype.unwrap_or_else(zun_core::Any::null)))
            }
            "objectJsonStringify" => self.json_stringify(args),
            "objectRawJson" => {
                let text = match args.first() {
                    Some(Value::Str(text)) => text.clone(),
                    _ => return Err(RunError::BadToken),
                };
                Ok(zun_gc::with_current_heap_mut(|heap| {
                    let obj = heap.alloc_dynamic();
                    heap.dynamic_set_prototype(obj, Some(zun_core::Any::null()));
                    let boxed = zun_core::Any::string(heap.intern_string(text));
                    heap.dynamic_define_data(obj, "rawJSON", boxed, false, true, false);
                    heap.dynamic_freeze(obj);
                    heap.dynamic_mark_raw_json(obj);
                    Value::Object(obj)
                }))
            }
            "objectIsRawJson" => {
                let raw = match args.first() {
                    Some(Value::Object(obj)) => {
                        zun_gc::with_current_heap(|heap| heap.dynamic_is_raw_json(*obj))
                    }
                    Some(Value::Any(any)) => any.as_object().is_some_and(|addr| {
                        zun_gc::with_current_heap(|heap| {
                            // SAFETY: the address came from a live boxed object.
                            heap.dynamic_is_raw_json(unsafe {
                                zun_gc::Obj::from_addr(addr as *mut u64)
                            })
                        })
                    }),
                    _ => false,
                };
                Ok(Value::Bool(raw))
            }
            "objectPrototype" => {
                let root = Self::object_prototype_singleton();
                Ok(Value::Any(zun_core::Any::object(root.addr() as usize)))
            }
            "functionPrototype" => {
                let proto = Self::function_prototype_singleton();
                Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)))
            }
            "stringPrototype" => {
                let proto = Self::string_prototype_singleton();
                Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)))
            }
            "regexpPrototype" => {
                let root = Self::regexp_prototype_singleton();
                Ok(Value::Any(zun_core::Any::object(root.addr() as usize)))
            }
            "objectSetPrototypeOf" | "reflectSetPrototypeOf" => {
                let reflect = name == "reflectSetPrototypeOf";
                // From here on a class prototype may no longer inherit from its
                // base class's; see `inherited_member_detached`.
                crate::metadata::PROTOTYPE_RELINKED
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if !reflect {
                    // `Object.setPrototypeOf(O, proto)`: a nullish `O` and a `proto`
                    // that is neither an object nor null are TypeErrors; a primitive
                    // `O` is returned unchanged (RequireObjectCoercible, then no-op).
                    let target = unbox_arg(args.first().ok_or(RunError::BadToken)?)
                        .unwrap_or(Value::Void);
                    let proto = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)
                        .unwrap_or(Value::Void);
                    if matches!(target, Value::Void | Value::Null) {
                        return Err(self.proto_type_error(
                            "Object.setPrototypeOf called on null or undefined",
                        ));
                    }
                    let valid_proto = match &proto {
                        Value::Null | Value::Object(_) | Value::Function(_) => true,
                        Value::Any(any) => {
                            matches!(
                                any.tag(),
                                zun_core::AnyTag::Null | zun_core::AnyTag::Object
                            ) || any.as_class_descriptor().is_some()
                        }
                        _ => false,
                    };
                    if !valid_proto {
                        return Err(self.proto_type_error(
                            "Object prototype may only be an Object or null",
                        ));
                    }
                    let target_is_object = match &target {
                        Value::Object(_) | Value::Function(_) => true,
                        Value::Any(any) => {
                            any.as_object().is_some() || any.as_class_descriptor().is_some()
                        }
                        _ => false,
                    };
                    if !target_is_object {
                        return Ok(args[0].clone());
                    }
                }
                // Re-parenting the static side of a user class to another class:
                // recorded on the class's expando bag and consulted by static
                // lookup.
                if let Some(token) = args.first().and_then(Self::class_token_of) {
                    if self.user_class_rid(token).is_some() {
                        if let Some(parent) = args.get(1).and_then(Self::class_token_of) {
                            let bag = Self::class_expando_bag(token);
                            zun_gc::with_current_heap_mut(|heap| {
                                heap.dynamic_set_prototype(
                                    bag,
                                    Some(zun_core::Any::class_descriptor(parent)),
                                )
                            });
                            return Ok(if reflect {
                                Value::Bool(true)
                            } else {
                                args[0].clone()
                            });
                        }
                    }
                }
                // A function value relinked by a script
                // (`Object.setPrototypeOf(Derived, Base)`, the tsc `__extends`
                // helper). Two links are kept: the one `Object.getPrototypeOf`
                // answers (a parent function rides in its function cell), and
                // the expando bag's own link, which points at the parent's bag
                // so a static read inherits through it. `Function.prototype`
                // itself restores the default.
                if let Ok(Value::Function(target)) =
                    unbox_arg(args.first().ok_or(RunError::BadToken)?)
                {
                    let proto_arg = args.get(1).ok_or(RunError::BadToken)?;
                    // Everything that can allocate runs first, so no address
                    // read below is taken across a collection.
                    let _ = Self::function_prototype_singleton();
                    let _ = crate::function_expando_bag(target);
                    if let Ok(Value::Function(parent)) = unbox_arg(proto_arg) {
                        let _ = crate::function_expando_bag(parent);
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.function_cell(parent);
                        });
                    }
                    let Ok(Value::Function(handle)) = unbox_arg(&args[0]) else {
                        return Err(RunError::BadToken);
                    };
                    let (link, lookup) = match unbox_arg(proto_arg) {
                        Ok(Value::Function(parent)) => {
                            let parent_bag = crate::function_expando_bag(parent);
                            let cell = zun_gc::with_current_heap_mut(|heap| {
                                Value::Function(parent).box_any_cell(heap)
                            });
                            if let Some(cell) = cell {
                                crate::record_linked_function_bag(parent_bag, cell);
                            }
                            (cell, Some(zun_core::Any::object(parent_bag.addr() as usize)))
                        }
                        Ok(Value::Null | Value::Void) => {
                            (Some(zun_core::Any::null()), Some(zun_core::Any::null()))
                        }
                        _ => {
                            let link = match Self::class_token_of(proto_arg) {
                                Some(token) => Some(zun_core::Any::class_descriptor(token)),
                                None => prototype_from_arg(proto_arg),
                            };
                            (link, link)
                        }
                    };
                    let Some(link) = link else {
                        // Not an object or null: only `Reflect` reaches here.
                        return Err(self.proto_type_error(
                            "Object prototype may only be an Object or null",
                        ));
                    };
                    let default = Self::function_prototype_singleton();
                    let relinked = link.as_object() != Some(default.addr() as usize);
                    crate::set_function_prototype_override(handle, relinked.then_some(link));
                    let bag = crate::function_expando_bag(handle);
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set_prototype(bag, lookup.filter(|_| relinked))
                    });
                    return Ok(if reflect {
                        Value::Bool(true)
                    } else {
                        args[0].clone()
                    });
                }
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let obj = match self.proxy_route(proxy, "setPrototypeOf")? {
                    ProxyRoute::Trap(callback, target) => {
                        let prototype = args.get(1).ok_or(RunError::BadToken)?.clone();
                        let ok = self
                            .invoke_proxy_trap(callback, vec![Value::Object(target), prototype])?;
                        let ok = zun_gc::with_current_heap(|heap| is_truthy(&ok, heap));
                        if reflect {
                            return Ok(Value::Bool(ok));
                        }
                        if !ok {
                            return Err(RunError::Native(
                                "TypeError: 'setPrototypeOf' on proxy: trap returned falsish".into(),
                            ));
                        }
                        return Ok(Value::Object(proxy));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                // A typed instance keeps its relinked prototype in its expando
                // bag; an explicit `Object.prototype` link must stay explicit
                // there, because the default link would fall back to the class
                // prototype.
                if self.instance_class_prototype(obj).is_some() {
                    if let Some(link) = prototype_from_arg(args.get(1).ok_or(RunError::BadToken)?)
                    {
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.ensure_instance_bag(obj);
                            heap.dynamic_set_prototype(obj, Some(link));
                        });
                        return Ok(if reflect {
                            Value::Bool(true)
                        } else {
                            Value::Object(obj)
                        });
                    }
                }
                let prototype = Self::normalize_prototype(
                    prototype_from_arg(args.get(1).ok_or(RunError::BadToken)?),
                    args.get(1).ok_or(RunError::BadToken)?,
                );
                // OrdinarySetPrototypeOf: an unchanged link always succeeds; a
                // changed one is refused on a non-extensible object or when it
                // would close a prototype cycle.
                let root = Self::object_prototype_singleton();
                let refusal = zun_gc::with_current_heap(|heap| {
                    if !heap.is_dynamic_object(obj) {
                        return None;
                    }
                    let current = heap
                        .dynamic_prototype(obj)
                        .filter(|link| link.as_object() != Some(root.addr() as usize));
                    if current == prototype {
                        return None;
                    }
                    if !heap.dynamic_is_extensible(obj) {
                        return Some("#<Object> is not extensible");
                    }
                    let mut next = prototype.and_then(|link| link.as_object());
                    for _ in 0..100_000 {
                        let Some(address) = next else { break };
                        if address == obj.addr() as usize {
                            return Some("Cyclic __proto__ value");
                        }
                        let step = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
                        next = heap.dynamic_prototype(step).and_then(|link| link.as_object());
                    }
                    None
                });
                if let Some(message) = refusal {
                    if reflect {
                        return Ok(Value::Bool(false));
                    }
                    return Err(RunError::Native(format!("TypeError: {message}")));
                }
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_set_prototype(obj, prototype));
                if reflect {
                    return Ok(Value::Bool(true));
                }
                Ok(Value::Object(obj))
            }
            _ => Err(RunError::BadToken),
        }
    }

    /// `receiver.method(argument)` for an `Object.prototype` reflector
    /// (`hasOwnProperty`, `propertyIsEnumerable`, `isPrototypeOf`) on a dynamic
    /// receiver. A function the script installed under that name on the
    /// receiver or along its prototype chain takes the call, with the receiver
    /// as `this`; a non-callable own property of that name makes the call a
    /// TypeError; otherwise the built-in answers.
    pub(super) fn object_proto_call(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let receiver_arg = args.first().ok_or(RunError::BadToken)?;
        let method = match args.get(1).ok_or(RunError::BadToken)? {
            Value::Str(method) => method.to_string(),
            _ => return Err(RunError::BadToken),
        };
        let argument = args.get(2).ok_or(RunError::BadToken)?;
        if let Some(receiver) = object(receiver_arg) {
            let object_prototype = Self::object_prototype_singleton();
            let mut current = receiver;
            for _ in 0..256 {
                if current == object_prototype {
                    break;
                }
                let (function, shadowed, next) = zun_gc::with_current_heap(|heap| {
                    (
                        heap.dynamic_get_function(current, &method),
                        heap.dynamic_has_own(current, &method),
                        heap.dynamic_prototype(current)
                            .and_then(|link| link.as_object())
                            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
                    )
                });
                if let Some(function) = function {
                    // A built-in method value found on a prototype object is
                    // the built-in itself: let the intrinsic answer.
                    if function.builtin_method().is_some() {
                        break;
                    }
                    let argument = unbox_arg(argument).unwrap_or_else(|_| argument.clone());
                    return self.invoke_function_value(
                        function,
                        self.with_this_arg(function, receiver, vec![argument]),
                    );
                }
                if shadowed
                    && zun_gc::with_current_heap(|heap| !heap.dynamic_is_accessor(current, &method))
                {
                    return Err(RunError::Native(format!(
                        "TypeError: {method} is not a function"
                    )));
                }
                match next {
                    Some(next) => current = next,
                    None => break,
                }
            }
        }
        let native = match method.as_str() {
            "hasOwnProperty" => "objectHasOwn",
            "propertyIsEnumerable" => "objectPropertyIsEnumerable",
            _ => "objectIsPrototypeOf",
        };
        self.call_managed_collection(native, &[receiver_arg.clone(), argument.clone()])
    }

    /// Own enumerable string-keyed entries of a class value: the initialised
    /// static fields a user class declares, in declaration order, then the
    /// properties scripts stored on the class (kept in its expando bag).
    /// `Ok(None)` when `value` is not a class value. `read_values` false
    /// leaves every value `undefined`, as `Object.keys` never reads one.
    fn class_value_entries(
        &mut self,
        value: &Value,
        read_values: bool,
    ) -> Result<Option<Vec<(String, Value)>>, RunError> {
        let token = match unbox_arg(value) {
            Ok(Value::Any(any)) => any.as_class_descriptor(),
            _ => None,
        };
        let Some(token) = token else {
            return Ok(None);
        };
        let mut out: Vec<(String, Value)> = Vec::new();
        // A prelude class standing for a namespace object (`JSON`) holds
        // internal helper statics that are not properties of the namespace.
        if let Some(rid) = self
            .user_class_rid(token)
            .filter(|rid| self.namespace_class_members(*rid).is_none())
        {
            let fields: Vec<String> = self
                .class_own_props(rid)
                .into_iter()
                .filter(|(_, own)| matches!(own, crate::property_ops::ClassOwn::Field(..)))
                .map(|(name, _)| name)
                .collect();
            for name in fields {
                let value = if read_values {
                    self.class_static_read(rid, &name)?.unwrap_or(Value::Void)
                } else {
                    Value::Void
                };
                out.push((name, value));
            }
        }
        if let Some(bag) = Self::class_expando_peek(token) {
            for (key, value) in self.object_own_entries_impl(bag, read_values)? {
                // A value stored over a static method (`JSON.stringify = fn`)
                // replaces the value of a property that stays non-enumerable.
                let over_method = match self.user_class_rid(token) {
                    Some(rid) => self.class_static_method(rid, &key).is_some(),
                    None => self.builtin_ctor_own_descriptor(token, &key).is_some(),
                };
                if !over_method && !out.iter().any(|(name, _)| name == &key) {
                    out.push((key, value));
                }
            }
        }
        Ok(Some(out))
    }

    /// Store one property `Object.assign` (and object spread) copies onto
    /// `target`. A function value rides the function slot; a typed class
    /// instance takes a declared field in place and keeps anything else as an
    /// expando; a receiver with no property table yet gains one, so the copy
    /// lands instead of vanishing.
    fn assign_store(
        &mut self,
        target: zun_gc::Obj,
        key: String,
        value: Value,
    ) -> Result<(), RunError> {
        let (dynamic, typed) = zun_gc::with_current_heap(|heap| {
            (
                heap.is_dynamic_object(target),
                (!heap.is_dynamic_object(target) || heap.is_instance_bag(target))
                    && heap.array_len(target).is_none()
                    && target.type_rid() >= 1,
            )
        });
        let is_function = matches!(value, Value::Function(_));
        if typed {
            if let Some(message) = self.member_store_refusal(target, &key) {
                return Err(RunError::Native(message));
            }
        }
        if typed && !is_function && self.instance_member_set(target, &key, &value) {
            return Ok(());
        }
        // An array target takes a canonical index as an element.
        if !is_function {
            if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(target)) {
                if let Some(index) = key
                    .parse::<usize>()
                    .ok()
                    .filter(|index| index.to_string() == key && *index < u32::MAX as usize)
                {
                    zun_gc::with_current_heap_mut(|heap| {
                        let boxed = value.box_any(heap).unwrap_or_else(zun_core::Any::null);
                        if index >= len {
                            heap.array_resize(target, index + 1);
                        }
                        heap.array_set(target, index, boxed);
                    });
                    return Ok(());
                }
            }
        }
        if !dynamic {
            zun_gc::with_current_heap_mut(|heap| {
                if typed {
                    heap.ensure_instance_bag(target);
                } else {
                    heap.ensure_dynamic_entry(target);
                }
            });
        }
        if let Some(message) = zun_gc::with_current_heap(|heap| {
            crate::integrity::named_store_refusal(heap, target, &key)
        }) {
            return Err(RunError::Native(message));
        }
        zun_gc::with_current_heap_mut(|heap| match value {
            Value::Function(handle) => {
                heap.dynamic_set_function(target, key, handle);
            }
            other => {
                let boxed = other.box_any(heap).unwrap_or_else(zun_core::Any::null);
                heap.dynamic_set(target, key, boxed);
            }
        });
        Ok(())
    }
}
