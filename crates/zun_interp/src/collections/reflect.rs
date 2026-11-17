//! Reflect reflection and collection/symbol membership native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_reflect(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "collectionHas" => {
                // `key in collection`: prototype members of Map / Set / the weak
                // collections plus the Object.prototype members they inherit.
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Ok(Value::Bool(false)),
                };
                let (is_map, is_set) = zun_gc::with_current_heap(|heap| {
                    (
                        heap.map_len(object).is_some(),
                        heap.set_len(object).is_some(),
                    )
                });
                let own: &[&str] = if is_map {
                    &[
                        "size", "get", "set", "has", "delete", "clear", "forEach", "keys",
                        "values", "entries",
                    ]
                } else if is_set {
                    &[
                        "size",
                        "add",
                        "has",
                        "delete",
                        "clear",
                        "forEach",
                        "keys",
                        "values",
                        "entries",
                        "union",
                        "intersection",
                        "difference",
                        "symmetricDifference",
                        "isSubsetOf",
                        "isSupersetOf",
                        "isDisjointFrom",
                    ]
                } else {
                    &["get", "set", "add", "has", "delete"]
                };
                let inherited = [
                    "constructor",
                    "toString",
                    "toLocaleString",
                    "valueOf",
                    "hasOwnProperty",
                    "isPrototypeOf",
                    "propertyIsEnumerable",
                    "__proto__",
                    "__defineGetter__",
                    "__defineSetter__",
                    "__lookupGetter__",
                    "__lookupSetter__",
                ];
                let hit = own.contains(&name.as_str())
                    || inherited.contains(&name.as_str())
                    || zun_gc::with_current_heap(|heap| heap.dynamic_has(object, &name));
                Ok(Value::Bool(hit))
            }
            "symbolHas" => {
                // `sym in obj`: an own or inherited symbol-keyed property.
                let symbol = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                    Value::Any(any) => any.as_symbol().ok_or(RunError::BadToken)?,
                    _ => return Err(RunError::BadToken),
                };
                let mut cursor = object(args.first().ok_or(RunError::BadToken)?);
                if cursor.is_none() {
                    return Err(RunError::Native(
                        "TypeError: Cannot use 'in' operator to search in a non-object".into(),
                    ));
                }
                let mut depth = 0;
                while let Some(current) = cursor {
                    let found = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get_symbol(current, symbol).is_some()
                            || heap.dynamic_get_symbol_function(current, symbol).is_some()
                            || heap.dynamic_symbol_getter(current, symbol).is_some()
                    || heap.dynamic_symbol_setter(current, symbol).is_some()
                    });
                    if found {
                        return Ok(Value::Bool(true));
                    }
                    cursor = self.symbol_chain_next(current);
                    depth += 1;
                    if depth > 64 {
                        break;
                    }
                }
                Ok(Value::Bool(false))
            }
            "reflectHas" => {
                // A symbol key: walk the prototype chain for an own or
                // inherited symbol-keyed property, mirroring `symbolHas`.
                if let Some(crate::property_ops::PropKey::Sym(symbol)) =
                    args.get(1).and_then(crate::property_ops::prop_key)
                {
                    let mut cursor = object(args.first().ok_or(RunError::BadToken)?);
                    let mut depth = 0;
                    while let Some(current) = cursor {
                        let found = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get_symbol(current, symbol).is_some()
                                || heap.dynamic_get_symbol_function(current, symbol).is_some()
                                || heap.dynamic_symbol_getter(current, symbol).is_some()
                    || heap.dynamic_symbol_setter(current, symbol).is_some()
                        });
                        if found {
                            return Ok(Value::Bool(true));
                        }
                        cursor = self.symbol_chain_next(current);
                        depth += 1;
                        if depth > 64 {
                            break;
                        }
                    }
                    return Ok(Value::Bool(false));
                }
                if let Some(token) = args.first().and_then(Self::class_token_of) {
                    if let Some(Value::Str(key)) = args.get(1) {
                        let bag = Self::class_expando_bag(token);
                        // Own: a script-stored property (data, method or accessor),
                        // a declared static, or a built-in constructor member.
                        // Inherited: what `Function.prototype` and
                        // `Object.prototype` supply to every constructor.
                        let found = zun_gc::with_current_heap(|heap| heap.dynamic_has_own(bag, key))
                            || self
                                .user_class_rid(token)
                                .is_some_and(|rid| self.class_has_own(rid, key))
                            || (self.user_class_rid(token).is_none()
                                && self.builtin_ctor_own_descriptor(token, key).is_some())
                            || matches!(key.as_ref(), "apply" | "bind" | "call" | "constructor")
                            || crate::object_proto_mode(key).is_some();
                        return Ok(Value::Bool(found));
                    }
                }
                // A function value owns `name` and `length`, a `prototype` when it
                // is a constructor, its expando properties and (for the
                // coercion constructors) their constants; `Function.prototype`
                // and `Object.prototype` supply the rest.
                if let Some(Ok(Value::Function(handle))) = args.first().map(unbox_arg) {
                    if let Some(Ok(Value::Str(key))) = args.get(1).map(unbox_arg) {
                        let bag = crate::function_expando_bag(handle);
                        let constructor = self.coerce_ctor_name(handle).is_some()
                            || (handle.builtin_method().is_none()
                                && !handle.is_bound()
                                && handle.generator_method().is_none()
                                && !self
                                    .module
                                    .tables
                                    .method_defs
                                    .get((handle.method_rid() as usize).wrapping_sub(1))
                                    .is_some_and(|m| self.module.strings.get(m.name).starts_with('<')));
                        let hit = matches!(key.as_ref(), "name" | "length")
                            || (key.as_ref() == "prototype" && constructor)
                            || self.coerce_ctor_constant(handle, &key).is_some()
                            || self.coerce_ctor_static_kind(handle, &key).is_some()
                            || [
                                "apply", "bind", "call", "constructor", "toString", "toLocaleString",
                                "valueOf", "hasOwnProperty", "isPrototypeOf", "propertyIsEnumerable",
                                "__proto__",
                            ]
                            .contains(&key.as_ref())
                            || zun_gc::with_current_heap(|heap| heap.dynamic_has_own(bag, &key));
                        return Ok(Value::Bool(hit));
                    }
                }
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or_else(|| {
                    RunError::Native(
                        "TypeError: Cannot use 'in' operator to search in a non-object".into(),
                    )
                })?;
                if let Ok(key) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?) {
                    if let Some(hit) = self.typed_view_has(object, &key) {
                        return Ok(Value::Bool(hit));
                    }
                }
                // `ToPropertyKey`: a non-string, non-symbol key is coerced to its
                // string spelling, so `Reflect.has(o, undefined)` asks for
                // `"undefined"` rather than failing. `prop_key` already covers
                // the string and numeric forms; the other primitives spell
                // themselves.
                let key_arg = args.get(1).ok_or(RunError::BadToken)?;
                let owned = match crate::property_ops::prop_key(key_arg) {
                    Some(crate::property_ops::PropKey::Str(text)) => text,
                    _ => match unbox_arg(key_arg) {
                        Ok(Value::Void) => "undefined".to_string(),
                        Ok(Value::Null) => "null".to_string(),
                        Ok(Value::Bool(flag)) => flag.to_string(),
                        _ => return Err(RunError::BadToken),
                    },
                };
                let name = &owned;
                // An array answers for its elements (holes are absent) and `length`.
                if zun_gc::with_current_heap(|heap| heap.array_len(object).is_some()) {
                    let in_array = self.call_managed_collection("arrayHas", args)?;
                    // A method installed on the array lives in its function slot.
                    let own_named = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_has(object, name)
                            || heap.dynamic_get_function(object, name).is_some()
                    });
                    if matches!(in_array, Value::Bool(true)) || !own_named {
                        // A native-backed subclass instance also answers for
                        // the members its class prototype chain supplies.
                        if matches!(in_array, Value::Bool(false))
                            && Self::native_chain_has(object, name)
                        {
                            return Ok(Value::Bool(true));
                        }
                        return Ok(in_array);
                    }
                    return Ok(Value::Bool(true));
                }
                if Self::native_chain_has(object, name) {
                    return Ok(Value::Bool(true));
                }
                // A dynamic object answers for its whole prototype chain. A typed
                // instance carrying an expando table is not one: its declared
                // members answer below, after its own expandos.
                if zun_gc::with_current_heap(|heap| {
                    heap.is_dynamic_object(object) && !heap.is_instance_bag(object)
                }) {
                    return self.call_managed_collection("forInHas", args);
                }
                let dynamic_hit =
                    zun_gc::with_current_heap(|heap| heap.dynamic_has_own(object, name));
                if dynamic_hit {
                    return Ok(Value::Bool(true));
                }
                // A typed class instance: own fields, then methods / accessors
                // inherited through its class chain, then `constructor`.
                let typed = zun_gc::with_current_heap(|heap| {
                    (!heap.is_dynamic_object(object) || heap.is_instance_bag(object))
                        && heap.array_len(object).is_none()
                        && object.type_rid() >= 1
                });
                if typed {
                    let name = name.to_string();
                    if name == "constructor" {
                        return Ok(Value::Bool(true));
                    }
                    if self
                        .object_own_entries_impl(object, false)?
                        .iter()
                        .any(|(key, _)| key == &name)
                    {
                        return Ok(Value::Bool(true));
                    }
                    if self.error_hidden_own(object).iter().any(|(key, _)| key == &name) {
                        return Ok(Value::Bool(true));
                    }
                    // Every instance inherits the `Object.prototype` members.
                    let found = self.instance_method_rid(object, &name).is_some()
                        || self
                            .instance_method_rid(object, &format!("get_{name}"))
                            .is_some()
                        || crate::object_proto_mode(&name).is_some();
                    return Ok(Value::Bool(found));
                }
                Ok(Value::Bool(false))
            }
            "reflectDelete" => {
                // A symbol key: a symbol-keyed data property is configurable,
                // so deletion always reports success; remove it from the
                // object's symbol store when present.
                if let Some(crate::property_ops::PropKey::Sym(symbol)) =
                    args.get(1).and_then(crate::property_ops::prop_key)
                {
                    if let Some(target) = object(args.first().ok_or(RunError::BadToken)?) {
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_delete_symbol(target, symbol)
                        });
                    }
                    return Ok(Value::Bool(true));
                }
                // The indices and `length` of a `String` wrapper are
                // non-configurable.
                if let (Some(target), Some(Ok(Value::Str(key)))) =
                    (args.first().and_then(object), args.get(1).map(unbox_arg))
                {
                    if self.string_wrapper_owns(target, &key) {
                        return Ok(Value::Bool(false));
                    }
                }
                if let Some(token) = args.first().and_then(Self::class_token_of) {
                    if let Some(Value::Str(key)) = args.get(1) {
                        let bag = Self::class_expando_bag(token);
                        if zun_gc::with_current_heap(|heap| heap.dynamic_has_own(bag, key)) {
                            return Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                                heap.dynamic_delete(bag, key)
                            })));
                        }
                    }
                }
                if let Ok(Value::Any(descriptor)) = unbox_arg(args.first().ok_or(RunError::BadToken)?)
                {
                    if let Some(rid) = descriptor
                        .as_class_descriptor()
                        .and_then(|t| self.user_class_rid(t))
                    {
                        if let Ok(Value::Str(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)
                        {
                            return Ok(Value::Bool(self.class_delete_own(rid, &key)));
                        }
                    }
                }
                // A boxed string (modelled as the primitive): its indices and
                // `length` are non-configurable, any other key is deletable.
                if let Ok(Value::Str(text)) = unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    if let Ok(Value::Str(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?) {
                        let index = key.parse::<usize>().ok().filter(|i| i.to_string() == *key);
                        let fixed = key.as_ref() == "length"
                            || index.is_some_and(|i| i < zun_core::wtf16::len16(&text));
                        return Ok(Value::Bool(!fixed));
                    }
                }
                // A function's own `name`/`length` are configurable, `prototype` is not.
                if let Ok(Value::Function(_)) = unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    if let Ok(Value::Str(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?) {
                        return Ok(Value::Bool(key.as_ref() != "prototype"));
                    }
                }
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let name: zun_core::ZStr = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.clone(),
                    other => match crate::property_ops::prop_key(other) {
                        Some(crate::property_ops::PropKey::Str(text)) => text.as_str().into(),
                        _ => return Err(RunError::BadToken),
                    },
                };
                let name = &name;
                // A Proxy target: the `deleteProperty` trap decides (a `true` answer for
                // a non-configurable target key breaks the invariant); without a trap
                // the operation applies to the proxied target.
                let mut object = object;
                if let Some(target) = zun_gc::with_current_heap(|heap| heap.proxy_target(object)) {
                    if let Some(callback) =
                        zun_gc::with_current_heap(|heap| heap.proxy_trap(object, "deleteProperty"))
                    {
                        let result = self.invoke_proxy_trap(
                            callback,
                            vec![Value::Object(target), Value::Str(name.clone())],
                        )?;
                        let verdict =
                            !matches!(result, Value::Bool(false) | Value::Void | Value::Null);
                        if verdict
                            && !zun_gc::with_current_heap(|heap| {
                                heap.dynamic_can_delete(target, name)
                            })
                        {
                            return match self.new_error_object(
                                "TypeError",
                                &format!("'deleteProperty' on proxy: trap returned truish for property '{name}' which is non-configurable in the proxy target"),
                            ) {
                                Some(error) => Err(RunError::Thrown(Value::Object(error))),
                                None => Err(RunError::BadToken),
                            };
                        }
                        return Ok(Value::Bool(verdict));
                    }
                    object = target;
                }
                if name.as_ref() == "name" && self.error_delete_assigned_name(object) {
                    return Ok(Value::Bool(true));
                }
                // Arrays: an in-range index becomes a hole (unless sealed/frozen),
                // `length` is non-configurable, anything else is a plain key.
                if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(object)) {
                    if name.as_ref() == "length" {
                        return Ok(Value::Bool(false));
                    }
                    if let Ok(index) = name.parse::<usize>() {
                        if index >= len {
                            return Ok(Value::Bool(true));
                        }
                        return Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                            if heap.array_integrity(object) < 2 {
                                heap.array_set(object, index, zun_core::Any::hole());
                                true
                            } else {
                                false
                            }
                        })));
                    }
                }
                // A sealed or frozen array refuses to delete an existing element.
                if let Ok(index) = name.parse::<usize>() {
                    let verdict = zun_gc::with_current_heap(|heap| {
                        heap.array_len(object)
                            .map(|len| index >= len || heap.array_integrity(object) < 2)
                    });
                    if verdict == Some(false) {
                        return Ok(Value::Bool(false));
                    }
                }
                Ok(Value::Bool(zun_gc::with_current_heap_mut(|heap| {
                    // Deleting an absent key succeeds; a non-configurable one is refused.
                    if heap.dynamic_can_delete(object, name) {
                        heap.dynamic_delete(object, name);
                        true
                    } else {
                        false
                    }
                })))
            }
            "deleteStrict" => self.delete_strict(args),
            "reflectSetReceiver" => self.reflect_set_receiver(args),
            "reflectGetReceiver" => self.reflect_get_receiver(args),
            "reflectRequireObject" => {
                // `Reflect.*` on a primitive target is a TypeError. A string is let
                // through: `Object("s")` is still modelled as the primitive, so a
                // string target is how a boxed string reaches here.
                let first = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let primitive = match &first {
                    Value::Int(..) | Value::Float(..) | Value::Bool(_)
                    | Value::Null | Value::Void => true,
                    Value::Any(any) => matches!(any.tag(), zun_core::AnyTag::Symbol),
                    _ => false,
                };
                if primitive {
                    let method = match args.get(1).map(unbox_arg) {
                        Some(Ok(Value::Str(method))) => method.to_string(),
                        _ => "method".to_owned(),
                    };
                    return Err(RunError::Native(format!(
                        "TypeError: Reflect.{method} called on non-object"
                    )));
                }
                Ok(args[0].clone())
            }
            "reflectSet" => {
                // `Reflect.set(target, key, value)` returns a boolean success,
                // respecting writability (existing own data property) and
                // extensibility (adding a new property). Non-object targets and
                // failed stores return `false` rather than throwing.
                // A boxed string (modelled as the primitive) accepts new keys but its
                // indices and `length` are read-only.
                if let Ok(Value::Str(text)) = unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    if let Ok(Value::Str(key)) = unbox_arg(args.get(1).ok_or(RunError::BadToken)?) {
                        let index = key.parse::<usize>().ok().filter(|i| i.to_string() == *key);
                        let fixed = key.as_ref() == "length"
                            || index.is_some_and(|i| i < zun_core::wtf16::len16(&text));
                        return Ok(Value::Bool(!fixed));
                    }
                }
                self.reflect_set(args)
            }
            "reflectConstructNew" => {
                if let Some(Value::Function(handle)) =
                    args.first().and_then(|arg| unbox_arg(arg).ok())
                {
                    let array = object(args.get(1).ok_or(RunError::BadToken)?)
                        .ok_or(RunError::BadToken)?;
                    let values = self.read_array_values(array)?;
                    if let Some(ctor) = self.coerce_ctor_name(handle) {
                        return self.construct_coerce_wrapper(handle, ctor, values);
                    }
                    let proto_source = match args.get(2).map(unbox_arg) {
                        Some(Ok(Value::Function(target))) => target,
                        _ => handle,
                    };
                    if !self.function_is_constructor(proto_source) || !self.function_is_constructor(handle) {
                        return Err(RunError::Native("TypeError: value is not a constructor".into()));
                    }
                    return self.construct_ordinary_function(handle, proto_source, values);
                }
                let token = args
                    .first()
                    .and_then(Self::class_token_of)
                    .ok_or_else(|| RunError::Native("TypeError: value is not a constructor".into()))?;
                let array =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let values = self.read_array_values(array)?;
                // `Reflect.construct(Map|Set, args, UserSubclass)`: build a real
                // instance of the subclass (it owns the native storage and the
                // subclass methods) instead of a raw collection with a swapped
                // prototype.
                if self.user_class_rid(token).is_none() {
                    if let Some(sub) = args
                        .get(2)
                        .and_then(Self::class_token_of)
                        .and_then(|t| self.user_class_rid(t))
                    {
                        // A native-backed base (`Array`, a buffer) is the
                        // opposite case: the native value *is* the instance, so
                        // it falls through to the prototype swap below.
                        if let Some(wanted) = Self::builtin_ctor_global_name(token)
                            .filter(|name| !zun_natives::NATIVE_CARRIER_BASES.contains(name))
                        {
                            let base_name = format!("__Builtin{wanted}");
                            if self.class_chain_has_name(sub, &base_name) {
                                return self.construct_class_from_descriptor(
                                    zun_natives::USER_CLASS_FLAG | sub,
                                    values,
                                );
                            }
                        }
                    }
                }
                let built = if self.user_class_rid(token).is_some() || self.error_family_rid(token).is_some() {
                    self.construct_class_from_descriptor(token, values)?
                } else {
                    match self.construct_builtin_descriptor(token, array, values) {
                        Some(built) => built?,
                        None => {
                            return Err(RunError::Native(
                                "TypeError: value is not a constructor".into(),
                            ))
                        }
                    }
                };
                // The result inherits `newTarget.prototype` when that is an object.
                let new_target = args.get(2).cloned().unwrap_or(Value::Void);
                let proto: Option<zun_gc::Obj> = match Self::class_token_of(&new_target) {
                    Some(t) => match self.user_class_rid(t) {
                        Some(rid) => Some(self.class_prototype_object(rid)),
                        None => self
                            .error_family_rid(t)
                            .map(|rid| self.class_prototype_object(rid)),
                    },
                    None => match unbox_arg(&new_target) {
                        Ok(Value::Function(handle)) => {
                            self.ensure_function_prototype(handle);
                            Self::function_prototype_property(handle)
                        }
                        _ => None,
                    },
                };
                if let (Some(proto), Some(result)) = (proto, object(&built)) {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.ensure_instance_bag(result);
                        heap.dynamic_set_prototype(
                            result,
                            Some(zun_core::Any::object(proto.addr() as usize)),
                        )
                    });
                }
                Ok(built)
            }
            "reflectConstruct" => {
                if let Ok(Value::Function(handle)) =
                    unbox_arg(args.first().ok_or(RunError::BadToken)?)
                {
                    let array = object(args.get(1).ok_or(RunError::BadToken)?)
                        .ok_or(RunError::BadToken)?;
                    let values = self.read_array_values(array)?;
                    if let Some(ctor) = self.coerce_ctor_name(handle) {
                        return self.construct_coerce_wrapper(handle, ctor, values);
                    }
                    if !self.function_is_constructor(handle) {
                        return Err(RunError::Native("TypeError: value is not a constructor".into()));
                    }
                    return self.construct_ordinary_function(handle, handle, values);
                }
                let token = match args.first().and_then(Self::class_token_of) {
                    Some(token) => token,
                    None => {
                        return match self
                            .new_error_object("TypeError", "value is not a constructor")
                        {
                            Some(object) => Err(RunError::Thrown(Value::Object(object))),
                            None => Err(RunError::BadToken),
                        };
                    }
                };
                let array =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let values = self.read_array_values(array)?;
                if token & zun_natives::USER_CLASS_FLAG == 0
                    && self.error_family_rid(token).is_none()
                {
                    if let Some(built) = self.construct_builtin_descriptor(token, array, values.clone())
                    {
                        return built;
                    }
                }
                self.construct_class_from_descriptor(token, values)
            }
            _ => Err(RunError::BadToken),
        }
    }
}

