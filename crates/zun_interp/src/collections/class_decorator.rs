//! Class construction, decorator, and instanceof native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    pub(super) fn col_class_decorator(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RunError> {
        match name {
            "dynamicConstruct" => {
                // `new ns.C(...args)`: construct a class export of a dynamically
                // imported namespace. `(namespace, className, argsArray)`.
                let namespace = args.first().ok_or(RunError::BadToken)?.clone();
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let array =
                    object(args.get(2).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let values = self.read_array_values(array)?;
                let loader = self.loader.ok_or(RunError::BadToken)?;
                let result = crate::namespace::construct_namespace_class(
                    loader,
                    &namespace,
                    &name,
                    values,
                    &mut *self.output,
                )?;
                // The snapshot object lives on the current heap, so box it as an
                // object `Any` for the dynamic call site.
                let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                Ok(Value::Any(boxed))
            }
            "dynamicSetFunction" => {
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let function = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                zun_gc::with_current_heap_mut(|heap| {
                    heap.dynamic_set_function(object, name.to_string(), function);
                });
                Ok(Value::Void)
            }
            "decoratorAddInitializer" => {
                let function = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                self.decorator_initializers.push(function);
                Ok(Value::Void)
            }
            "decoratorRunInitializers" => {
                // Run only the batch queued so far (a callback that registers
                // more initializers cannot extend it), popping one at a time so
                // the not-yet-run tail stays in the rooted queue.
                let batch = self.decorator_initializers.len();
                // Walk the current batch by index instead of `remove(0)`, which
                // would shift the whole queue per initializer (O(N²)). The
                // unprocessed tail stays in the rooted queue throughout.
                for index in 0..batch {
                    let Some(function) = self.decorator_initializers.get(index).copied() else {
                        break;
                    };
                    if let Err(error) =
                        self.invoke_function_value(function, vec![Value::Any(zun_core::Any::null())])
                    {
                        // The already-run prefix is gone, as with `pop_front`;
                        // initializers a callback queued stay for the next drain.
                        self.decorator_initializers.drain(..index + 1);
                        return Err(error);
                    }
                }
                let processed = batch.min(self.decorator_initializers.len());
                self.decorator_initializers.drain(..processed);
                Ok(Value::Void)
            }
            "classBox" => {
                let descriptor = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) if any.as_class_descriptor().is_some() => *any,
                    _ => return Err(RunError::BadToken),
                };
                let record = zun_gc::with_current_heap_mut(|heap| {
                    let record = heap.alloc_dynamic();
                    heap.dynamic_set(record, "__classDesc", descriptor);
                    record
                });
                Ok(Value::Object(record))
            }
            "applyDecorator" => {
                // `applyDecorator(decoratorFn, value, context)` — invoke a
                // decorator-factory result (a function value) with
                // `(value, context)` at class-definition time, discarding the
                // return. Backs decorator factories `@dec(args)`.
                let function = match args.first().ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                let value = args.get(1).ok_or(RunError::BadToken)?.clone();
                let context = args.get(2).ok_or(RunError::BadToken)?.clone();
                self.invoke_function_value(function, vec![value, context])?;
                Ok(Value::Void)
            }
            "classSetSymbolFunction" => {
                // `classSetSymbolFunction(classRid, symbol, fn)` — attach a
                // method under a Symbol key to a typed class's per-type side
                // table (the typed-class analogue of
                // `dynamicSetSymbolFunction`).
                let class_rid = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    Value::Float(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                let symbol = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_symbol().ok_or(RunError::BadToken)?,
                    _ => return Err(RunError::BadToken),
                };
                let function = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                self.class_symbol_methods
                    .insert((class_rid, symbol.get()), function);
                // Subclasses inherit the symbol method unless they declare their own.
                let count = self.module.tables.type_defs.len() as u32;
                for rid in 1..=count {
                    if rid == class_rid {
                        continue;
                    }
                    let mut cursor = rid;
                    let mut hops = 0;
                    while hops < 64 {
                        hops += 1;
                        let Some(def) = self.module.tables.type_defs.get(cursor as usize - 1)
                        else {
                            break;
                        };
                        if def.base.is_null()
                            || def.base.table() != Some(zun_metadata::TableKind::TypeDef)
                        {
                            break;
                        }
                        cursor = def.base.rid();
                        if cursor == class_rid {
                            self.class_symbol_methods
                                .entry((rid, symbol.get()))
                                .or_insert(function);
                            break;
                        }
                    }
                }
                Ok(Value::Void)
            }
            "classSetMethod" => {
                let class_rid = match args.first().ok_or(RunError::BadToken)? {
                    Value::Int(value, _) => *value as u32,
                    Value::Float(value, _) => *value as u32,
                    _ => return Err(RunError::BadToken),
                };
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let function = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                self.class_named_methods
                    .entry(class_rid)
                    .or_default()
                    .insert(name, function);
                Ok(Value::Void)
            }
            "classSetFieldDefault" => {
                let as_u32 = |value: &Value| match value {
                    Value::Int(value, _) => Ok(*value as u32),
                    Value::Float(value, _) => Ok(*value as u32),
                    _ => Err(RunError::BadToken),
                };
                let class_rid = as_u32(args.first().ok_or(RunError::BadToken)?)?;
                let slot = as_u32(args.get(1).ok_or(RunError::BadToken)?)?;
                let value = unbox_arg(args.get(2).ok_or(RunError::BadToken)?)?;
                self.class_field_defaults.insert((class_rid, slot), value);
                Ok(Value::Void)
            }
            "dynamicSetSymbolFunction" => {
                // Attach a method under a Symbol key (e.g.
                // `C[Symbol.hasInstance]`). The symbol arrives as an immediate
                // `Any`; the function value goes into the dedicated
                // symbol-function slot since functions are not boxable `Any`.
                let object =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                let function = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Function(function) => *function,
                    _ => return Err(RunError::BadToken),
                };
                let symbol = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Any(any) => any.as_symbol(),
                    _ => return Err(RunError::BadToken),
                };
                match symbol {
                    Some(symbol) => zun_gc::with_current_heap_mut(|heap| {
                        heap.dynamic_set_symbol_function(object, symbol, function);
                    }),
                    None => {
                        // A runtime key that is not a symbol names an ordinary method.
                        let name = self.dyn_to_string(&args[1])?;
                        zun_gc::with_current_heap_mut(|heap| {
                            heap.dynamic_set_function(object, name, function);
                        });
                    }
                }
                Ok(Value::Void)
            }
            "instanceofBuiltin" => {
                let ctor = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let operand = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let found = match operand {
                    Value::Function(_) => ctor == "Function" || ctor == "Object",
                    Value::Object(obj) => {
                        let direct = zun_gc::with_current_heap(|heap| {
                            builtin_constructor_name(heap, obj).map(|name| name == ctor)
                        });
                        match direct {
                            Some(true) => true,
                            _ if ctor == "Object" => true,
                            _ => {
                                // A user subclass of a built-in extends its
                                // `__Builtin<Name>` wrapper base.
                                let wrapper = format!("__Builtin{ctor}");
                                let mut rid = obj.type_rid();
                                let mut hit = false;
                                while rid >= 1 {
                                    let Some(def) =
                                        self.module.tables.type_defs.get(rid as usize - 1)
                                    else {
                                        break;
                                    };
                                    if self.module.strings.get(def.name) == wrapper {
                                        hit = true;
                                        break;
                                    }
                                    rid = if def.base.is_null() {
                                        0
                                    } else {
                                        def.base.rid()
                                    };
                                }
                                hit
                            }
                        }
                    }
                    _ => false,
                };
                Ok(Value::Bool(found))
            }
            "instanceofDynamic" => {
                // `value instanceof C` where `C` is a dynamic object. If `C`
                // carries a `[Symbol.hasInstance]` method, call it with `value`
                // and return `ToBoolean` of the result; otherwise a dynamic
                // object has no class identity, so the answer is `false`.
                let value = args.first().ok_or(RunError::BadToken)?.clone();
                // A class descriptor held in an `any` variable: a user (or prelude)
                // class answers through the runtime class hierarchy.
                if let Ok(Value::Any(descriptor)) =
                    unbox_arg(args.get(1).ok_or(RunError::BadToken)?)
                {
                    if let Some(rid) = descriptor
                        .as_class_descriptor()
                        .and_then(|t| self.user_class_rid(t))
                    {
                        return self.call_managed_collection(
                            "instanceofClass",
                            &[value, Value::Int(rid as i64, zun_il::NumKind::I32)],
                        );
                    }
                    // A realm base-type descriptor: resolve its global name, then the
                    // class of that name (or the built-in constructor check).
                    if let Some(token) = descriptor.as_class_descriptor() {
                        let class_name = zun_gc::with_current_heap_mut(|heap| {
                            let global = heap.global_object();
                            heap.dynamic_entries(global).and_then(|entries| {
                                entries
                                    .into_iter()
                                    .find(|(_, any)| any.as_class_descriptor() == Some(token))
                                    .map(|(name, _)| name)
                            })
                        });
                        if let Some(class_name) = class_name {
                            let rid = self.type_rid_of(&class_name);
                            if let Some(rid) = rid {
                                return self.call_managed_collection(
                                    "instanceofClass",
                                    &[value, Value::Int(rid as i64, zun_il::NumKind::I32)],
                                );
                            }
                            return self.call_managed_collection(
                                "instanceofBuiltin",
                                &[value, Value::Str(class_name.into())],
                            );
                        }
                        return Ok(Value::Bool(false));
                    }
                }
                let rhs =
                    object(args.get(1).ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // A constructor record (`Error`, `Array` ... read through `any`): the
                // class it names decides membership.
                let (record_desc, record_name) = zun_gc::with_current_heap(|heap| {
                    (
                        heap.dynamic_get(rhs, "__classDesc"),
                        heap.dynamic_get(rhs, "name").and_then(|any| {
                            match Value::unbox_any(any, heap) {
                                Some(Value::Str(text)) => Some(text.to_string()),
                                _ => None,
                            }
                        }),
                    )
                });
                if let Some(rid) = record_desc
                    .and_then(|d| d.as_class_descriptor())
                    .and_then(|t| self.user_class_rid(t))
                {
                    return self.call_managed_collection(
                        "instanceofClass",
                        &[value, Value::Int(rid as i64, zun_il::NumKind::I32)],
                    );
                }
                if let Some(name) = record_name {
                    if matches!(
                        name.as_str(),
                        "Array"
                            | "Map"
                            | "Set"
                            | "Promise"
                            | "RegExp"
                            | "Object"
                            | "Function"
                            | "Date"
                    ) {
                        return self.call_managed_collection(
                            "instanceofBuiltin",
                            &[value, Value::Str(name.into())],
                        );
                    }
                }
                let handler = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_symbol_function(
                        rhs,
                        zun_core::WellKnownSymbol::HasInstance.id(),
                    )
                });
                let found = match handler {
                    Some(handle) => {
                        let result = self.invoke_function_value(handle, vec![value])?;
                        zun_gc::with_current_heap(|heap| is_truthy(&result, heap))
                    }
                    None => false,
                };
                Ok(Value::Bool(found))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
