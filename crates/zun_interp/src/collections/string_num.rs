//! String and number coercion / formatting native methods.
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    #[inline]
    pub(super) fn col_string_num(&mut self, name: &str, args: &[Value]) -> Result<Value, RunError> {
        match name {
            "replaceFn" => {
                // `str.replace(search, fn)` / `str.replaceAll(search, fn)`: the
                // replacement of each match is `fn(match, ...groups, offset, str)`.
                let search = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let input = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                    Value::Str(text) => text,
                    _ => return Err(RunError::BadToken),
                };
                let callback = match args.get(2).ok_or(RunError::BadToken)? {
                    Value::Function(handle) => *handle,
                    _ => return Err(RunError::BadToken),
                };
                let all = matches!(args.get(3), Some(Value::Bool(true)));
                let spans: Vec<(usize, usize, Vec<Option<String>>)> = match &search {
                    Value::Object(regex) => zun_gc::with_current_heap_mut(|heap| {
                        heap.regex_replace_spans(*regex, &input, all)
                    })
                    .ok_or(RunError::BadToken)?
                    .map_err(|error| RunError::Native(error.to_string()))?,
                    other => {
                        let needle = self.dyn_to_string(other)?;
                        let mut found = Vec::new();
                        let to16 = |byte: usize| zun_core::wtf16::len16(&input[..byte]);
                        if needle.is_empty() {
                            // Empty pattern: every UTF-16 position, tearing astral pairs.
                            let total = zun_core::wtf16::len16(&input);
                            for at in 0..=total {
                                found.push((at, at, vec![Some(String::new())]));
                                if !all {
                                    break;
                                }
                            }
                        } else {
                            let mut from = 0;
                            while let Some(at) = input[from..].find(&needle) {
                                let start = from + at;
                                found.push((
                                    to16(start),
                                    to16(start + needle.len()),
                                    vec![Some(needle.clone())],
                                ));
                                from = start + needle.len();
                                if !all {
                                    break;
                                }
                            }
                        }
                        found
                    }
                };
                let to_any = |value: Value| {
                    Value::Any(
                        zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                            .unwrap_or_else(zun_core::Any::null),
                    )
                };
                let group_names: Vec<Option<String>> = match &search {
                    Value::Object(regex) => {
                        zun_gc::with_current_heap(|heap| heap.regex_group_names(*regex))
                            .unwrap_or_default()
                    }
                    _ => Vec::new(),
                };
                let has_named = group_names.iter().any(|name| name.is_some());
                let units: Vec<u16> = zun_core::wtf16::encode(&input).collect();
                let mut out_units: Vec<u16> = Vec::new();
                let mut last = 0;
                for (start, end, groups) in spans {
                    out_units.extend_from_slice(&units[last..start]);
                    let named_groups: Vec<(String, Option<String>)> = if has_named {
                        group_names
                            .iter()
                            .enumerate()
                            .filter_map(|(index, name)| {
                                name.as_ref().map(|name| {
                                    (name.clone(), groups.get(index).cloned().flatten())
                                })
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let mut call_args: Vec<Value> = groups
                        .into_iter()
                        .map(|group| match group {
                            Some(text) => to_any(Value::Str(text.into())),
                            None => to_any(Value::Void),
                        })
                        .collect();
                    call_args.push(to_any(Value::Float(start as f64, zun_il::NumKind::F64)));
                    call_args.push(to_any(Value::Str(input.clone())));
                    if has_named {
                        let object = zun_gc::with_current_heap_mut(|heap| {
                            let object = heap.alloc_dynamic();
                            heap.dynamic_set_prototype(object, Some(zun_core::Any::null()));
                            for (name, value) in named_groups {
                                let any = match value {
                                    Some(text) => {
                                        zun_core::Any::string(heap.intern_string(text.into()))
                                    }
                                    None => zun_core::Any::void(),
                                };
                                heap.dynamic_set(object, name, any);
                            }
                            object
                        });
                        call_args.push(Value::Any(zun_core::Any::object(object.addr() as usize)));
                    }
                    let produced = self.invoke_callback(callback, call_args)?;
                    out_units.extend(zun_core::wtf16::encode(&self.dyn_to_string(&produced)?));
                    last = end;
                }
                out_units.extend_from_slice(&units[last..]);
                if let Value::Object(regex) = &search {
                    zun_gc::with_current_heap_mut(|heap| {
                        if heap
                            .regex_flags(*regex)
                            .is_some_and(|flags| flags.contains('g'))
                        {
                            heap.regex_store_last_index(*regex, 0.0)
                        } else {
                            Ok(())
                        }
                    })
                    .map_err(|error| RunError::Native(error.to_string()))?;
                }
                Ok(Value::Str(zun_core::wtf16::from_units(&out_units).into()))
            }
            "stringMethod" => {
                let receiver = match unbox_arg(args.first().ok_or(RunError::BadToken)?)? {
                    Value::Str(text) => text,
                    _ => return Err(RunError::BadToken),
                };
                let name = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.clone(),
                    _ => return Err(RunError::BadToken),
                };
                let result = self
                    .string_method_dyn(&receiver, &name, &args[2..])?
                    .ok_or(RunError::BadToken)?;
                let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                    .unwrap_or_else(zun_core::Any::null);
                Ok(Value::Any(boxed))
            }
            "stringCoerce" | "stringCoerceStrict" => {
                // `String(x)` — ECMAScript ToString. Primitives stringify
                // directly; an object runs OrdinaryToPrimitive with hint
                // "string": call `[Symbol.toPrimitive]("string")` if present and
                // ToString its result, else expose `[Symbol.toStringTag]` as
                // `"[object <Tag>]"`, else `"[object Object]"`.
                let value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let text = match value {
                    Value::Object(obj)
                        if zun_gc::with_current_heap(|heap| heap.array_len(obj).is_some()) =>
                    {
                        self.array_join_dyn(obj, ",")?.unwrap_or_default()
                    }
                    Value::Object(obj)
                        if zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some()) =>
                    {
                        zun_gc::with_current_heap(|heap| {
                            heap.bigint_value(obj)
                                .map(|v| v.to_string())
                                .unwrap_or_default()
                        })
                    }
                    Value::Object(obj)
                        if zun_gc::with_current_heap(|heap| heap.regex_source(obj).is_some()) =>
                    {
                        zun_gc::with_current_heap(|heap| {
                            let source = heap.regex_source(obj).unwrap_or_default();
                            let flags = heap.regex_flags(obj).unwrap_or_default();
                            let ordered: String =
                                "dgimsuvy".chars().filter(|c| flags.contains(*c)).collect();
                            format!("/{source}/{ordered}")
                        })
                    }
                    Value::Object(obj) => {
                        if let Some(prim) = self.ordinary_to_primitive(obj, "string")? {
                            prim.display()
                        } else {
                            let tag = zun_gc::with_current_heap(|heap| {
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
                            match tag {
                                Some(tag) => format!("[object {tag}]"),
                                None => "[object Object]".to_string(),
                            }
                        }
                    }
                    Value::Any(a) if a.as_symbol().is_some() => {
                        if name == "stringCoerceStrict" {
                            return Err(RunError::Native(
                                "TypeError: Cannot convert a Symbol value to a string".into(),
                            ));
                        }
                        let id = a.as_symbol().unwrap_or_else(|| unreachable!());
                        let description = zun_gc::with_current_heap(|heap| {
                            heap.symbol_description(id).map(|d| d.to_string())
                        });
                        format!("Symbol({})", description.unwrap_or_default())
                    }
                    other => other.display(),
                };
                Ok(Value::Str(text.into()))
            }
            "splitLimit" => {
                // `split` limit: undefined means unbounded (-1); anything else is
                // ToUint32 of ToNumber.
                let value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                if matches!(value, Value::Void) {
                    return Ok(Value::Int(-1, zun_il::NumKind::I64));
                }
                let number = self.to_number_value(value)?;
                let limit = if number.is_finite() {
                    number.trunc().rem_euclid(4294967296.0) as i64
                } else {
                    0
                };
                Ok(Value::Int(limit, zun_il::NumKind::I64))
            }
            "numberCoerce" => {
                // `Number(x)` — ECMAScript ToNumber. A number is itself; a
                // string parses (NaN on failure); a boolean is 0/1; `null`→0,
                // `undefined`→NaN. An object runs OrdinaryToPrimitive with hint
                // "number": call `[Symbol.toPrimitive]("number")` if present and
                // ToNumber its result, else ToNumber of its string coercion.
                if let Some(Value::Any(any)) = args.first() {
                    if any.tag() == zun_core::AnyTag::Symbol {
                        return Err(RunError::Native(
                            "TypeError: Cannot convert a Symbol value to a number".into(),
                        ));
                    }
                }
                let value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let number = self.to_number_value(value)?;
                Ok(Value::Float(number, zun_il::NumKind::F64))
            }
            "mathApply" => {
                let name = match args.first().ok_or(RunError::BadToken)? {
                    Value::Str(text) => text.to_string(),
                    _ => return Err(RunError::BadToken),
                };
                let array = match unbox_arg(args.get(1).ok_or(RunError::BadToken)?)? {
                    Value::Object(obj) => Some(obj),
                    _ => None,
                };
                let elements: Vec<zun_core::Any> = match array {
                    Some(obj) => zun_gc::with_current_heap(|heap| {
                        let length = heap.array_len(obj).unwrap_or(0);
                        (0..length)
                            .map(|i| heap.array_get(obj, i).unwrap_or_else(zun_core::Any::void))
                            .collect()
                    }),
                    None => Vec::new(),
                };
                let mut numbers = Vec::with_capacity(elements.len());
                for any in elements {
                    if any.tag() == zun_core::AnyTag::Symbol {
                        return Err(RunError::Native(
                            "TypeError: Cannot convert a Symbol value to a number".into(),
                        ));
                    }
                    let value = zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                        .ok_or(RunError::BadToken)?;
                    if let Value::Object(obj) = &value {
                        if zun_gc::with_current_heap(|heap| heap.bigint_value(*obj).is_some()) {
                            return Err(RunError::Native(
                                "TypeError: Cannot convert a BigInt value to a number".into(),
                            ));
                        }
                    }
                    numbers.push(self.to_number_value(value)?);
                }
                let result = zun_natives::apply_math(&name, &numbers).unwrap_or(f64::NAN);
                Ok(Value::Float(result, zun_il::NumKind::F64))
            }
            "numberCoerceStrict" => {
                let value = unbox_arg(args.first().ok_or(RunError::BadToken)?)?;
                let is_bigint = match &value {
                    Value::Object(obj) => {
                        zun_gc::with_current_heap(|heap| heap.bigint_value(*obj).is_some())
                    }
                    _ => false,
                };
                let is_symbol = matches!(
                    args.first(),
                    Some(Value::Any(any)) if any.tag() == zun_core::AnyTag::Symbol
                );
                if is_bigint {
                    return Err(RunError::Native(
                        "TypeError: Cannot convert a BigInt value to a number".into(),
                    ));
                }
                if is_symbol {
                    return Err(RunError::Native(
                        "TypeError: Cannot convert a Symbol value to a number".into(),
                    ));
                }
                let number = self.to_number_value(value)?;
                Ok(Value::Float(number, zun_il::NumKind::F64))
            }
            "looseEquals" => {
                let left = args.first().ok_or(RunError::BadToken)?.clone();
                let right = args.get(1).ok_or(RunError::BadToken)?.clone();
                Ok(Value::Bool(self.loose_equals(left, right)?))
            }
            "numberToFixedJs"
            | "numberToExponentialJs"
            | "numberToPrecisionJs"
            | "numberToStringRadixJs" => {
                let x = match args.first().ok_or(RunError::BadToken)? {
                    Value::Float(number, _) => *number,
                    Value::Int(number, _) => *number as f64,
                    _ => return Err(RunError::BadToken),
                };
                let arg = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)?;
                let arg_number = match arg {
                    Value::Void => None,
                    other => Some(self.to_number_value(other)?),
                };
                let result = match name {
                    "numberToFixedJs" => zun_natives::js_to_fixed(x, arg_number.unwrap_or(0.0)),
                    "numberToExponentialJs" => zun_natives::js_to_exponential(x, arg_number),
                    "numberToPrecisionJs" => zun_natives::js_to_precision(x, arg_number),
                    _ => zun_natives::js_to_radix_string(x, arg_number),
                };
                match result {
                    Ok(text) => Ok(Value::Str(text.into())),
                    Err(message) => {
                        let text = message.strip_prefix("RangeError: ").unwrap_or(&message);
                        match self.new_error_object("RangeError", text) {
                            Some(object) => Err(RunError::Thrown(Value::Object(object))),
                            None => Err(RunError::Native(message)),
                        }
                    }
                }
            }
            "stringSymbolDispatch" => self.string_symbol_dispatch(args),
            _ => Err(RunError::BadToken),
        }
    }
}