impl<'a> Interp<'a> {
    /// The object stored in a function's `prototype` property, if any. A
    /// missing or primitive value yields `None`.
    pub(super) fn function_prototype_property(handle: zun_core::FunctionHandle) -> Option<zun_gc::Obj> {
        let bag = crate::function_expando_bag(handle);
        zun_gc::with_current_heap(|heap| heap.dynamic_get(bag, "prototype"))
            .and_then(|a| a.as_object())
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
    }

    /// `[[Construct]]` for an ordinary (non-class) function: allocate a fresh
    /// object inheriting `proto_source.prototype` (falling back to
    /// `Object.prototype`), call `handle` with it as `this`, and return the
    /// call's result when it is an object, else the fresh object.
    /// Whether `handle` can be the target of `new`. A class member (method,
    /// accessor, static method) and a generator are not constructors; a plain
    /// function, which owns no class membership, is.
    pub(crate) fn function_is_constructor(&self, handle: zun_core::FunctionHandle) -> bool {
        if handle.builtin_method().is_some() || handle.generator_method().is_some() {
            return false;
        }
        if handle.is_bound() {
            return true;
        }
        let rid = handle.method_rid();
        let owned_by_class = self
            .module
            .tables
            .type_defs
            .iter()
            .any(|def| rid >= def.method_start && rid < def.method_end);
        // A built-in function global (`eval`) is callable but not a constructor.
        if self.native_global_function_name(handle).is_some() {
            return false;
        }
        if owned_by_class {
            return false;
        }
        let Some(method) = self
            .module
            .tables
            .method_defs
            .get((rid as usize).wrapping_sub(1))
        else {
            return true;
        };
        let Ok(body) = read_body(&self.module.code, method.body_offset) else {
            return true;
        };
        let mut offset = 0;
        while offset < body.code.len() {
            let Ok((instruction, size)) = decode_one(&body.code[offset..]) else {
                break;
            };
            if instruction.opcode == Opcode::Yield {
                return false;
            }
            offset += size;
        }
        true
    }

