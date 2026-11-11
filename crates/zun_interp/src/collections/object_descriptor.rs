//! Object property-descriptor, shape, reflection, and type-tag native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    #[allow(clippy::redundant_guards)] // guard reads clearer than the equivalent pattern
    pub(super) fn col_object_descriptor(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RunError> {
        match name {
            "templateObject" => {
                let cooked =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let raw =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // One template object per call site, cached on the global object.
                let site = match args.get(2) {
                    Some(Value::Int(n, _)) => *n,
                    Some(Value::Float(n, _)) => *n as i64,
                    _ => -1,
                };
                let key = format!("template#{site}");
                if site >= 0 {
                    let cached = zun_gc::with_current_heap_mut(|heap| {
                        {
                            let g = heap.global_object();
                            heap.dynamic_get(g, &key)
                        }
                        .and_then(|a| a.as_object())
                    });
                    if let Some(addr) = cached {
                        return Ok(Value::Object(unsafe {
                            zun_gc::Obj::from_addr(addr as *mut u64)
                        }));
                    }
                }
                let mask = match args.get(3) {
                    Some(Value::Str(mask)) => mask.to_string(),
                    _ => String::new(),
                };
                zun_gc::with_current_heap_mut(|heap| {
                    for (index, flag) in mask.chars().enumerate() {
                        if flag == '1' {
                            heap.array_set(cooked, index, zun_core::Any::void());
                        }
                    }
                    heap.ensure_dynamic_entry(cooked);
                    heap.dynamic_define_data(
                        cooked,
                        "raw".to_owned(),
                        zun_core::Any::object(raw.addr() as usize),
                        false,
                        false,
                        false,
                    );
                });
                self.call_managed_collection("objectFreeze", &[Value::Object(raw)])?;
                self.call_managed_collection("objectFreeze", &[Value::Object(cooked)])?;
                if site >= 0 {
                    zun_gc::with_current_heap_mut(|heap| {
                        let global = heap.global_object();
                        heap.dynamic_set(
                            global,
                            key,
                            zun_core::Any::object(cooked.addr() as usize),
                        );
                    });
                }
                Ok(Value::Object(cooked))
            }
            "objectGroupBy" => {
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let callback = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(source))
                    .ok_or(RunError::BadToken)?;
                let obj = zun_gc::with_current_heap_mut(|heap| {
                    let obj = heap.alloc_dynamic();
                    // The result has a null prototype.
                    heap.dynamic_set_prototype(obj, Some(zun_core::Any::null()));
                    obj
                });
                let roots = [source, obj];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                for index in 0..length {
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(roots[0], index))
                        .ok_or(RunError::BadToken)?;
                    let produced = self.invoke_callback(
                        callback,
                        vec![
                            Value::Any(element),
                            Value::Any(zun_core::Any::float(index as f64)),
                        ],
                    )?;
                    let key = match unbox_arg(&produced)? {
                        Value::Void => "undefined".to_string(),
                        Value::Null => "null".to_string(),
                        Value::Float(n, _) if n == 0.0 => "0".to_string(),
                        _ => {
                            let key_any =
                                zun_gc::with_current_heap_mut(|heap| produced.box_any(heap))
                                    .ok_or(RunError::BadToken)?;
                            zun_gc::with_current_heap(|heap| stringify_any(key_any, heap))
                                .ok_or(RunError::BadToken)?
                        }
                    };
                    zun_gc::with_current_heap_mut(|heap| {
                        let group = match heap
                            .dynamic_get(roots[1], &key)
                            .and_then(|any| any.as_object())
                        {
                            Some(address) => unsafe { zun_gc::Obj::from_addr(address as *mut u64) },
                            None => {
                                let group = heap.alloc_array(0);
                                heap.dynamic_set(
                                    roots[1],
                                    key.clone(),
                                    zun_core::Any::object(group.addr() as usize),
                                );
                                group
                            }
                        };
                        heap.array_push(group, element);
                    });
                }
                Ok(Value::Object(roots[1]))
            }
            "objectFromEntries" => {
                let not_iterable = || RunError::Native("TypeError: object is not iterable".into());
                let source =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or_else(not_iterable)?;
                // A non-array iterable (Map, Set, generator, user iterable) is
                // drained into an array of entries first.
                let array = if zun_gc::with_current_heap(|heap| heap.array_len(source).is_some()) {
                    source
                } else {
                    let target = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                    self.drive_iterable(source, target, |heap, array, value| {
                        heap.array_push(array, value);
                    })
                    .map_err(|_| not_iterable())?
                };
                let _array_root = [array];
                let _array_scope = unsafe { zun_gc::NativeRootScope::enter(&_array_root) };
                let array = _array_root[0];
                let length = zun_gc::with_current_heap(|heap| heap.array_len(array))
                    .ok_or(RunError::BadToken)?;
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                for index in 0..length {
                    let pair_any = zun_gc::with_current_heap(|heap| heap.array_get(array, index))
                        .ok_or(RunError::BadToken)?;
                    let Some(pair) = pair_any.as_object() else {
                        return Err(RunError::Native(
                            "TypeError: Iterator value is not an entry object".into(),
                        ));
                    };
                    let pair = unsafe { zun_gc::Obj::from_addr(pair as *mut u64) };
                    let (key_any, value_any) = zun_gc::with_current_heap(|heap| {
                        (heap.array_get(pair, 0), heap.array_get(pair, 1))
                    });
                    let key_any = key_any.ok_or(RunError::BadToken)?;
                    let value_any = value_any.ok_or(RunError::BadToken)?;
                    let key = zun_gc::with_current_heap(|heap| stringify_any(key_any, heap))
                        .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set(obj, key, value_any);
                    });
                }
                Ok(Value::Object(obj))
            }
            "objectIs" => {
                let left = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let right = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)?;
                Ok(Value::Bool(same_value(&left, &right)))
            }
            "objectHasOwn" => {
                // A user class value owns its statics (not its bases').
                if let Ok(Value::Any(descriptor)) = unbox_arg(args.first().ok_or(RunError::BadToken)?)
                {
                    if let Some(rid) = descriptor
                        .as_class_descriptor()
                        .and_then(|t| self.user_class_rid(t))
                    {
                        if let Ok(Value::Str(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)
                        {
                            return Ok(Value::Bool(self.class_has_own(rid, &key)));
                        }
                    }
                }
                // A function value owns `name` and `length`; `prototype` is only
                // modelled for constructible functions, which are not tracked here.
                if let Ok(Value::Function(_)) = unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    if let Ok(Value::Str(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?) {
                        return Ok(Value::Bool(matches!(key.as_ref(), "name" | "length")));
                    }
                }
                let obj =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                    Value::Str(key) => key.to_string(),
                    Value::Any(any) if any.as_symbol().is_some() => {
                        // A symbol key: own when the object carries it as a value,
                        // a method or an accessor.
                        let symbol = any.as_symbol().ok_or(RunError::BadToken)?;
                        let has = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get_symbol(obj, symbol).is_some()
                                || heap.dynamic_get_symbol_function(obj, symbol).is_some()
                                || heap.dynamic_symbol_getter(obj, symbol).is_some()
                        });
                        return Ok(Value::Bool(has));
                    }
                    other => self.dyn_to_string(&other)?,
                };
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let has = if is_dynamic {
                    zun_gc::with_current_heap(|heap| {
                        heap.dynamic_has(obj, &key)
                            || heap.dynamic_get_function(obj, &key).is_some()
                    })
                } else {
                    self.object_own_entries(obj)?
                        .iter()
                        .any(|(name, _)| name == &key)
                };
                Ok(Value::Bool(has))
            }
            "objectIsPrototypeOf" => {
                let prototype =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let prototype_addr = prototype.addr() as usize;
                let mut current = zun_gc::with_current_heap(|heap| heap.dynamic_prototype(target));
                let mut found = false;
                while let Some(link) = current {
                    match link.as_object() {
                        Some(addr) if addr == prototype_addr => {
                            found = true;
                            break;
                        }
                        Some(addr) => {
                            let next = unsafe { zun_gc::Obj::from_addr(addr as *mut u64) };
                            current =
                                zun_gc::with_current_heap(|heap| heap.dynamic_prototype(next));
                        }
                        None => break,
                    }
                }
                Ok(Value::Bool(found))
            }
            "objectDefineProperty" | "reflectDefineProperty" => {
                let strict = name == "objectDefineProperty";
                let proxy = object(args.first().ok_or(RunError::BadToken)?).ok_or_else(|| {
                    RunError::Native("TypeError: Object.defineProperty called on non-object".into())
                })?;
                let key_value =
                    match crate::property_ops::prop_key(args.get(1).ok_or(RunError::BadToken)?) {
                        Some(crate::property_ops::PropKey::Str(text)) => Value::Str(text.into()),
                        Some(crate::property_ops::PropKey::Sym(symbol)) => {
                            Value::Any(zun_core::Any::symbol(symbol))
                        }
                        // An object key is coerced with ToPropertyKey (string hint).
                        None => match object(args.get(1).ok_or(RunError::BadToken)?) {
                            Some(key_object) => {
                                match self.ordinary_to_primitive(key_object, "string")? {
                                    Some(Value::Str(text)) => Value::Str(text),
                                    _ => Value::Str("[object Object]".into()),
                                }
                            }
                            None => return Err(RunError::BadToken),
                        },
                    };
                let desc =
                    crate::property_ops::descriptor_object(args.get(2).ok_or(RunError::BadToken)?)
                        .ok_or_else(|| {
                            RunError::Native(
                                "TypeError: Property description must be an object".into(),
                            )
                        })?;
                let obj = match self.proxy_route(proxy, "defineProperty") {
                    ProxyRoute::Trap(callback, target) => {
                        let result = self.invoke_proxy_trap(
                            callback,
                            vec![Value::Object(target), key_value, Value::Object(desc)],
                        )?;
                        if strict {
                            return Ok(Value::Object(proxy));
                        }
                        return Ok(Value::Bool(zun_gc::with_current_heap(|heap| {
                            is_truthy(&result, heap)
                        })));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                self.define_property_native(
                    &[Value::Object(obj), key_value, Value::Object(desc)],
                    strict,
                )
            }
            "objectDefineProperties" => {
                let obj =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let descs =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // Each own enumerable key of `descs` maps a property name to a
                // descriptor object.
                let names = self.object_own_names(descs)?;
                // Gather (and validate) every descriptor before applying any, so
                // a throwing getter or a bad descriptor leaves the target clean.
                // The normalised descriptors are held in a rooted array.
                let holder = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                let roots = [obj, descs, holder];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                let mut gathered: Vec<String> = Vec::new();
                for name in names {
                    if !zun_gc::with_current_heap(|heap| {
                        heap.dynamic_is_enumerable(roots[1], &name)
                    }) {
                        continue;
                    }
                    let value = self.dynamic_member_get(roots[1], &name)?;
                    let Some(desc) = crate::property_ops::descriptor_object(&value) else {
                        return Err(RunError::Native(
                            "TypeError: Property description must be an object".into(),
                        ));
                    };
                    let desc = self.normalize_descriptor(desc)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.array_push(roots[2], zun_core::Any::object(desc.addr() as usize))
                    });
                    gathered.push(name);
                }
                for (position, name) in gathered.into_iter().enumerate() {
                    let desc = zun_gc::with_current_heap(|heap| heap.array_get(roots[2], position))
                        .and_then(|any| any.as_object())
                        .map(|addr| unsafe { zun_gc::Obj::from_addr(addr as *mut u64) })
                        .ok_or(RunError::BadToken)?;
                    let obj = roots[0];
                    if !self.define_own_property(
                        obj,
                        &crate::property_ops::PropKey::Str(name.clone()),
                        desc,
                    ) {
                        return Err(RunError::Native(format!(
                            "TypeError: Cannot redefine property: {name}"
                        )));
                    }
                }
                // Symbol-keyed descriptors of the map define symbol properties.
                let symbols = zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(roots[1]));
                for symbol in symbols {
                    if !zun_gc::with_current_heap(|heap| {
                        heap.dynamic_is_symbol_enumerable(roots[1], symbol)
                    }) {
                        continue;
                    }
                    let Some(any) =
                        zun_gc::with_current_heap(|heap| heap.dynamic_get_symbol(roots[1], symbol))
                    else {
                        continue;
                    };
                    let value = zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                        .unwrap_or(Value::Void);
                    let Some(desc) = crate::property_ops::descriptor_object(&value) else {
                        return Err(RunError::Native(
                            "TypeError: Property description must be an object".into(),
                        ));
                    };
                    let desc = self.normalize_descriptor(desc)?;
                    if !self.define_own_property(
                        roots[0],
                        &crate::property_ops::PropKey::Sym(symbol),
                        desc,
                    ) {
                        return Err(RunError::Native(
                            "TypeError: Cannot redefine property".into(),
                        ));
                    }
                }
                Ok(Value::Object(roots[0]))
            }
            "objectGetOwnPropertyDescriptor" => {
                if let Some(Value::Any(any)) = args.first().and_then(|a| unbox_arg(a).ok()) {
                    if let Some(rid) = any.as_class_descriptor().and_then(|t| self.user_class_rid(t)) {
                        if let Some(Value::Str(key)) = args.get(1) {
                            return Ok(match self.class_own_descriptor(rid, key) {
                                Some(d) => Value::Object(d),
                                None => Value::Void,
                            });
                        }
                    }
                }
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.to_string(),
                    other => match crate::property_ops::prop_key(other) {
                        Some(crate::property_ops::PropKey::Str(text)) => text,
                        Some(crate::property_ops::PropKey::Sym(symbol)) => {
                            return Ok(match self.build_symbol_descriptor(proxy, symbol) {
                                Some(descriptor) => Value::Object(descriptor),
                                None => Value::Void,
                            });
                        }
                        _ => return Err(RunError::BadToken),
                    },
                };
                let obj = match self.proxy_route(proxy, "getOwnPropertyDescriptor") {
                    ProxyRoute::Trap(callback, target) => {
                        let result = self.invoke_proxy_trap(
                            callback,
                            vec![Value::Object(target), Value::Str(key.clone().into())],
                        )?;
                        return Ok(result);
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                match self.build_descriptor(obj, &key)? {
                    Some(descriptor) => Ok(Value::Object(descriptor)),
                    None => Ok(Value::Void),
                }
            }
            "objectGetOwnPropertyDescriptors" => {
                // A primitive is read through its box: a string owns its code units and
                // `length`; a number or boolean owns nothing.
                match unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    Ok(Value::Str(text)) => {
                        let units: Vec<String> = text
                            .encode_utf16()
                            .map(|u| String::from_utf16_lossy(&[u]))
                            .collect();
                        let out = zun_gc::with_current_heap_mut(|heap| {
                            let out = heap.alloc_dynamic();
                            let mut entries: Vec<(String, zun_core::Any, bool)> = Vec::new();
                            for (index, unit) in units.iter().enumerate() {
                                let id = heap.intern_string(unit.as_str());
                                entries.push((index.to_string(), zun_core::Any::string(id), true));
                            }
                            entries.push((
                                "length".to_string(),
                                zun_core::Any::float(units.len() as f64),
                                false,
                            ));
                            for (name, value, enumerable) in entries {
                                let descriptor = heap.alloc_dynamic();
                                heap.dynamic_set(descriptor, "value", value);
                                heap.dynamic_set(
                                    descriptor,
                                    "writable",
                                    zun_core::Any::bool(false),
                                );
                                heap.dynamic_set(
                                    descriptor,
                                    "enumerable",
                                    zun_core::Any::bool(enumerable),
                                );
                                heap.dynamic_set(
                                    descriptor,
                                    "configurable",
                                    zun_core::Any::bool(false),
                                );
                                heap.dynamic_set(
                                    out,
                                    name,
                                    zun_core::Any::object(descriptor.addr() as usize),
                                );
                            }
                            out
                        });
                        return Ok(Value::Object(out));
                    }
                    Ok(Value::Bool(_)) | Ok(Value::Int(..)) | Ok(Value::Float(..)) => {
                        let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                        return Ok(Value::Object(out));
                    }
                    _ => {}
                }
                let obj =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let names = self.object_own_names(obj)?;
                let out = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                for name in names {
                    if let Some(descriptor) = self.build_descriptor(obj, &name)? {
                        let boxed = zun_core::Any::object(descriptor.addr() as usize);
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set(out, name, boxed);
                        });
                    }
                }
                let symbols = zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(obj));
                for symbol in symbols {
                    if let Some(descriptor) = self.build_symbol_descriptor(obj, symbol) {
                        let boxed = zun_core::Any::object(descriptor.addr() as usize);
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set_symbol(out, symbol, boxed);
                        });
                    }
                }
                Ok(Value::Object(out))
            }
            "objectGetOwnPropertyNames" => {
                if let Some(Value::Any(any)) = args.first().and_then(|a| unbox_arg(a).ok()) {
                    if let Some(rid) = any.as_class_descriptor().and_then(|t| self.user_class_rid(t)) {
                        let names = self.class_own_names(rid);
                        let array = zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(names.len());
                            for (index, name) in names.iter().enumerate() {
                                let item = Value::Str(name.clone().into())
                                    .box_any(heap)
                                    .unwrap_or_else(zun_core::Any::null);
                                let _ = heap.array_set(array, index, item);
                            }
                            array
                        });
                        return Ok(Value::Object(array));
                    }
                }
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // `Reflect.ownKeys(proxy)` consults the `ownKeys` trap; the trap
                // returns the key array directly.
                let obj = match self.proxy_route(proxy, "ownKeys") {
                    ProxyRoute::Trap(callback, target) => {
                        let result =
                            self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
                        // CreateListFromArrayLike: the result must be an
                        // array-like object whose entries are strings/symbols.
                        let list = object(&result).and_then(|list| {
                            zun_gc::with_current_heap(|heap| heap.array_len(list)).map(|n| (list, n))
                        });
                        let Some((list, count)) = list else {
                            return Err(RunError::Native(
                                "TypeError: proxy ownKeys trap result is not a list".to_string(),
                            ));
                        };
                        for index in 0..count {
                            let entry = zun_gc::with_current_heap(|heap| heap.array_get(list, index))
                                .ok_or(RunError::BadToken)?;
                            let ok = entry.as_symbol().is_some()
                                || matches!(
                                    unbox_arg(&Value::Any(entry)),
                                    Ok(Value::Str(_))
                                );
                            if !ok {
                                return Err(RunError::Native(
                                    "TypeError: proxy ownKeys trap result entry is not a string or symbol"
                                        .to_string(),
                                ));
                            }
                        }
                        return Ok(result);
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                let names = self.object_own_names(obj)?;
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(names.len());
                    for (index, name) in names.iter().enumerate() {
                        let item = Value::Str(name.clone().into())
                            .box_any(heap)
                            .unwrap_or_else(zun_core::Any::null);
                        let _ = heap.array_set(array, index, item);
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "reflectOwnKeys" => {
                // Own string keys followed by own symbol keys. A proxy's
                // `ownKeys` trap result is returned as is.
                let names = self.call_managed_collection("objectGetOwnPropertyNames", args)?;
                let Value::Object(names_array) = names else {
                    return Ok(names);
                };
                let target =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let symbols = zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(target));
                if symbols.is_empty() {
                    return Ok(Value::Object(names_array));
                }
                let roots = [names_array];
                let _scope = unsafe { zun_gc::NativeRootScope::enter(&roots) };
                zun_gc::with_current_heap_mut(|heap| {
                    for symbol in symbols {
                        let end = heap.array_len(roots[0]).unwrap_or(0);
                        heap.array_insert(roots[0], end, zun_core::Any::symbol(symbol));
                    }
                });
                Ok(Value::Object(roots[0]))
            }
            "objectGetOwnPropertySymbols" => {
                // `Object.getOwnPropertySymbols(obj)` — an array of the object's
                // own symbol keys (as `Any` symbol values), in insertion-stable
                // id order. A non-dynamic/typed object has no symbol keys.
                let obj =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let symbols = zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(obj));
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(symbols.len());
                    for (index, symbol) in symbols.iter().enumerate() {
                        let _ = heap.array_set(array, index, zun_core::Any::symbol(*symbol));
                    }
                    array
                });
                Ok(Value::Object(array))
            }
            "objectSeal" => {
                // `Object.seal(primitive)` is the identity (ECMAScript).
                let arg = args.first().ok_or(RunError::BadToken)?;
                let Some(obj) = object(arg) else {
                    return Ok(arg.clone());
                };
                if self.apply_nondynamic_integrity(obj, 2)? {
                    return Ok(Value::Object(obj));
                }
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_seal(obj));
                Ok(Value::Object(obj))
            }
            "objectIsSealed" => {
                let arg = args.first().ok_or(RunError::BadToken)?;
                let Some(obj) = object(arg) else {
                    return Ok(Value::Bool(true));
                };
                if let Some((level, len)) = zun_gc::with_current_heap(|heap| {
                    heap.array_len(obj)
                        .map(|len| (heap.array_integrity(obj), len))
                }) {
                    return Ok(Value::Bool(level >= 2 || (level >= 1 && len == 0)));
                }
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let sealed = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_sealed(obj))
                } else {
                    // A fixed-layout typed object has an immutable shape.
                    true
                };
                Ok(Value::Bool(sealed))
            }
            "objectPreventExtensions" => {
                // `Object.preventExtensions(primitive)` is the identity.
                let arg = args.first().ok_or(RunError::BadToken)?;
                let Some(proxy) = object(arg) else {
                    return Ok(arg.clone());
                };
                let obj = match self.proxy_route(proxy, "preventExtensions") {
                    ProxyRoute::Trap(callback, target) => {
                        self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
                        return Ok(Value::Object(proxy));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                if self.apply_nondynamic_integrity(obj, 1)? {
                    return Ok(Value::Object(obj));
                }
                zun_gc::with_current_heap_mut(|heap| heap.dynamic_prevent_extensions(obj));
                Ok(Value::Object(obj))
            }
            "objectIsExtensible" => {
                // A primitive is never extensible (ECMAScript).
                let arg = args.first().ok_or(RunError::BadToken)?;
                let Some(proxy) = object(arg) else {
                    return Ok(Value::Bool(false));
                };
                let obj = match self.proxy_route(proxy, "isExtensible") {
                    ProxyRoute::Trap(callback, target) => {
                        let result =
                            self.invoke_proxy_trap(callback, vec![Value::Object(target)])?;
                        return Ok(Value::Bool(matches!(result, Value::Bool(true))));
                    }
                    ProxyRoute::Target(target) => target,
                    ProxyRoute::NotProxy => proxy,
                };
                if let Some(level) = zun_gc::with_current_heap(|heap| {
                    heap.array_len(obj).map(|_| heap.array_integrity(obj))
                }) {
                    return Ok(Value::Bool(level == 0));
                }
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let extensible = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_extensible(obj))
                } else {
                    // Typed objects cannot gain fields.
                    false
                };
                Ok(Value::Bool(extensible))
            }
            "objectPropertyIsEnumerable" => {
                let obj =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let is_dynamic = zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj));
                let enumerable = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_enumerable(obj, &key))
                } else {
                    // Typed declared fields are enumerable own properties.
                    self.object_own_entries(obj)?
                        .iter()
                        .any(|(name, _)| name == &key)
                };
                Ok(Value::Bool(enumerable))
            }
            "captureStackTrace" => {
                // Runtime stack capture for `Error` construction (PLAN §3.1):
                // walk the live activation chain, innermost first, rendering
                // one frame line each. The managed `Error.stack` getter
                // prepends the `name: message` header, so this returns only the
                // frame lines.
                //
                // Fidelity: method names come from the module's `MethodDef`
                // table and the source location from the sequence-point debug
                // info, which records *byte spans* into the original source —
                // the interpreter has no source text, so a frame renders
                // `(<start>..<end>)` byte offsets rather than `file:line:col`.
                // The innermost frame is the one constructing the `Error`;
                // `captureStackTrace` itself is a `call.builtin` (no activation
                // of its own), so it does not appear. Only frames of the
                // current module are walked; a frame in another loaded module
                // runs on its own interpreter and is not visible here.
                Ok(Value::Str(self.render_stack_trace().into()))
            }
            "typeofAny" => {
                // Runtime `typeof` for an `any` operand. A managed reference is
                // refined with the heap: a BigInt payload is `"bigint"`, every
                // other object is `"object"`. `null` is `"object"` and
                // `undefined`/absent is `"undefined"`, as in JavaScript.
                let classify_object = |obj: zun_gc::Obj| {
                    if zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some()) {
                        "bigint"
                    } else if zun_gc::with_current_heap(|heap| {
                        heap.function_of_cell(obj).is_some()
                            || heap.dynamic_get_function(obj, "__boxedFn").is_some()
                    }) {
                        "function"
                    } else if zun_gc::with_current_heap(|heap| {
                        heap.is_dynamic_object(obj)
                            && heap
                                .dynamic_get(obj, "__classDesc")
                                .is_some_and(|d| d.as_class_descriptor().is_some())
                    }) {
                        // A `constructor` record stands for its class value.
                        "function"
                    } else {
                        "object"
                    }
                };
                let name = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(..) | Value::Float(..) => "number",
                    Value::Bool(_) => "boolean",
                    Value::Str(_) => "string",
                    Value::Function(_) => "function",
                    Value::Void => "undefined",
                    Value::Null => "object",
                    Value::Object(obj) => classify_object(*obj),
                    Value::Any(any) => match any.tag() {
                        zun_core::AnyTag::Int | zun_core::AnyTag::Float => "number",
                        zun_core::AnyTag::Bool => "boolean",
                        zun_core::AnyTag::String => "string",
                        zun_core::AnyTag::Symbol => "symbol",
                        // A realm class descriptor is a constructor value.
                        zun_core::AnyTag::ClassDescriptor => "function",
                        zun_core::AnyTag::Void => "undefined",
                        zun_core::AnyTag::Null => "object",
                        zun_core::AnyTag::Object => match any.as_object() {
                            Some(address) => classify_object(unsafe {
                                zun_gc::Obj::from_addr(address as *mut u64)
                            }),
                            None => "object",
                        },
                    },
                };
                Ok(Value::Str(name.into()))
            }
            "objectToStringTag" => {
                // `Object.prototype.toString.call(value)`: the builtin tag, or a
                // string `[Symbol.toStringTag]` own/inherited property.
                let value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let tag: String = match &value {
                    Value::Void => "Undefined".into(),
                    Value::Null => "Null".into(),
                    Value::Int(..) | Value::Float(..) => "Number".into(),
                    Value::Bool(_) => "Boolean".into(),
                    Value::Str(_) => "String".into(),
                    Value::Function(_) => "Function".into(),
                    Value::Any(a) if a.as_symbol().is_some() => "Symbol".into(),
                    Value::Object(obj) => {
                        let obj = *obj;
                        let custom = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get_symbol(
                                obj,
                                zun_core::WellKnownSymbol::ToStringTag.id(),
                            )
                            .and_then(|any| {
                                match Value::unbox_any(any, heap) {
                                    Some(Value::Str(text)) => Some(text.to_string()),
                                    _ => None,
                                }
                            })
                        });
                        let class_tag = self
                            .module
                            .tables
                            .type_defs
                            .get((obj.type_rid() as usize).wrapping_sub(1))
                            .map(|def| self.module.strings.get(def.name))
                            .filter(|name| *name == "__AsyncGenerator")
                            .map(|_| "AsyncGenerator".to_string());
                        if let Some(custom) = custom.or(class_tag) {
                            custom
                        } else {
                            zun_gc::with_current_heap(|heap| {
                                if heap.array_len(obj).is_some() {
                                    if heap.array_is_arguments(obj) {
                                        "Arguments"
                                    } else {
                                        "Array"
                                    }
                                } else if heap.array_iterator_done(obj).is_some() {
                                    if heap.array_iterator_mode(obj) == Some(3) {
                                        "String Iterator"
                                    } else {
                                        "Array Iterator"
                                    }
                                } else if heap.generator_state(obj).is_some() {
                                    "Generator"
                                } else if heap.regex_source(obj).is_some() {
                                    "RegExp"
                                } else if heap.bigint_value(obj).is_some() {
                                    "BigInt"
                                } else if heap.promise_state(obj).is_some() {
                                    "Promise"
                                } else if heap.map_entries(obj).is_some() {
                                    "Map"
                                } else if heap.set_values(obj).is_some() {
                                    "Set"
                                } else {
                                    "Object"
                                }
                            })
                            .into()
                        }
                    }
                    _ => "Object".into(),
                };
                Ok(Value::Str(format!("[object {tag}]").into()))
            }
            "instanceofClass" => {
                // Runtime `instanceof` for an operand whose static type does not
                // fold the answer. The second argument is the right-hand class's
                // metadata TypeDef rid; walk the operand's runtime class
                // hierarchy against it. A non-object operand (number, string,
                // `null`, …) and a plain dynamic object are never an instance.
                let class_rid = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    Value::Float(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                // A typed class carrying a `[Symbol.hasInstance]` method decides
                // membership itself: call it with the operand (boxed to `any`)
                // and return `ToBoolean` of the result.
                if let Some(&handler) = self
                    .class_symbol_methods
                    .get(&(class_rid, zun_core::WellKnownSymbol::HasInstance.id().get()))
                {
                    let operand = args.first().ok_or(RunError::BadToken)?.clone();
                    let boxed = match operand {
                        Value::Any(_) => operand,
                        other => zun_gc::with_current_heap_mut(|heap| {
                            other.box_any(heap).map(Value::Any)
                        })
                        .ok_or(RunError::BadToken)?,
                    };
                    let result = self.invoke_function_value(handler, vec![boxed])?;
                    let truthy = zun_gc::with_current_heap(|heap| is_truthy(&result, heap));
                    return Ok(Value::Bool(truthy));
                }
                let found = match object(args.first().ok_or(RunError::BadToken)?) {
                    // A BigInt payload object is a primitive: its core type id
                    // must not be read as a module class rid.
                    Some(obj)
                        if zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some()) =>
                    {
                        false
                    }
                    Some(obj) => self.types.is_assignable(obj.type_rid(), class_rid),
                    None => false,
                };
                Ok(Value::Bool(found))
            }
            // Binary ser/de PRIMITIVE (NUCLEUS #27). `serialize` inspects the
            // raw operand (so a function value is rejected cleanly, not boxed
            // away); `deserialize` reads a byte view and rebuilds the graph.
            "serialize" => self.serialize_value(args.first().ok_or(RunError::BadToken)?),
            "deserialize" => {
                let view =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                self.deserialize_value(view)
            }
            _ => Err(RunError::BadToken),
        }
    }
}
