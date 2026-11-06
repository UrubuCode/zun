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
                let object = object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Ok(Value::Bool(false)),
                };
                let (is_map, is_set) = zun_gc::with_current_heap(|heap| {
                    (heap.map_len(object).is_some(), heap.set_len(object).is_some())
                });
                let own: &[&str] = if is_map {
                    &["size", "get", "set", "has", "delete", "clear", "forEach", "keys", "values", "entries"]
                } else if is_set {
                    &["size", "add", "has", "delete", "clear", "forEach", "keys", "values", "entries", "union", "intersection", "difference", "symmetricDifference", "isSubsetOf", "isSupersetOf", "isDisjointFrom"]
                } else {
                    &["get", "set", "add", "has", "delete"]
                };
                let inherited = [
                    "constructor", "toString", "toLocaleString", "valueOf", "hasOwnProperty",
                    "isPrototypeOf", "propertyIsEnumerable", "__proto__", "__defineGetter__",
                    "__defineSetter__", "__lookupGetter__", "__lookupSetter__",
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
                let mut depth = 0;
                while let Some(current) = cursor {
                    let found = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get_symbol(current, symbol).is_some()
                            || heap.dynamic_get_symbol_function(current, symbol).is_some()
                            || heap.dynamic_symbol_getter(current, symbol).is_some()
                    });
                    if found {
                        return Ok(Value::Bool(true));
                    }
                    cursor = zun_gc::with_current_heap(|heap| heap.dynamic_prototype(current))
                        .and_then(|proto| proto.as_object())
                        .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                    depth += 1;
                    if depth > 64 {
                        break;
                    }
                }
                Ok(Value::Bool(false))
            }
            "reflectHas" => {
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name,
                    _ => return Err(RunError::BadToken),
                };
                // An array answers for its elements (holes are absent) and `length`.
                if zun_gc::with_current_heap(|heap| heap.array_len(object).is_some()) {
                    let in_array = self.call_managed_collection("arrayHas", args)?;
                    if matches!(in_array, Value::Bool(true)) || !zun_gc::with_current_heap(|heap| heap.dynamic_has(object, name)) {
                        return Ok(in_array);
                    }
                    return Ok(Value::Bool(true));
                }
                // A dynamic object answers for its whole prototype chain.
                if zun_gc::with_current_heap(|heap| heap.is_dynamic_object(object)) {
                    return self.call_managed_collection("forInHas", args);
                }
                let dynamic_hit = zun_gc::with_current_heap(|heap| heap.dynamic_has(object, name));
                if dynamic_hit {
                    return Ok(Value::Bool(true));
                }
                // A typed class instance: own fields, then methods / accessors
                // inherited through its class chain, then `constructor`.
                let typed = zun_gc::with_current_heap(|heap| {
                    !heap.is_dynamic_object(object) && heap.array_len(object).is_none() && object.type_rid() >= 1
                });
                if typed {
                    let name = name.to_string();
                    if name == "constructor" {
                        return Ok(Value::Bool(true));
                    }
                    if self.object_own_entries_impl(object, false)?.iter().any(|(key, _)| key == &name) {
                        return Ok(Value::Bool(true));
                    }
                    let found = self.instance_method_rid(object, &name).is_some()
                        || self.instance_method_rid(object, &format!("get_{name}")).is_some();
                    return Ok(Value::Bool(found));
                }
                Ok(Value::Bool(false))
            }
            "reflectDelete" => {
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
                let object = object(args.first().ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let name: std::rc::Rc<str> = match args.get(1).ok_or(RunError::BadToken)? {
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
                    if let Some(callback) = zun_gc::with_current_heap(|heap| heap.proxy_trap(object, "deleteProperty")) {
                        let result = self.invoke_proxy_trap(
                            callback,
                            vec![Value::Object(target), Value::Str(name.clone())],
                        )?;
                        let verdict = !matches!(result, Value::Bool(false) | Value::Void | Value::Null);
                        if verdict && !zun_gc::with_current_heap(|heap| heap.dynamic_can_delete(target, name)) {
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
                        heap.array_len(object).map(|len| index >= len || heap.array_integrity(object) < 2)
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
            "reflectConstruct" => {
                let token = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) if any.as_class_descriptor().is_some() => {
                        any.as_class_descriptor().ok_or(RunError::BadToken)?
                    }
                    _ => {
                        return match self.new_error_object("TypeError", "value is not a constructor") {
                            Some(object) => Err(RunError::Thrown(Value::Object(object))),
                            None => Err(RunError::BadToken),
                        };
                    }
                };
                let array = object(args.get(1).ok_or(RunError::BadToken)?)
                    .ok_or(RunError::BadToken)?;
                let values = self.read_array_values(array)?;
                self.construct_class_from_descriptor(token, values)
            }
            _ => Err(RunError::BadToken),
        }
    }
}