    pub(super) fn construct_ordinary_function(
        &mut self,
        handle: zun_core::FunctionHandle,
        proto_source: zun_core::FunctionHandle,
        values: Vec<Value>,
    ) -> Result<Value, RunError> {
        // An arrow function and an object-literal method shorthand are not
        // constructible: both own no `prototype`, and `new` on either is a
        // TypeError in JS. Upstream's `018e5d3d` taught
        // `ensure_function_prototype` to withhold the `prototype` from them;
        // this is the other half, and it is the long-open "constructibility
        // flag" item -- without it `new ({ m() {} }).m()` ran the method body
        // and answered an object.
        if self.is_arrow_closure(handle) || self.is_method_closure(handle) {
            return Err(self.proto_type_error("value is not a constructor"));
        }
        // The `prototype` is created lazily; a later read must see the object
        // this instance inherits, so materialise it before inheriting.
        self.ensure_function_prototype(proto_source);
        let fresh = zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic());
        // The call below re-enters managed code, which can collect and move it.
        zun_gc::root!(fresh = fresh);
        let root = Self::object_prototype_singleton();
        if let Some(proto) = Self::function_prototype_property(proto_source) {
            if proto.addr() != root.addr() {
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set_prototype(
                        fresh.get(),
                        Some(zun_core::Any::object(proto.addr() as usize)),
                    )
                });
            }
        }
        let call_args = self.with_this_arg(handle, fresh.get(), values);
        let result = self.invoke_function_value(handle, call_args)?;
        // `[[Construct]]`: "if Type(result) is Object, return result" -- and a
        // *function* is an Object, so `function Maker() { return Point }` with
        // `new Maker()` answers `Point`, not the fresh instance. Only
        // `Value::Object` was honoured here, so a returned function or class
        // value was silently discarded and `new (new Maker())(5, 6)` then
        // failed with "value is not a constructor". A returned primitive,
        // `null` or `undefined` is ignored, which already worked.
        if matches!(result, Value::Function(_)) {
            return Ok(result);
        }
        if let Value::Any(any) = &result {
            if any.as_class_descriptor().is_some() {
                return Ok(result);
            }
        }
        let built = object(&result).unwrap_or_else(|| fresh.get());
        let boxed = zun_gc::with_current_heap_mut(|heap| Value::Object(built).box_any(heap))
            .unwrap_or_else(zun_core::Any::null);
        Ok(Value::Any(boxed))
    }
}

