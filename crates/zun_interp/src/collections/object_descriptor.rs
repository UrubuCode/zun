//! Object property-descriptor, shape, reflection, and type-tag native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    /// Whether a realm class descriptor stands for an ECMAScript namespace object
    /// (`JSON`, `Atomics`) that the prelude models as a class of statics.
    fn descriptor_is_namespace(&self, token: u32) -> bool {
        if token & zun_natives::USER_CLASS_FLAG == 0 {
            return false;
        }
        let rid = (token & !zun_natives::USER_CLASS_FLAG) as usize;
        self.module
            .tables
            .type_defs
            .get(rid.wrapping_sub(1))
            .is_some_and(|def| {
                zun_natives::NAMESPACE_CLASSES.contains(&self.module.strings.get(def.name))
            })
    }

    #[allow(clippy::redundant_guards)] // guard reads clearer than the equivalent pattern
    pub(super) fn col_object_descriptor(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RunError> {
        if let Some(answer) = self.object_primitive_receiver(name, args) {
            return answer;
        }
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
                // The callback can evacuate the source, the result and its own closure
                // environment, and `element` is pushed into its group after the call, so it
                // rides in a rooted one-slot array (an `Any` is not a rooted carrier).
                zun_gc::root!(source = source);
                zun_gc::root!(obj = obj);
                zun_gc::root!(callback = callback);
                let holder = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(1));
                zun_gc::root!(holder = holder);
                for index in 0..length {
                    let element =
                        zun_gc::with_current_heap(|heap| heap.array_get(source.get(), index))
                            .ok_or(RunError::BadToken)?;
                    zun_gc::with_current_heap_mut(|heap| heap.array_set(holder.get(), 0, element));
                    let produced = self.invoke_callback(
                        callback.get(),
                        vec![
                            Value::Any(element),
                            Value::Any(zun_core::Any::float(index as f64)),
                        ],
                    )?;
                    let element = zun_gc::with_current_heap(|heap| heap.array_get(holder.get(), 0))
                        .ok_or(RunError::BadToken)?;
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
                            .dynamic_get(obj.get(), &key)
                            .and_then(|any| any.as_object())
                        {
                            Some(address) => unsafe { zun_gc::Obj::from_addr(address as *mut u64) },
                            None => {
                                let group = heap.alloc_array(0);
                                heap.dynamic_set(
                                    obj.get(),
                                    key.clone(),
                                    zun_core::Any::object(group.addr() as usize),
                                );
                                group
                            }
                        };
                        heap.array_push(group, element);
                    });
                }
                Ok(Value::Object(obj.get()))
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
                // Keep-alive: the result object and the interned keys are allocated while the
                // entries array is still being read.
                zun_gc::root!(array = array);
                let length = zun_gc::with_current_heap(|heap| heap.array_len(array.get()))
                    .ok_or(RunError::BadToken)?;
                let obj = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
                for index in 0..length {
                    let pair_any =
                        zun_gc::with_current_heap(|heap| heap.array_get(array.get(), index))
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
                            // A declared static, or a property a script stored
                            // on the class value (kept in its expando bag).
                            let stored = Self::class_expando_peek(rid | zun_natives::USER_CLASS_FLAG)
                                .is_some_and(|bag| {
                                    zun_gc::with_current_heap(|heap| heap.dynamic_has_own(bag, &key))
                                });
                            return Ok(Value::Bool(stored || self.class_has_own(rid, &key)));
                        }
                    }
                    // A built-in constructor owns its members and whatever a
                    // script stored on it.
                    if let Some(token) = descriptor.as_class_descriptor() {
                        if let Ok(Value::Str(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)
                        {
                            let stored = Self::class_expando_peek(token).is_some_and(|bag| {
                                zun_gc::with_current_heap(|heap| heap.dynamic_has_own(bag, &key))
                            });
                            return Ok(Value::Bool(
                                stored || self.builtin_ctor_own_descriptor(token, &key).is_some(),
                            ));
                        }
                    }
                    // A symbol key on a class value: a declared static symbol
                    // method, or a symbol property a script stored on it.
                    if let Some(token) = descriptor.as_class_descriptor() {
                        if let Ok(Value::Any(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)
                        {
                            if let Some(symbol) = key.as_symbol() {
                                let user_rid = self.user_class_rid(token);
                                let bag_token =
                                    user_rid.map_or(token, |rid| rid | zun_natives::USER_CLASS_FLAG);
                                let stored = Self::class_expando_peek(bag_token)
                                    .is_some_and(|bag| Self::bag_has_own_symbol(bag, symbol));
                                let declared = user_rid
                                    .is_some_and(|rid| self.class_declares_static_symbol(rid, symbol));
                                return Ok(Value::Bool(stored || declared));
                            }
                        }
                    }
                }
                // A function value owns `name` and `length`, plus the properties
                // a script stored on it (kept in its expando bag). `prototype`
                // is excluded: the bag also holds a lazily materialised one for
                // functions that are not constructors, and constructibility is
                // not tracked here.
                if let Ok(Value::Function(handle)) = unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    if let Ok(Value::Str(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?) {
                        let bag = crate::function_expando_bag(handle);
                        return Ok(Value::Bool(
                            matches!(key.as_ref(), "name" | "length")
                                || (key.as_ref() != "prototype"
                                    && zun_gc::with_current_heap(|heap| {
                                        heap.dynamic_has_own(bag, &key)
                                    })),
                        ));
                    }
                    // A symbol key: only what a script stored on the function.
                    if let Ok(Value::Any(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?) {
                        if let Some(symbol) = key.as_symbol() {
                            let bag = crate::function_expando_bag(handle);
                            return Ok(Value::Bool(Self::bag_has_own_symbol(bag, symbol)));
                        }
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
                    || heap.dynamic_symbol_setter(obj, symbol).is_some()
                        });
                        return Ok(Value::Bool(has));
                    }
                    other => self.dyn_to_string(&other)?,
                };
                // An array or a typed class instance that took an expando has a
                // property table too, but its elements and declared fields live
                // outside it: those answer through the own-entry walk below.
                let is_dynamic = zun_gc::with_current_heap(|heap| {
                    heap.is_dynamic_object(obj)
                        && !heap.is_instance_bag(obj)
                        && heap.array_len(obj).is_none()
                });
                let has = if is_dynamic {
                    zun_gc::with_current_heap(|heap| {
                        heap.dynamic_has(obj, &key)
                            || heap.dynamic_get_function(obj, &key).is_some()
                            || heap.dynamic_accessor_getter(obj, &key).is_some()
                            || heap.dynamic_accessor_setter(obj, &key).is_some()
                    })
                } else if key == "length"
                    && zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some())
                {
                    // An array's `length` is its own data property.
                    true
                } else {
                    self.object_own_entries_impl(obj, false)?
                        .iter()
                        .any(|(name, _)| name == &key)
                        || self.error_hidden_own(obj).iter().any(|(name, _)| name == &key)
                        || zun_gc::with_current_heap(|heap| heap.dynamic_has_own(obj, &key))
                };
                Ok(Value::Bool(has))
            }
            "objectIsPrototypeOf" => {
                let prototype =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let target =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                Ok(Value::Bool(self.proto_chain_contains(target, prototype)?))
            }
            "objectDefineProperty" | "reflectDefineProperty" => {
                let strict = name == "objectDefineProperty";
                let function_bag = match args.first().map(unbox_arg) {
                    Some(Ok(Value::Function(handle))) => Some(crate::function_expando_bag(handle)),
                    _ => None,
                };
                let proxy = match (args.first().and_then(Self::class_token_of), function_bag) {
                    (Some(token), _) => Self::class_expando_bag(token),
                    (None, Some(bag)) => bag,
                    (None, None) => object(args.first().ok_or(RunError::BadToken)?).ok_or_else(|| {
                        RunError::Native(
                            "TypeError: Object.defineProperty called on non-object".into(),
                        )
                    })?,
                };
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
                let obj = match self.proxy_route(proxy, "defineProperty")? {
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
                // `Object.create(proto, props)` marks itself with a third
                // argument: there, an absent or `undefined` property map means
                // "no properties", while `Object.defineProperties` throws for it
                // the way the specification's `ToObject(Properties)` does.
                // `null` throws in both, so only `undefined` is let through.
                let from_create =
                    matches!(args.get(2).map(unbox_arg), Some(Ok(Value::Bool(true))));
                let properties = args.get(1).ok_or(RunError::BadToken)?;
                if from_create && matches!(unbox_arg(properties), Ok(Value::Void) | Err(_)) {
                    return Ok(args.first().cloned().unwrap_or(Value::Void));
                }
                if matches!(unbox_arg(properties), Ok(Value::Void) | Ok(Value::Null)) {
                    return Err(RunError::Native(
                        "TypeError: Cannot convert undefined or null to object".into(),
                    ));
                }
                let descs = object(properties).ok_or(RunError::BadToken)?;
                // Each own enumerable key of `descs` maps a property name to a
                // descriptor object.
                let names = self.object_own_names(descs)?;
                // Gather (and validate) every descriptor before applying any, so
                // a throwing getter or a bad descriptor leaves the target clean.
                // The normalised descriptors are held in a rooted array.
                let holder = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                // `dynamic_member_get` and `normalize_descriptor` run user getters, which can
                // collect; the target, the descriptor map and the holder are read back through
                // guards.
                zun_gc::root!(obj = obj);
                zun_gc::root!(descs = descs);
                zun_gc::root!(holder = holder);
                let mut gathered: Vec<String> = Vec::new();
                for name in names {
                    if !zun_gc::with_current_heap(|heap| {
                        heap.dynamic_is_enumerable(descs.get(), &name)
                    }) {
                        continue;
                    }
                    let value = self.dynamic_member_get(descs.get(), &name)?;
                    let Some(desc) = crate::property_ops::descriptor_object(&value) else {
                        return Err(RunError::Native(
                            "TypeError: Property description must be an object".into(),
                        ));
                    };
                    let desc = self.normalize_descriptor(desc)?;
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.array_push(holder.get(), zun_core::Any::object(desc.addr() as usize))
                    });
                    gathered.push(name);
                }
                for (position, name) in gathered.into_iter().enumerate() {
                    let desc =
                        zun_gc::with_current_heap(|heap| heap.array_get(holder.get(), position))
                            .and_then(|any| any.as_object())
                            .map(|addr| unsafe { zun_gc::Obj::from_addr(addr as *mut u64) })
                            .ok_or(RunError::BadToken)?;
                    let obj = obj.get();
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
                let symbols =
                    zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(descs.get()));
                for symbol in symbols {
                    if !zun_gc::with_current_heap(|heap| {
                        heap.dynamic_is_symbol_enumerable(descs.get(), symbol)
                    }) {
                        continue;
                    }
                    let Some(any) =
                        zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get_symbol(descs.get(), symbol)
                        })
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
                        obj.get(),
                        &crate::property_ops::PropKey::Sym(symbol),
                        desc,
                    ) {
                        return Err(RunError::Native(
                            "TypeError: Cannot redefine property".into(),
                        ));
                    }
                }
                Ok(Value::Object(obj.get()))
            }
            "objectGetOwnPropertyDescriptor" => {
                // A function value owns `name` / `length` (read-only,
                // non-enumerable, configurable); script-defined keys live in
                // its expando bag.
                if let Some(Ok(Value::Function(handle))) = args.first().map(unbox_arg) {
                    let bag = crate::function_expando_bag(handle);
                    return match args.get(1).and_then(crate::property_ops::prop_key) {
                        Some(crate::property_ops::PropKey::Str(key)) => {
                            if zun_gc::with_current_heap(|heap| heap.dynamic_has_own(bag, &key)) {
                                return Ok(match self.build_descriptor(bag, &key)? {
                                    Some(d) => Value::Object(d),
                                    None => Value::Void,
                                });
                            }
                            if key == "prototype" {
                                if let Some(d) = self.coerce_ctor_prototype_descriptor(handle) {
                                    return Ok(Value::Object(d));
                                }
                            }
                            if let Some(constant) = self.coerce_ctor_constant(handle, &key) {
                                return Ok(Value::Object(Self::constant_descriptor_object(constant)));
                            }
                            if let Some(d) = self.coerce_ctor_static_descriptor(handle, &key) {
                                return Ok(Value::Object(d));
                            }
                            if key != "name" && key != "length" {
                                return Ok(Value::Void);
                            }
                            let value = self.function_meta(handle, &key).unwrap_or_else(|| {
                                if key == "name" {
                                    Value::Str("".into())
                                } else {
                                    Value::Float(0.0, zun_il::NumKind::F64)
                                }
                            });
                            let out = zun_gc::with_current_heap_mut(|heap| {
                                let out = heap.alloc_dynamic();
                                let any = value.box_any(heap).unwrap_or_else(zun_core::Any::void);
                                heap.dynamic_set(out, "value".to_owned(), any);
                                heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(false));
                                heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(false));
                                heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(true));
                                out
                            });
                            Ok(Value::Object(out))
                        }
                        _ => Ok(Value::Void),
                    };
                }
                // A property a script defined on a class value lives in its bag.
                if let Some(token) = args.first().and_then(Self::class_token_of) {
                    let bag = Self::class_expando_bag(token);
                    match args.get(1).and_then(crate::property_ops::prop_key) {
                        Some(crate::property_ops::PropKey::Str(key)) => {
                            if zun_gc::with_current_heap(|heap| heap.dynamic_has_own(bag, &key)) {
                                return Ok(match self.build_descriptor(bag, &key)? {
                                    Some(d) => Value::Object(d),
                                    None => Value::Void,
                                });
                            }
                        }
                        Some(crate::property_ops::PropKey::Sym(symbol)) => {
                            if let Some(d) = self.build_symbol_descriptor(bag, symbol) {
                                return Ok(Value::Object(d));
                            }
                            if self.user_class_rid(token).is_some() {
                                return Ok(Value::Void);
                            }
                        }
                        None => {}
                    }
                }
                // `<TypedArray>.BYTES_PER_ELEMENT`: a read-only, non-enumerable,
                // non-configurable data property of the constructor.
                if let Some(token) = args.first().and_then(Self::class_token_of) {
                    if token & zun_natives::USER_CLASS_FLAG == 0
                        && matches!(args.get(1), Some(Value::Str(k)) if k.as_ref() == "BYTES_PER_ELEMENT")
                    {
                        if let Some(kind) = self.typed_ctor_kind(token) {
                            let out = zun_gc::with_current_heap_mut(|heap| {
                                let out = heap.alloc_dynamic();
                                heap.dynamic_set(out, "value".to_owned(), zun_core::Any::float(kind.size() as f64));
                                heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(false));
                                heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(false));
                                heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(false));
                                out
                            });
                            return Ok(Value::Object(out));
                        }
                    }
                }
                // A built-in constructor owns `prototype`, `length` and `name`.
                if let Some(token) = args.first().and_then(Self::class_token_of) {
                    if let Some(crate::property_ops::PropKey::Str(key)) =
                        args.get(1).and_then(crate::property_ops::prop_key)
                    {
                        if let Some(d) = self.builtin_ctor_own_descriptor(token, &key) {
                            return Ok(Value::Object(d));
                        }
                        // A built-in constructor has no other own string-keyed property.
                        if token & zun_natives::USER_CLASS_FLAG == 0 {
                            return Ok(Value::Void);
                        }
                    }
                }
                // A built-in constructor owns an accessor `[Symbol.species]`
                // (the collection / array / promise / regexp family only).
                if let Some(Value::Any(any)) = args.first().and_then(|a| unbox_arg(a).ok()) {
                    if let Some(token) = any.as_class_descriptor() {
                        if self.user_class_rid(token).is_none() {
                            let is_species = matches!(
                                args.get(1).and_then(crate::property_ops::prop_key),
                                Some(crate::property_ops::PropKey::Sym(symbol))
                                    if symbol.get() == zun_core::WellKnownSymbol::Species.id().get()
                            );
                            let has_species = zun_gc::with_current_heap_mut(|heap| {
                                let global = heap.global_object();
                                ["Map", "Set", "Array", "Promise", "RegExp", "ArrayBuffer"]
                                    .iter()
                                    .any(|name| {
                                        heap.dynamic_get(global, name)
                                            .and_then(|a| a.as_class_descriptor())
                                            == Some(token)
                                    })
                            });
                            if is_species && has_species {
                                let out = zun_gc::with_current_heap_mut(|heap| {
                                    let out = heap.alloc_dynamic();
                                    heap.dynamic_set_function(
                                        out,
                                        "get",
                                        zun_core::FunctionHandle::new_builtin_method(
                                            crate::property_ops::BUILTIN_SPECIES_GETTER,
                                        ),
                                    );
                                    heap.dynamic_set(out, "set".to_owned(), zun_core::Any::void());
                                    heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(false));
                                    heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(true));
                                    out
                                });
                                return Ok(Value::Object(out));
                            }
                            if matches!(args.get(1), Some(Value::Any(_)))
                                && args.get(1).and_then(crate::property_ops::prop_key).is_some_and(
                                    |k| matches!(k, crate::property_ops::PropKey::Sym(_)),
                                )
                            {
                                return Ok(Value::Void);
                            }
                        }
                    }
                }
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
                            // A symbol key routes through the same proxy surface
                            // as a string key: a `getOwnPropertyDescriptor` trap
                            // sees the symbol, otherwise the descriptor is read
                            // from the proxied target (or the object itself).
                            let key_value = other.clone();
                            let obj =
                                match self.proxy_route(proxy, "getOwnPropertyDescriptor")? {
                                    ProxyRoute::Trap(callback, target) => {
                                        let result = self.invoke_proxy_trap(
                                            callback,
                                            vec![Value::Object(target), key_value],
                                        )?;
                                        return Ok(result);
                                    }
                                    ProxyRoute::Target(target) => target,
                                    ProxyRoute::NotProxy => proxy,
                                };
                            return Ok(match self.build_symbol_descriptor(obj, symbol) {
                                Some(descriptor) => Value::Object(descriptor),
                                None => Value::Void,
                            });
                        }
                        _ => return Err(RunError::BadToken),
                    },
                };
                let obj = match self.proxy_route(proxy, "getOwnPropertyDescriptor")? {
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
                        let mut names = self.class_own_names(rid);
                        if let Some(bag) = any
                            .as_class_descriptor()
                            .and_then(Self::class_expando_peek)
                        {
                            for key in zun_gc::with_current_heap(|heap| heap.dynamic_keys(bag))
                                .unwrap_or_default()
                            {
                                if !names.contains(&key) {
                                    names.push(key);
                                }
                            }
                        }
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
                // A built-in constructor with static methods owns `length`,
                // `name`, `prototype` and those methods.
                if let Some(Value::Any(any)) = args.first().and_then(|a| unbox_arg(a).ok()) {
                    if let Some(token) = any.as_class_descriptor() {
                        let mut statics = self.builtin_static_names(token);
                        statics.extend(self.builtin_constant_names(token));
                        if !statics.is_empty() {
                            let mut names = vec!["length", "name"];
                            if self.builtin_ctor_has_prototype(token) {
                                names.push("prototype");
                            }
                            names.extend(statics);
                            let array = zun_gc::with_current_heap_mut(|heap| {
                                let array = heap.alloc_array(names.len());
                                for (index, name) in names.iter().enumerate() {
                                    let item = Value::Str((*name).into())
                                        .box_any(heap)
                                        .unwrap_or_else(zun_core::Any::null);
                                    let _ = heap.array_set(array, index, item);
                                }
                                array
                            });
                            return Ok(Value::Object(array));
                        }
                    }
                }
                // A function value owns `length` and `name` plus whatever its
                // expando bag holds.
                if let Some(Ok(Value::Function(handle))) = args.first().map(unbox_arg) {
                    let bag = crate::function_expando_bag(handle);
                    let mut names = vec!["length".to_owned(), "name".to_owned()];
                    if self.coerce_ctor_name(handle).is_some() {
                        // A constructor owns `prototype`, then its statics, then
                        // its constants (the specification's own-key order).
                        names.push("prototype".to_owned());
                        names.extend(self.coerce_ctor_static_names(handle).into_iter().map(str::to_owned));
                        names.extend(self.coerce_ctor_constant_names(handle).into_iter().map(str::to_owned));
                    }
                    for key in self.object_own_names(bag)? {
                        if !names.contains(&key) {
                            names.push(key);
                        }
                    }
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
                let proxy =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // `Reflect.ownKeys(proxy)` consults the `ownKeys` trap; the trap
                // returns the key array directly.
                let obj = match self.proxy_route(proxy, "ownKeys")? {
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
                // `Reflect.ownKeys(primitive)` is a `TypeError`. The old
                // `object(..)` resolution rejected primitives as a side effect
                // of reading the symbol side; routing symbols through the
                // intrinsic lost that, and a string answered `0|1|length`.
                // Functions and class values are *not* primitives and must pass.
                let argument = args
                    .first()
                    .map(|arg| unbox_arg(arg))
                    .transpose()?
                    .unwrap_or(Value::Void);
                if matches!(
                    argument,
                    Value::Int(..)
                        | Value::Float(..)
                        | Value::Bool(_)
                        | Value::Str(_)
                        | Value::Null
                        | Value::Void
                ) {
                    return Err(RunError::BadToken);
                }
                let names = self.call_managed_collection("objectGetOwnPropertyNames", args)?;
                let Value::Object(names_array) = names else {
                    return Ok(names);
                };
                // The symbol side goes through the symbols intrinsic rather
                // than reading `dynamic_symbol_keys` off a heap object. That
                // intrinsic already knows the cases this one kept rejecting --
                // a class value and a function value are not heap objects, so
                // `Reflect.ownKeys(K)` threw while
                // `Object.getOwnPropertySymbols(K)` answered correctly. One
                // source of truth for "what symbol keys does this have".
                let symbol_array =
                    self.call_managed_collection("objectGetOwnPropertySymbols", args)?;
                let Value::Object(symbol_array) = symbol_array else {
                    return Ok(Value::Object(names_array));
                };
                zun_gc::root!(names_array = names_array);
                zun_gc::root!(symbol_array = symbol_array);
                zun_gc::with_current_heap_mut(|heap| {
                    let count = heap.array_len(symbol_array.get()).unwrap_or(0);
                    for index in 0..count {
                        if let Some(symbol) = heap.array_get(symbol_array.get(), index) {
                            let end = heap.array_len(names_array.get()).unwrap_or(0);
                            heap.array_insert(names_array.get(), end, symbol);
                        }
                    }
                });
                Ok(Value::Object(names_array.get()))
            }
            "objectGetOwnPropertySymbols" => {
                // `Object.getOwnPropertySymbols(obj)` — an array of the object's
                // own symbol keys (as `Any` symbol values), in insertion-stable
                // id order. A non-dynamic/typed object has no symbol keys.
                // A class constructor value is not a heap object: its symbol
                // statics are not modelled as own properties, so it reports none.
                if let Ok(Value::Any(any)) = unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    if let Some(token) = any.as_class_descriptor() {
                        // Only symbols a script defined on the class value are modelled.
                        let symbols = Self::class_expando_peek(token)
                            .map(|bag| zun_gc::with_current_heap(|heap| heap.dynamic_symbol_keys(bag)))
                            .unwrap_or_default();
                        let array = zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(symbols.len());
                            for (index, symbol) in symbols.iter().enumerate() {
                                let _ = heap.array_set(array, index, zun_core::Any::symbol(*symbol));
                            }
                            array
                        });
                        return Ok(Value::Object(array));
                    }
                }
                // A function value is not a heap object, and has no own symbol
                // keys: node answers an empty array where this used to reject
                // the argument outright.
                if matches!(
                    args.first().map(|arg| unbox_arg(arg)),
                    Some(Ok(Value::Function(_)))
                ) || matches!(args.first(), Some(Value::Function(_)))
                {
                    let array = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0));
                    return Ok(Value::Object(array));
                }
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
                zun_gc::with_current_heap_mut(|heap| {
                    crate::integrity::ensure_property_table(heap, obj);
                    heap.dynamic_seal(obj)
                });
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
                    // An object with no property table was never sealed
                    // (sealing gives it one) and can still gain own properties.
                    // A proxy keeps its historical answer: its integrity
                    // queries are not routed through the target here.
                    zun_gc::with_current_heap(|heap| heap.proxy_target(obj).is_some())
                };
                Ok(Value::Bool(sealed))
            }
            "objectPreventExtensions" => {
                // `Object.preventExtensions(primitive)` is the identity.
                let arg = args.first().ok_or(RunError::BadToken)?;
                let Some(proxy) = object(arg) else {
                    return Ok(arg.clone());
                };
                let obj = match self.proxy_route(proxy, "preventExtensions")? {
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
                zun_gc::with_current_heap_mut(|heap| {
                    crate::integrity::ensure_property_table(heap, obj);
                    heap.dynamic_prevent_extensions(obj)
                });
                Ok(Value::Object(obj))
            }
            "objectIsExtensible" => {
                // A primitive is never extensible (ECMAScript).
                let arg = args.first().ok_or(RunError::BadToken)?;
                let Some(proxy) = object(arg) else {
                    return Ok(Value::Bool(false));
                };
                let obj = match self.proxy_route(proxy, "isExtensible")? {
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
                    // An object with no property table never had extensions
                    // prevented: a typed class instance, Map, Set ... takes
                    // expando properties.
                    true
                };
                Ok(Value::Bool(extensible))
            }
            "objectPropertyIsEnumerable" => {
                let key = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(key) => key.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                // A class value: a property a script stored on it answers from
                // its expando bag; an initialised declared static field is
                // enumerable too. A function value answers from its bag.
                let first = args.first().ok_or(RunError::BadToken)?;
                if let Ok(Value::Any(descriptor)) = unbox_arg(first) {
                    if let Some(token) = descriptor.as_class_descriptor() {
                        let stored = Self::class_expando_peek(token).is_some_and(|bag| {
                            zun_gc::with_current_heap(|heap| heap.dynamic_is_enumerable(bag, &key))
                        });
                        let declared = self.user_class_rid(token).is_some_and(|rid| {
                            self.class_own_props(rid).iter().any(|(name, own)| {
                                name == &key
                                    && matches!(own, crate::property_ops::ClassOwn::Field(..))
                            })
                        });
                        return Ok(Value::Bool(stored || declared));
                    }
                }
                if let Ok(Value::Function(handle)) = unbox_arg(first) {
                    let bag = crate::function_expando_bag(handle);
                    return Ok(Value::Bool(zun_gc::with_current_heap(|heap| {
                        heap.dynamic_is_enumerable(bag, &key)
                    })));
                }
                let obj = object(first).ok_or(RunError::BadToken)?;
                // A typed class instance that took an expando still answers for
                // its declared fields, which live outside its property table.
                let is_dynamic = zun_gc::with_current_heap(|heap| {
                    heap.is_dynamic_object(obj)
                        && !heap.is_instance_bag(obj)
                        && heap.array_len(obj).is_none()
                });
                let enumerable = if is_dynamic {
                    zun_gc::with_current_heap(|heap| heap.dynamic_is_enumerable(obj, &key))
                } else {
                    // Typed declared fields are enumerable own properties.
                    self.object_own_entries_impl(obj, false)?
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
                    // A proxy is callable exactly when its (innermost) target is.
                    let mut obj = obj;
                    while let Some(inner) =
                        zun_gc::with_current_heap(|heap| heap.proxy_target(obj))
                    {
                        obj = inner;
                    }
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
                        // (except an ECMAScript namespace the prelude models as a
                        // class: `JSON`, `Atomics` are plain objects).
                        zun_core::AnyTag::ClassDescriptor => {
                            if self.descriptor_is_namespace(any.as_class_descriptor().unwrap_or(0))
                            {
                                "object"
                            } else {
                                "function"
                            }
                        }
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
                // A function stored in a container comes back as a function
                // *cell* (an object that holds the handle), and the cell was
                // observable here: `typeof` unboxes it and says "function"
                // while this branded it `Object`, so
                // `Object.prototype.toString.call(arr[0])` answered
                // `[object Object]` where node answers `[object Function]`.
                // The same unboxing the dynamic member read uses.
                let value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let value = zun_gc::with_current_heap(|heap| {
                    crate::collections::unbox_function_box(value, heap)
                });
                let tag: String = match &value {
                    Value::Void => "Undefined".into(),
                    Value::Null => "Null".into(),
                    Value::Int(..) | Value::Float(..) => "Number".into(),
                    Value::Bool(_) => "Boolean".into(),
                    Value::Str(_) => "String".into(),
                    Value::Function(_) => "Function".into(),
                    Value::Any(a) if a.as_symbol().is_some() => "Symbol".into(),
                    // A namespace the prelude models as a class of statics
                    // (`JSON`, `Atomics`, `crypto`) carries its own brand.
                    Value::Any(a)
                        if a.as_class_descriptor()
                            .is_some_and(|token| self.descriptor_is_namespace(token)) =>
                    {
                        let rid = (a.as_class_descriptor().unwrap_or(0)
                            & !zun_natives::USER_CLASS_FLAG) as usize;
                        self.module
                            .tables
                            .type_defs
                            .get(rid.wrapping_sub(1))
                            .map(|def| {
                                zun_natives::namespace_class_tag(self.module.strings.get(def.name))
                                    .to_string()
                            })
                            .unwrap_or_else(|| "Object".into())
                    }
                    Value::Object(obj) => {
                        // The tag read can run a user getter, which can move the
                        // object: read its address back afterwards.
                        zun_gc::root!(held = *obj);
                        let custom = self.string_tag_of(held.get())?;
                        let obj = held.get();
                        let class_tag = self
                            .module
                            .tables
                            .type_defs
                            .get((obj.type_rid() as usize).wrapping_sub(1))
                            .map(|def| self.module.strings.get(def.name))
                            .filter(|name| *name == "__AsyncGenerator")
                            .map(|_| "AsyncGenerator".to_string())
                            .or_else(|| {
                                (!zun_gc::with_current_heap(|heap| {
                                    heap.is_dynamic_object(obj)
                                        || heap.promise_state(obj).is_some()
                                        || heap.bigint_value(obj).is_some()
                                })
                                    && obj.type_rid() != 0
                                    && self.is_error_instance(obj))
                                .then(|| "Error".to_string())
                            })
                            // A `Date` instance (a prelude class) carries the Date brand.
                            .or_else(|| {
                                (!zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj))
                                    && obj.type_rid() != 0
                                    && self.class_chain_has_name(obj.type_rid(), "Date"))
                                .then(|| "Date".to_string())
                            })
                            // A Boolean/Number/String object, or the constructor's own
                            // `prototype` (itself such an object), carries that brand.
                            .or_else(|| self.wrapper_kind(obj).map(str::to_owned));
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
                                } else if let Some(kind) = heap.typed_array_element(obj) {
                                    kind.js_name()
                                } else if heap.data_view_buffer(obj).is_some() {
                                    "DataView"
                                } else if heap.buffer_len(obj).is_some() {
                                    "ArrayBuffer"
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
                    // A typed instance answers through its prototype chain, so a
                    // relinked class prototype (`Object.setPrototypeOf(C.prototype, ..)`)
                    // changes the answer; typed code that statically knows the
                    // class still folds at compile time.
                    Some(obj) if self.instance_class_prototype(obj).is_some() => {
                        let target = self.class_prototype_object(class_rid);
                        self.proto_chain_contains(obj, target)?
                    }
                    // A dynamic object (`Object.create(C.prototype)`) is an instance when the
                    // class prototype is on its chain.
                    Some(obj) => {
                        self.types.is_assignable(obj.type_rid(), class_rid)
                            || zun_gc::with_current_heap(|heap| heap.is_dynamic_object(obj))
                                && {
                                    let target = self.class_prototype_object(class_rid);
                                    self.proto_chain_contains(obj, target)?
                                }
                    }
                    None => false,
                };
                Ok(Value::Bool(found))
            }
            "hasPrivateBrand" => {
                // `#x in value` for an instance private. The second argument is
                // the declaring class's metadata TypeDef rid, the third the
                // private's name. A brand is installed on the instance by that
                // class's constructor, so the answer is the operand's run-time
                // *type* chain and nothing else: unlike `instanceofClass` above
                // it never consults the prototype chain, so a relinked instance
                // keeps its brand and `Object.create(C.prototype)` has none.
                // Any other object answers false; a primitive operand is the
                // TypeError `in` raises for a non-object.
                let class_rid = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    Value::Float(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                let primitive = match unbox_arg(args.first().ok_or(RunError::BadToken)?)? {
                    Value::Object(obj) => {
                        match zun_gc::with_current_heap(|heap| {
                            heap.bigint_value(obj).map(|big| big.to_string())
                        }) {
                            Some(big) => big,
                            // `instance_class_prototype` is the "is a typed
                            // class instance" test: it rejects dynamic objects,
                            // arrays, and core objects (a Promise, say) whose
                            // core type id merely aliases a module TypeDef rid.
                            None => {
                                return Ok(Value::Bool(
                                    self.instance_class_prototype(obj).is_some()
                                        && self.types.is_assignable(obj.type_rid(), class_rid),
                                ));
                            }
                        }
                    }
                    Value::Any(any) => match any.as_symbol() {
                        Some(symbol) => zun_gc::with_current_heap(|heap| {
                            format!(
                                "Symbol({})",
                                heap.symbol_description(symbol).unwrap_or_default()
                            )
                        }),
                        // Any other dynamic value that is not a primitive.
                        None => return Ok(Value::Bool(false)),
                    },
                    Value::Function(_) => return Ok(Value::Bool(false)),
                    other => other.display(),
                };
                let private_name = match args.get(2) {
                    Some(Value::Str(name)) => name.to_string(),
                    _ => String::new(),
                };
                Err(RunError::Native(format!(
                    "TypeError: Cannot use 'in' operator to search for '{private_name}' in {primitive}"
                )))
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

impl Interp<'_> {
    /// Whether the property table `bag` carries `symbol` as an own value,
    /// method or accessor.
    fn bag_has_own_symbol(bag: zun_gc::Obj, symbol: zun_core::SymbolId) -> bool {
        zun_gc::with_current_heap(|heap| {
            heap.dynamic_get_symbol(bag, symbol).is_some()
                || heap.dynamic_get_symbol_function(bag, symbol).is_some()
                || heap.dynamic_symbol_getter(bag, symbol).is_some()
                || heap.dynamic_symbol_setter(bag, symbol).is_some()
        })
    }

    /// Whether class `rid` itself declares a static method keyed by `symbol`.
    /// `class_symbol_methods` also records instance methods and the entries a
    /// subclass inherits, so the method must be static and must not be the
    /// very method the base class answers with.
    fn class_declares_static_symbol(&self, rid: u32, symbol: zun_core::SymbolId) -> bool {
        let Some(handle) = self.class_symbol_methods.get(&(rid, symbol.get())) else {
            return false;
        };
        let method_rid = handle.method_rid();
        let Some(def) = self.module.tables.type_defs.get((rid as usize).wrapping_sub(1)) else {
            return false;
        };
        if !def.base.is_null() && def.base.table() == Some(zun_metadata::TableKind::TypeDef) {
            let inherited = self
                .class_symbol_methods
                .get(&(def.base.rid(), symbol.get()))
                .is_some_and(|base| base.method_rid() == method_rid);
            if inherited {
                return false;
            }
        }
        self.module
            .tables
            .method_defs
            .get((method_rid as usize).wrapping_sub(1))
            .is_some_and(|method| {
                method.is_static && !self.module.strings.get(method.name).contains("@this")
            })
    }
}