impl<'a> Interp<'a> {
    /// `value instanceof fn` for an ordinary function: whether `fn.prototype`
    /// appears on the dynamic prototype chain of `value`.
    pub(crate) fn function_instanceof(
        &mut self,
        value: &Value,
        handle: zun_core::FunctionHandle,
    ) -> Result<bool, RunError> {
        self.ensure_function_prototype(handle);
        let Some(target) = Self::function_prototype_property(handle) else {
            return Ok(false);
        };
        let Some(current) = unbox_arg(value).ok().as_ref().and_then(object) else {
            return Ok(false);
        };
        self.proto_chain_contains(current, target)
    }
}

impl<'a> Interp<'a> {
    /// `[[Construct]]` of the `String` / `Number` / `Boolean` constructors
    /// reached as the coercion function value `handle`: coerce the first
    /// argument through that function (the specified ToString / ToNumber /
    /// ToBoolean, with the specified no-argument default) and box it in the
    /// prelude wrapper class `new String(x)` builds, so the result carries
    /// `valueOf` and `toString` exactly like the lowered `new` form.
    pub(super) fn construct_coerce_wrapper(
        &mut self,
        handle: zun_core::FunctionHandle,
        ctor: &str,
        values: Vec<Value>,
    ) -> Result<Value, RunError> {
        let token = self
            .wrapper_class_token(ctor)
            .ok_or_else(|| RunError::Native("TypeError: value is not a constructor".into()))?;
        let primitive = match values.into_iter().next() {
            Some(first) => {
                let boxed = zun_gc::with_current_heap_mut(|heap| first.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                self.invoke_function_value(handle, vec![Value::Any(boxed)])?
            }
            None => match ctor {
                "Number" => Value::Float(0.0, zun_il::NumKind::F64),
                "Boolean" => Value::Bool(false),
                _ => Value::Str("".into()),
            },
        };
        let primitive = match unbox_arg(&primitive) {
            Ok(value) => value,
            Err(_) => primitive,
        };
        self.construct_class_from_descriptor(token, vec![primitive])
    }
}

impl<'a> Interp<'a> {
    /// The next link of `current`'s prototype chain for a symbol-keyed lookup: the
    /// class prototype object of a typed instance (where instance symbol methods
    /// and accessors live), else the dynamic prototype link.
    pub(super) fn symbol_chain_next(&self, current: zun_gc::Obj) -> Option<zun_gc::Obj> {
        self.instance_class_prototype(current).or_else(|| {
            zun_gc::with_current_heap(|heap| heap.dynamic_prototype(current))
                .and_then(|proto| proto.as_object())
                .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) })
        })
    }
}
