//! dynamicCall0..6 dynamic member-call dispatch (very hot).
//!
//! Extracted verbatim from the `call_managed_collection_inner` dispatch;
//! each arm routes here by its unique native method name.

use super::*;

impl<'a> Interp<'a> {
    #[inline]
    pub(super) fn col_dynamic_call(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RunError> {
        match name {
            "dynamicCall0" | "dynamicCall1" | "dynamicCall2" | "dynamicCall3" | "dynamicCall4"
            | "dynamicCall5" | "dynamicCall6" => {
                // Consumed here, before anything can re-enter, so the mark
                // covers this call only.
                let builtin_member_only = super::take_builtin_member_only();
                // Tier 0 type feedback: this is the dynamic `any` miss path, so
                // recording the observed receiver/argument tags here is free for
                // typed code (which resolves to typed natives and never arrives).
                // Gated and off by default; the `active_frame` (method rid +
                // bytecode offset of the dynamic call instruction) is the stable
                // site key.
                if crate::feedback::enabled() {
                    if let Some((method_rid, offset)) = self.active_frame {
                        crate::feedback::record(self.module, method_rid, offset, args);
                    }
                }
                // A method a program installed on `String.prototype` wins over
                // the built-in of the same name and is called with the string
                // as `this`.
                if let (Ok(Value::Str(text)), Some(Value::Str(member))) =
                    (unbox_arg(args.first().ok_or(RunError::BadToken)?), args.get(1))
                {
                    if let Some(handle) = self.installed_wrapper_method("String", member) {
                        let call_values = args[2..]
                            .iter()
                            .map(|arg| match arg {
                                Value::Any(any) => {
                                    zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                        .ok_or(RunError::BadToken)
                                }
                                other => Ok(other.clone()),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let call_args = self.with_this_value(handle, Value::Str(text), call_values);
                        let result = self.invoke_function_value(handle, call_args)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                // A primitive-wrapper instance (`new String("ab")`) has no
                // methods of its own beyond the ones its class declares: any
                // other member is its primitive's (`w.trim()`, `n.toFixed(1)`).
                if let (Some(wrapper), Some(Value::Str(member))) =
                    (args.first().and_then(object), args.get(1))
                {
                    let declared = self
                        .class_named_methods
                        .get(&wrapper.type_rid())
                        .is_some_and(|methods| methods.contains_key(&**member));
                    if !declared {
                        if let Some(ctor) = self.wrapper_instance_kind(wrapper) {
                            let own = zun_gc::with_current_heap(|heap| {
                                heap.dynamic_get_function(wrapper, member).is_some()
                                    || heap.dynamic_get(wrapper, member).is_some()
                            });
                            if !own {
                                let primitive = self.wrapper_primitive(Value::Object(wrapper), ctor)?;
                                let mut redirected = args.to_vec();
                                redirected[0] = self.proto_box(primitive);
                                return self.col_dynamic_call(name, &redirected);
                            }
                        }
                    }
                }
                // A string receiver (an `any`-typed string): JS String methods.
                if let Ok(Value::Str(text)) = unbox_arg(args.first().ok_or(RunError::BadToken)?) {
                    // A RegExp argument routes the regex-taking String methods
                    // (`replace`/`replaceAll`/`match`/`matchAll`/`search`) to the
                    // regex natives, as the statically typed forms do.
                    if let (Value::Str(member), Some(first)) =
                        (args.get(1).ok_or(RunError::BadToken)?, args.get(2))
                    {
                        let regex = match unbox_arg(first) {
                            Ok(Value::Object(obj))
                                if zun_gc::with_current_heap(|heap| {
                                    heap.regex_source(obj).is_some()
                                }) =>
                            {
                                Some(Value::Object(obj))
                            }
                            _ => None,
                        };
                        if let Some(regex) = regex {
                            let subject = Value::Str(text.clone());
                            let routed = match member.as_ref() {
                                "replace" | "replaceAll" if args.len() == 4 => {
                                    let all = member.as_ref() == "replaceAll";
                                    match unbox_arg(&args[3])? {
                                        Value::Function(handle) => {
                                            Some(self.call_managed_collection(
                                                "replaceFn",
                                                &[
                                                    regex,
                                                    subject,
                                                    Value::Function(handle),
                                                    Value::Bool(all),
                                                ],
                                            )?)
                                        }
                                        Value::Str(replacement) => {
                                            Some(self.call_managed_collection(
                                                "regexpReplace",
                                                &[regex, subject, Value::Str(replacement)],
                                            )?)
                                        }
                                        _ => None,
                                    }
                                }
                                "match" => Some(
                                    self.call_managed_collection("regexpMatch", &[regex, subject])?,
                                ),
                                "search" => {
                                    Some(self.call_managed_collection(
                                        "regexpSearch",
                                        &[regex, subject],
                                    )?)
                                }
                                "matchAll" => Some(self.call_managed_collection(
                                    "stringMatchAll",
                                    &[regex, subject],
                                )?),
                                "split" if args.len() == 3 => Some(
                                    self.call_managed_collection("regexpSplit", &[regex, subject])?,
                                ),
                                "split" if args.len() == 4 => {
                                    let limit = match unbox_arg(&args[3])? {
                                        Value::Void | Value::Null => None,
                                        other => {
                                            let n = self.dyn_to_number(&other)?;
                                            Some(if n.is_nan() || n <= 0.0 {
                                                0
                                            } else if n >= 4294967295.0 {
                                                4294967295
                                            } else {
                                                n as i64
                                            })
                                        }
                                    };
                                    match limit {
                                        None => Some(self.call_managed_collection(
                                            "regexpSplit",
                                            &[regex, subject],
                                        )?),
                                        Some(n) => Some(self.call_managed_collection(
                                            "regexpSplitLimit",
                                            &[regex, subject, Value::Int(n, zun_il::NumKind::I64)],
                                        )?),
                                    }
                                }
                                _ => None,
                            };
                            if let Some(result) = routed {
                                let boxed =
                                    zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                        .unwrap_or_else(zun_core::Any::null);
                                return Ok(Value::Any(boxed));
                            }
                        }
                    }
                    if let Value::Str(member) = args.get(1).ok_or(RunError::BadToken)? {
                        if let Some(result) = self.string_method_dyn(&text, member, &args[2..])? {
                            let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                .unwrap_or_else(zun_core::Any::null);
                            return Ok(Value::Any(boxed));
                        }
                    }
                }
                let member = match args.get(1).ok_or(RunError::BadToken)? {
                    Value::Str(name) => name.clone(),
                    _ => return Err(RunError::BadToken),
                };
                // A dynamic receiver that unboxes to a primitive string carries
                // the full String method surface (`charCodeAt`, `slice`,
                // `toUpperCase`, …). Those natives live in the `string` group
                // under their bare name; dispatch directly so a string reached
                // through `any` keeps its methods.
                let receiver_concrete = match args.first().ok_or(RunError::BadToken)? {
                    Value::Any(any) => {
                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                            .unwrap_or(Value::Void)
                    }
                    other => other.clone(),
                };
                // A function carried through `any` rides in a `__boxedFn` record.
                let receiver_concrete = zun_gc::with_current_heap(|heap| {
                    unbox_function_box(receiver_concrete, heap)
                });
                // `Symbol.call(...)`, `.apply(...)` and `.bind(...)`: the
                // constructor value is callable, so the three
                // `Function.prototype` members act on its call behaviour.
                let receiver_concrete =
                    if matches!(member.as_ref(), "call" | "apply" | "bind") {
                        match self.callable_ctor_handle(&receiver_concrete) {
                            Some(handle) => Value::Function(handle),
                            None => receiver_concrete,
                        }
                    } else {
                        receiver_concrete
                    };
                if let Value::Str(_) = &receiver_concrete {
                    if let Some((id, native)) = zun_natives::resolve("string", &member) {
                        let mut native_values = Vec::with_capacity(args.len() - 1);
                        native_values.push(receiver_concrete.clone());
                        for arg in &args[2..] {
                            native_values.push(match arg {
                                Value::Any(any) => {
                                    zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                        .unwrap_or(Value::Void)
                                }
                                other => other.clone(),
                            });
                        }
                        if native.params.len() == native_values.len() {
                            // Coerce a float argument to the native's declared
                            // integer index/width (the typed call path does this
                            // for numeric literals; the dynamic path must too).
                            for (value, param) in native_values.iter_mut().zip(native.params.iter())
                            {
                                if let Value::Float(n, _) = value {
                                    use zun_natives::NativeTy::*;
                                    let kind = match param {
                                        I8 => Some(zun_il::NumKind::I8),
                                        I16 => Some(zun_il::NumKind::I16),
                                        I32 => Some(zun_il::NumKind::I32),
                                        I64 => Some(zun_il::NumKind::I64),
                                        U8 => Some(zun_il::NumKind::U8),
                                        U16 => Some(zun_il::NumKind::U16),
                                        U32 => Some(zun_il::NumKind::U32),
                                        U64 => Some(zun_il::NumKind::U64),
                                        _ => None,
                                    };
                                    if let Some(kind) = kind {
                                        *value = Value::Int(*n as i64, kind);
                                    }
                                }
                            }
                            let native_args: Vec<zun_natives::NativeArg> = native_values
                                .iter()
                                .map(crate::native::to_native_arg)
                                .collect();
                            let result = zun_natives::dispatch(id, &native_args, self.output)
                                .map_err(|e| RunError::Native(e.0))?;
                            let value = crate::native::from_native_value(result, id);
                            let boxed = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                                .unwrap_or_else(zun_core::Any::null);
                            return Ok(Value::Any(boxed));
                        }
                    }
                }
                // `toLocaleString()` on a string, number or boolean reached through
                // `any`: a string answers itself, a boolean its text, a number the
                // root-locale formatting typed code uses.
                if member.as_ref() == "toLocaleString" {
                    let text = match &receiver_concrete {
                        Value::Str(text) => Some(text.to_string()),
                        Value::Bool(flag) => Some(flag.to_string()),
                        Value::Float(..) | Value::Int(..) => {
                            let x = match &receiver_concrete {
                                Value::Float(number, _) => *number,
                                Value::Int(number, _) => *number as f64,
                                _ => unreachable!(),
                            };
                            match zun_natives::resolve("number", "numberToLocaleString") {
                                Some((id, _)) => {
                                    match zun_natives::dispatch(
                                        id,
                                        &[zun_natives::NativeArg::Float(x)],
                                        self.output,
                                    ) {
                                        Ok(zun_natives::NativeValue::Str(text)) => Some(text),
                                        _ => None,
                                    }
                                }
                                None => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(text) = text {
                        let boxed = zun_gc::with_current_heap_mut(|heap| {
                            Value::Str(text.into()).box_any(heap)
                        })
                        .unwrap_or_else(zun_core::Any::null);
                        return Ok(Value::Any(boxed));
                    }
                }
                // A number reached through `any` carries the Number.prototype
                // formatters (`x.toString(2)`, `x.toFixed(2)`, ...).
                if let Value::Float(_, _) | Value::Int(_, _) = &receiver_concrete {
                    let x = match &receiver_concrete {
                        Value::Float(number, _) => *number,
                        Value::Int(number, _) => *number as f64,
                        _ => unreachable!(),
                    };
                    let formatter = match member.as_ref() {
                        "toString" | "toFixed" | "toExponential" | "toPrecision" => true,
                        "valueOf" => {
                            return Ok(Value::Any(zun_core::Any::float(x)));
                        }
                        _ => false,
                    };
                    if formatter {
                        let argument = match args.get(2) {
                            Some(arg) => unbox_arg(arg)?,
                            None => Value::Void,
                        };
                        let argument = match argument {
                            Value::Void => None,
                            other => Some(self.to_number_value(other)?),
                        };
                        let result = match member.as_ref() {
                            "toFixed" => zun_natives::js_to_fixed(x, argument.unwrap_or(0.0)),
                            "toExponential" => zun_natives::js_to_exponential(x, argument),
                            "toPrecision" => zun_natives::js_to_precision(x, argument),
                            _ => zun_natives::js_to_radix_string(x, argument),
                        };
                        return match result {
                            Ok(text) => {
                                let boxed = zun_gc::with_current_heap_mut(|heap| {
                                    Value::Str(text.into()).box_any(heap)
                                })
                                .unwrap_or_else(zun_core::Any::null);
                                Ok(Value::Any(boxed))
                            }
                            Err(message) => {
                                let text = message.strip_prefix("RangeError: ").unwrap_or(&message);
                                match self.new_error_object("RangeError", text) {
                                    Some(object) => Err(RunError::Thrown(Value::Object(object))),
                                    None => Err(RunError::Native(message)),
                                }
                            }
                        };
                    }
                }
                // Boolean.prototype methods on a boolean reached through `any`.
                if let Value::Bool(flag) = &receiver_concrete {
                    match member.as_ref() {
                        "toString" => {
                            let text = Value::Str(if *flag { "true" } else { "false" }.into());
                            let boxed = zun_gc::with_current_heap_mut(|heap| text.box_any(heap))
                                .unwrap_or_else(zun_core::Any::null);
                            return Ok(Value::Any(boxed));
                        }
                        "valueOf" => return Ok(Value::Any(zun_core::Any::bool(*flag))),
                        _ => {}
                    }
                }
                // A BigInt or Symbol primitive reached through `any` resolves
                // the member on its own built-in prototype (`BigInt.prototype`,
                // `Symbol.prototype`) and calls it with the receiver as `this`.
                // The prototype holds the real method values, so a method a
                // program installs there is found exactly like a built-in one.
                {
                    let class = match &receiver_concrete {
                        Value::Object(o)
                            if zun_gc::with_current_heap(|heap| heap.bigint_value(*o).is_some()) =>
                        {
                            Some("BigInt")
                        }
                        Value::Any(any) if any.as_symbol().is_some() => Some("Symbol"),
                        _ => None,
                    };
                    if let Some(class) = class {
                        let method = self.builtin_prototype_by_name(class).and_then(|proto| {
                            zun_gc::with_current_heap(|heap| heap.dynamic_get_function(proto, &member))
                        });
                        if let Some(handle) = method {
                            let call_values = args[2..]
                                .iter()
                                .map(|arg| match arg {
                                    Value::Any(any) => {
                                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                            .ok_or(RunError::BadToken)
                                    }
                                    other => Ok(other.clone()),
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            let call_args =
                                self.with_this_value(handle, receiver_concrete.clone(), call_values);
                            let result = self.invoke_function_value(handle, call_args)?;
                            let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                .unwrap_or_else(zun_core::Any::void);
                            return Ok(Value::Any(boxed));
                        }
                        return match self
                            .new_error_object("TypeError", &format!("{member} is not a function"))
                        {
                            Some(object) => Err(RunError::Thrown(Value::Object(object))),
                            None => Err(RunError::BadToken),
                        };
                    }
                }
                if matches!(receiver_concrete, Value::Void | Value::Null) {
                    let kind = if matches!(receiver_concrete, Value::Null) {
                        "null"
                    } else {
                        "undefined"
                    };
                    return match self.new_error_object(
                        "TypeError",
                        &format!("Cannot read properties of {kind} (reading '{member}')"),
                    ) {
                        Some(object) => Err(RunError::Thrown(Value::Object(object))),
                        None => Err(RunError::BadToken),
                    };
                }
                // A static method called through a class value
                // (`(Cls as any).m()`, `this.m()` in a static): resolved on the
                // class or its bases.
                if let Value::Any(desc) = &receiver_concrete {
                    if let Some(rid) = desc
                        .as_class_descriptor()
                        .and_then(|t| self.user_class_rid(t))
                    {
                        // A function a script stored on the class value under
                        // the same name shadows the declared static method; the
                        // expando call below takes it.
                        let shadowed = Self::class_expando_read_own(
                            rid | zun_natives::USER_CLASS_FLAG,
                            &Value::Str(member.as_ref().into()),
                        )
                        .is_some();
                        if let Some((mrid, hidden)) =
                            self.class_static_method(rid, &member).filter(|_| !shadowed)
                        {
                            let mut call_args = args[2..]
                                .iter()
                                .map(|arg| match arg {
                                    Value::Any(any) => zun_gc::with_current_heap(|heap| {
                                        Value::unbox_any(*any, heap)
                                    })
                                    .ok_or(RunError::BadToken),
                                    other => Ok(other.clone()),
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            if hidden {
                                call_args.insert(0, Value::Any(*desc));
                            }
                            let handle = zun_core::FunctionHandle::new(mrid, None);
                            let result = self.invoke_function_value(handle, call_args)?;
                            let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                .unwrap_or_else(zun_core::Any::void);
                            return Ok(Value::Any(boxed));
                        }
                    }
                }
                // A function a script defined on the class value itself
                // (`Object.defineProperty(Cls, "m", { value: fn })`, a computed
                // static method): called with the class as `this`.
                if let Value::Any(desc) = &receiver_concrete {
                    if let Some(Value::Function(handle)) = desc.as_class_descriptor().and_then(|token| {
                        self.class_expando_read(token, &Value::Str(member.as_ref().into()))
                    }) {
                        let call_values = args[2..]
                            .iter()
                            .map(|arg| match arg {
                                Value::Any(any) => zun_gc::with_current_heap(|heap| {
                                    Value::unbox_any(*any, heap)
                                })
                                .ok_or(RunError::BadToken),
                                other => Ok(other.clone()),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let call_args =
                            self.with_this_value(handle, receiver_concrete.clone(), call_values);
                        let result = self.invoke_function_value(handle, call_args)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                // The `Function` constructor reached through `any`:
                // `F.call(thisArg, ...strings)` and `F.apply(thisArg, list)` build
                // the function exactly like `F(...strings)`; the receiver is unused.
                if matches!(member.as_ref(), "call" | "apply")
                    && self.is_function_constructor(&receiver_concrete)
                {
                    let strings = if member.as_ref() == "apply" {
                        match args.get(3).map(unbox_arg).transpose()? {
                            None | Some(Value::Void) | Some(Value::Null) => Vec::new(),
                            Some(list) => {
                                let boxed = self.proto_box(list);
                                let array = self
                                    .call_managed_collection("arrayFromArgumentList", &[boxed])?;
                                match object(&array) {
                                    Some(array) => self.read_array_values(array)?,
                                    None => Vec::new(),
                                }
                            }
                        }
                    } else {
                        args.get(3..).map(<[Value]>::to_vec).unwrap_or_default()
                    };
                    return self.function_from_strings_boxed(strings);
                }
                // A static method called through a built-in constructor value
                // (`const O: any = Object; O.keys(x)`).
                if let Value::Any(desc) = &receiver_concrete {
                    if let Some(kind) = desc
                        .as_class_descriptor()
                        .and_then(|token| self.builtin_static_kind(token, &member))
                    {
                        let call_args = args[2..].to_vec();
                        let handle = zun_core::FunctionHandle::new_builtin_method(kind);
                        let result = self.invoke_builtin_with_this(handle, Value::Void, call_args)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                // A built-in constructor value inherits `Function.prototype`
                // (`Object.toString()`): the member
                // resolves there and runs with the constructor as `this`.
                let inherits_function_proto = member.as_ref() == "toString"
                    && match &receiver_concrete {
                    Value::Any(desc) => desc.as_class_descriptor().is_some(),
                    Value::Function(ctor) => {
                        (self.coerce_ctor_name(*ctor).is_some()
                            && self.coerce_ctor_static_kind(*ctor, &member).is_none())
                            || self.native_global_function_name(*ctor).is_some()
                    }
                    _ => false,
                };
                {
                    if inherits_function_proto {
                        let method = Some(Self::function_prototype_singleton()).and_then(|proto| {
                            zun_gc::with_current_heap(|heap| heap.dynamic_get_function(proto, &member))
                        });
                        if let Some(handle) = method {
                            let call_values = args[2..]
                                .iter()
                                .map(|arg| match arg {
                                    Value::Any(any) => {
                                        zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                            .ok_or(RunError::BadToken)
                                    }
                                    other => Ok(other.clone()),
                                })
                                .collect::<Result<Vec<_>, _>>()?;
                            let call_args =
                                self.with_this_value(handle, receiver_concrete.clone(), call_values);
                            let result = self.invoke_function_value(handle, call_args)?;
                            let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                .unwrap_or_else(zun_core::Any::void);
                            return Ok(Value::Any(boxed));
                        }
                    }
                }
                // The same for `Number`, which reads as its coercion function
                // (`const N: any = Number; N.isInteger(x)`).
                if let Value::Function(ctor) = &receiver_concrete {
                    if let Some(kind) = self.coerce_ctor_static_kind(*ctor, &member) {
                        let call_args = args[2..].to_vec();
                        let handle = zun_core::FunctionHandle::new_builtin_method(kind);
                        let result = self.invoke_builtin_with_this(handle, Value::Void, call_args)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                // `Function.prototype.bind` on a function value reached through
                // `any`: `[thisArg, ...partial]` becomes the bound arguments.
                if let Value::Function(handle) = &receiver_concrete {
                    if member.as_ref() == "bind" {
                        let target = *handle;
                        let unboxed = args[2..]
                            .iter()
                            .map(|arg| match arg {
                                Value::Any(any) => {
                                    zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                        .ok_or(RunError::BadToken)
                                }
                                other => Ok(other.clone()),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let array = zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(unboxed.len());
                            for (index, value) in unboxed.iter().enumerate() {
                                // A callable has no `Any` form (`f.bind(g)`, `f.bind(null, g)`):
                                // it rides in a `functionBox` record the call path unwraps.
                                let any = match value {
                                    Value::Function(function) => {
                                        let record = heap.alloc_dynamic();
                                        heap.dynamic_set_function(record, "__boxedFn", *function);
                                        Value::Object(record).box_any(heap)
                                    }
                                    other => other.box_any(heap),
                                }
                                .unwrap_or_else(zun_core::Any::void);
                                let _ = heap.array_set(array, index, any);
                            }
                            array
                        });
                        let bound = self.col_function(
                            "functionBind",
                            &[Value::Function(target), Value::Object(array)],
                        )?;
                        let bound = self.col_function("functionBox", &[bound])?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| bound.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                // `Function.prototype.call/apply` on a function value reached
                // through `any`.
                if let Value::Function(handle) = &receiver_concrete {
                    if matches!(member.as_ref(), "call" | "apply") {
                        let handle = *handle;
                        let unboxed = args[2..]
                            .iter()
                            .map(|arg| match arg {
                                Value::Any(any) => {
                                    zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                        .ok_or(RunError::BadToken)
                                }
                                other => Ok(other.clone()),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let mut it = unboxed.into_iter();
                        let this_arg = it.next().unwrap_or(Value::Void);
                        let mut rest: Vec<Value> = if member.as_ref() == "apply" {
                            // CreateListFromArrayLike: an omitted, `null` or `undefined`
                            // list applies no arguments; an array-like object is read
                            // by `length` and index; any other primitive is a TypeError.
                            match it.next() {
                                None | Some(Value::Void) | Some(Value::Null) => Vec::new(),
                                Some(list) => {
                                    let boxed = self.proto_box(list);
                                    let array =
                                        self.call_managed_collection("arrayFromArgumentList", &[boxed])?;
                                    match object(&array) {
                                        Some(array) => self.read_array_values(array)?,
                                        None => Vec::new(),
                                    }
                                }
                            }
                        } else {
                            it.collect()
                        };
                        let takes_this = !handle.is_bound()
                            && self
                                .module
                                .tables
                                .method_defs
                                .get((handle.method_rid() as usize).wrapping_sub(1))
                                .map(|method| {
                                    let name = self.module.strings.get(method.name);
                                    name.ends_with("@this>")
                                        || name.ends_with("@this@rest>")
                                        || name.ends_with("@this")
                                })
                                .unwrap_or(false);
                        // A class instance method (`Class.prototype.m`) takes its
                        // receiver as the implicit first argument. This must test
                        // for a method *owned by a class*: a free function can own
                        // a virtual slot as well, and giving one the receiver
                        // shifts every real argument one place, so
                        // `f.call(undefined, 1, 2)` arrived as `f(undefined, 1)`.
                        let takes_this = takes_this
                            || (!handle.is_bound()
                                && self.method_rid_is_instance_method(handle.method_rid()))
                            || handle.builtin_method().is_some_and(|kind| kind >= 16);
                        if takes_this {
                            rest.insert(0, this_arg);
                        }
                        let result = self.invoke_function_value(handle, rest)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                // A function-valued expando property on a function (`fn.m = () => ..;
                // fn.m()`, or a static method of a run-time class): called with
                // the function itself as `this`, so `this === fn` holds and
                // `new this(...)` constructs it. Property reads and writes on
                // that receiver still land in the function's property bag.
                if let Value::Function(handle) = &receiver_concrete {
                    let bag = crate::function_expando_bag(*handle);
                    if let Some(method) =
                        zun_gc::with_current_heap(|heap| heap.dynamic_get_function(bag, &member))
                    {
                        let call_values = args[2..]
                            .iter()
                            .map(|arg| match arg {
                                Value::Any(any) => {
                                    zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                        .ok_or(RunError::BadToken)
                                }
                                other => Ok(other.clone()),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let call_args = self.with_this_value(method, Value::Function(*handle), call_values);
                        let result = self.invoke_function_value(method, call_args)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                // A static a function inherits through its `[[Prototype]]`
                // (`Object.setPrototypeOf(F, Base)`, which is how a run-time
                // class chains to its base): found on a parent function, it is
                // called like an own one; reaching a class, the call continues
                // on the class value, whose own dispatch resolves it.
                if let Value::Function(handle) = &receiver_concrete {
                    match self.function_inherited_static(*handle, &member) {
                        Some(InheritedStatic::Function(holder)) => {
                            let holder_bag = crate::function_expando_bag(holder);
                            if let Some(method) = zun_gc::with_current_heap(|heap| {
                                heap.dynamic_get_function(holder_bag, &member)
                            }) {
                                let call_values = args[2..]
                                    .iter()
                                    .map(|arg| match arg {
                                        Value::Any(any) => zun_gc::with_current_heap(|heap| {
                                            Value::unbox_any(*any, heap)
                                        })
                                        .ok_or(RunError::BadToken),
                                        other => Ok(other.clone()),
                                    })
                                    .collect::<Result<Vec<_>, _>>()?;
                                let call_args = self.with_this_value(method, Value::Function(*handle), call_values);
                                let result = self.invoke_function_value(method, call_args)?;
                                let boxed =
                                    zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                        .unwrap_or_else(zun_core::Any::void);
                                return Ok(Value::Any(boxed));
                            }
                        }
                        Some(InheritedStatic::Class(class)) => {
                            let mut forwarded = args.to_vec();
                            forwarded[0] = Value::Any(class);
                            return self.col_dynamic_call(name, &forwarded);
                        }
                        None => {}
                    }
                }
                // A member a function inherits from `Function.prototype`
                // (`fn.toString()`, or a method a program installed there):
                // called with the function itself as `this`.
                if let Value::Function(handle) = &receiver_concrete {
                    if let Some(Value::Function(method)) = self.function_prototype_member(&member) {
                        let call_values = args[2..]
                            .iter()
                            .map(|arg| match arg {
                                Value::Any(any) => {
                                    zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                        .ok_or(RunError::BadToken)
                                }
                                other => Ok(other.clone()),
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        let call_args =
                            self.with_this_value(method, Value::Function(*handle), call_values);
                        let result = self.invoke_function_value(method, call_args)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                let namespace =
                    object(args.first().ok_or(RunError::BadToken)?).ok_or(RunError::BadToken)?;
                // A built-in iterator object (from `Array`/`Map`/`Set`
                // `keys`/`values`/`entries`) answers `next()` with a fresh
                // `{ value, done }` result object, per the iterator protocol.
                // The `%Iterator.prototype%` helpers on a built-in iterator or
                // generator (`gen.map(..)`, `set.values().toArray()`).
                if let Some(index) = iterator_helpers::iter_helper_index(&member) {
                    if iterator_helpers::is_builtin_iterator_object(namespace) {
                        let result = self.iterator_builtin_call(
                            iterator_helpers::BUILTIN_ITER_HELPER_BASE + index,
                            Value::Object(namespace),
                            args[2..].to_vec(),
                        )?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                if matches!(member.as_ref(), "return" | "throw")
                    && zun_gc::with_current_heap(|heap| heap.generator_state(namespace).is_some())
                {
                    let kind = if member.as_ref() == "return" { 1 } else { 2 };
                    return self.call_generator_method(kind, namespace, &args[2..]);
                }
                if member.as_ref() == "next" {
                    if zun_gc::with_current_heap(|heap| heap.generator_state(namespace).is_some()) {
                        return self.call_generator_method(0, namespace, &args[2..]);
                    }
                    if let Some(result) = self.builtin_iterator_result(namespace)? {
                        return Ok(result);
                    }
                }
                let call_args = args[2..]
                    .iter()
                    .map(|arg| match arg {
                        Value::Any(any) => {
                            zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                .ok_or(RunError::BadToken)
                        }
                        other => Ok(other.clone()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                // An own function property (`arr.join = fn`, `obj.m = fn`) shadows
                // every built-in and inherited method of the same name, so it is
                // consulted before the typed-view, prototype-chain, array and
                // collection dispatch below.
                // A dynamic object may carry a callable member in its function
                // slot (e.g. the `resolve`/`reject` of `Promise.withResolvers`,
                // or a `bind`-stored function). Invoke it directly; the result
                // is re-boxed to `Any` for the dynamic call site. Only when no
                // such member exists is this a namespace call routed to the
                // loader.
                // A function stored through an `any` slot rides in a `__boxedFn`
                // record; it is called the same way.
                if let Some(handle) = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_function(namespace, &member).or_else(|| {
                        let any = heap.dynamic_get(namespace, &member)?;
                        let value = Value::unbox_any(any, heap)?;
                        match unbox_function_box(value, heap) {
                            Value::Function(handle) => Some(handle),
                            _ => None,
                        }
                    })
                }) {
                    // An object-literal method that uses `this` takes the
                    // receiver as a hidden first argument (marked by the
                    // `@this` suffix on its closure name).
                    let mut call_args = call_args;
                    if handle.builtin_method().is_some_and(|kind| kind >= 16) {
                        call_args.insert(0, Value::Object(namespace));
                    } else if !handle.is_bound() {
                        // A class instance method *stored on* a plain object
                        // takes the receiver too: `o.show = C.prototype.show;
                        // o.show()` must see `o` as `this`, and answered
                        // `undefined` because this site asked only about the
                        // `@this` closure marker that an object-literal method
                        // carries. `method_rid_is_instance_method` is the same
                        // question that settled the four other receiver sites
                        // (`4ae8a539`, `c07a0cea`, `0f47da98`, `529da44a`) --
                        // here the receiver was wrongly *omitted* rather than
                        // wrongly inserted.
                        let takes_this = self
                            .module
                            .tables
                            .method_defs
                            .get((handle.method_rid() as usize).wrapping_sub(1))
                            .map(|method| {
                                let n = self.module.strings.get(method.name);
                                n.ends_with("@this>") || n.ends_with("@this@rest>")
                            })
                            .unwrap_or(false)
                            || self.method_rid_is_instance_method(handle.method_rid());
                        if takes_this {
                            call_args.insert(0, Value::Object(namespace));
                        }
                    }
                    let result = self.invoke_function_value(handle, call_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any_cell(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    return Ok(Value::Any(boxed));
                }
                // The `Function` constructor held as a property (`globalThis.Function(...)`):
                // calling it is identical to constructing it.
                if zun_gc::with_current_heap(|heap| heap.dynamic_get(namespace, &member))
                    .is_some_and(|any| self.is_function_constructor(&Value::Any(any)))
                {
                    return self.function_from_strings_boxed(call_args);
                }
                // A callable constructor held as a property (`globalThis.Symbol("d")`).
                if let Some(handle) =
                    zun_gc::with_current_heap(|heap| heap.dynamic_get(namespace, &member))
                        .and_then(|any| self.callable_ctor_handle(&Value::Any(any)))
                {
                    let result = self.invoke_function_value(handle, call_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any_cell(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    return Ok(Value::Any(boxed));
                }
                // An own non-callable data property on an array (`arr.join = "x"`)
                // shadows the built-in method of that name: calling it is a
                // TypeError, not a call of the built-in.
                if zun_gc::with_current_heap(|heap| {
                    heap.array_len(namespace).is_some()
                        && heap.dynamic_has(namespace, &member)
                        && !heap.dynamic_is_accessor(namespace, &member)
                }) {
                    return match self
                        .new_error_object("TypeError", &format!("{member} is not a function"))
                    {
                        Some(object) => Err(RunError::Thrown(Value::Object(object))),
                        None => Err(RunError::BadToken),
                    };
                }
                // A typed array / ArrayBuffer / DataView reached through `any`.
                // A RegExp reached through `any` (a native-backed `RegExp`
                // subclass instance always is) answers `test` and `exec` with
                // the natives the typed forms use; the argument runs ToString.
                if matches!(member.as_ref(), "test" | "exec")
                    && zun_gc::with_current_heap(|heap| heap.regex_source(namespace).is_some())
                {
                    let input = match call_args.first() {
                        Some(argument) => self.dyn_to_string(&unbox_arg(argument)?)?,
                        None => "undefined".to_string(),
                    };
                    let native = if member.as_ref() == "test" {
                        "regexpTest"
                    } else {
                        "regexpExec"
                    };
                    let result = self.call_managed_collection(
                        native,
                        &[Value::Object(namespace), Value::Str(input.into())],
                    )?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any_cell(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    return Ok(Value::Any(boxed));
                }
                // The receiver is rooted across the native call: a callback can
                // collect, and the species step reads the receiver afterwards.
                zun_gc::root!(species_receiver = namespace);
                if let Some(result) = self.typed_view_call(namespace, &member, &call_args)? {
                    let result = self.apply_species(species_receiver.get(), &member, result)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                        .unwrap_or_else(zun_core::Any::void);
                    return Ok(Value::Any(boxed));
                }
                // A Proxy reached through `any`: the member is read through its
                // `get` trap and called with the proxy as `this`.
                if zun_gc::with_current_heap(|heap| heap.proxy_target(namespace).is_some()) {
                    let callee = self.proxy_dynamic_get(namespace, &member)?;
                    let callee = match callee {
                        Value::Any(any) => {
                            zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                                .unwrap_or(Value::Void)
                        }
                        other => other,
                    };
                    let Value::Function(handle) = callee else {
                        return match self
                            .new_error_object("TypeError", &format!("{member} is not a function"))
                        {
                            Some(object) => Err(RunError::Thrown(Value::Object(object))),
                            None => Err(RunError::BadToken),
                        };
                    };
                    let result = self.invoke_function_value(
                        handle,
                        self.with_this_arg(handle, namespace, call_args),
                    )?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                        .unwrap_or_else(zun_core::Any::void);
                    return Ok(Value::Any(boxed));
                }
                // A method inherited through the object's dynamic prototype chain
                // (e.g. a built-in collection built with a `newTarget.prototype`).
                // A borrowed built-in call skips it: the built-in itself is wanted.
                if !builtin_member_only {
                    let mut cursor = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_prototype(namespace)
                            .and_then(|any| any.as_object())
                            .map(|a| unsafe { zun_gc::Obj::from_addr(a as *mut u64) })
                    });
                    let mut depth = 0;
                    while let Some(proto) = cursor {
                        if let Some(handle) = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_get_function(proto, &member)
                        }) {
                            // A declared class instance method takes the receiver as
                            // its implicit first parameter.
                            let plain_instance_method = !handle.is_bound()
                                && handle.builtin_method().is_none()
                                && self
                                    .module
                                    .tables
                                    .method_defs
                                    .get((handle.method_rid() as usize).wrapping_sub(1))
                                    .is_some_and(|method| {
                                        let n = self.module.strings.get(method.name);
                                        !method.is_static
                                            && !n.starts_with('<')
                                            && !n.contains("@this")
                                            && !n.starts_with("get_")
                                            && !n.starts_with("set_")
                                    });
                            let mut call_args = self.with_this_arg(handle, namespace, call_args);
                            if plain_instance_method {
                                call_args.insert(0, Value::Object(namespace));
                            }
                            let result = self.invoke_function_value(handle, call_args)?;
                            let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                .unwrap_or_else(zun_core::Any::void);
                            return Ok(Value::Any(boxed));
                        }
                        cursor = zun_gc::with_current_heap(|heap| {
                            heap.dynamic_prototype(proto)
                                .and_then(|any| any.as_object())
                                .map(|a| unsafe { zun_gc::Obj::from_addr(a as *mut u64) })
                        });
                        depth += 1;
                        if depth > 64 {
                            break;
                        }
                    }
                }
                // A Promise reached through `any` (an awaited/dynamic value, an
                // async closure's result) answers `then`/`catch`/`finally`.
                if matches!(member.as_ref(), "then" | "catch" | "finally")
                    && zun_gc::with_current_heap(|heap| heap.promise_state(namespace)).is_some()
                {
                    let native = match member.as_ref() {
                        "then" => "promiseThen",
                        "catch" => "promiseCatch",
                        _ => "promiseFinally",
                    };
                    let mut native_args = vec![Value::Object(namespace)];
                    native_args.extend(call_args.iter().cloned());
                    let result = self.call_managed_collection(native, &native_args)?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                        .unwrap_or_else(zun_core::Any::void);
                    return Ok(Value::Any(boxed));
                }
                // A concrete array receiver dispatches its native methods
                // (`join`, `map`, `slice`, …) the same as a typed array, so an
                // array reached through `any` keeps its full method surface. The
                // array natives live in this same dispatcher under an
                // `array`-prefixed name (`arrayJoin`), matching the sema fallback.
                let is_array =
                    zun_gc::with_current_heap(|heap| heap.array_len(namespace).is_some());
                // The one authoritative array-method table (shared with the
                // typed sema path) names the member → native mapping and how
                // each method normalizes its arguments.
                let array_spec = zun_natives::array_method(member.as_ref());
                let array_handling = array_spec.map(|spec| spec.handling);
                if is_array && array_handling == Some(zun_natives::ArrayHandling::Push) {
                    // `push` has no `array*` native (typed code lowers it to an
                    // instruction); append directly and answer the new length.
                    if !call_args.is_empty() {
                        self.guard_array(namespace, crate::integrity::ArrayOp::Add)?;
                    }
                    let length = zun_gc::with_current_heap_mut(|heap| {
                        for value in &call_args {
                            // A callable has no `Any` form; it rides in a
                            // `functionBox` record exactly like the typed push.
                            let boxed = match value {
                                Value::Function(handle) => {
                                    let record = heap.alloc_dynamic();
                                    heap.dynamic_set_function(record, "__boxedFn", *handle);
                                    Value::Object(record).box_any(heap)
                                }
                                other => other.box_any(heap),
                            }
                            .unwrap_or_else(zun_core::Any::null);
                            let end = heap.array_len(namespace).unwrap_or(0);
                            heap.array_insert(namespace, end, boxed);
                        }
                        heap.array_len(namespace).unwrap_or(0)
                    });
                    return Ok(Value::Any(zun_core::Any::float(length as f64)));
                }
                if is_array && array_handling == Some(zun_natives::ArrayHandling::Pop) {
                    if zun_gc::with_current_heap(|heap| heap.array_len(namespace)) != Some(0) {
                        self.guard_array(namespace, crate::integrity::ArrayOp::Remove)?;
                    }
                    let removed = zun_gc::with_current_heap_mut(|heap| {
                        let length = heap.array_len(namespace).unwrap_or(0);
                        if length == 0 {
                            None
                        } else {
                            heap.array_remove(namespace, length - 1)
                        }
                    });
                    return Ok(Value::Any(removed.unwrap_or_else(zun_core::Any::void)));
                }
                // `arr.toString()` calls an own `join` the array took, as
                // `Array.prototype.toString` does (an own `toString` was
                // already called above).
                if is_array && member.as_ref() == "toString" {
                    if let Some(primitive) = self.array_own_primitive(namespace, "string")? {
                        let boxed = zun_gc::with_current_heap_mut(|heap| primitive.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                if is_array
                    && member.as_ref() == "toLocaleString"
                    && zun_gc::with_current_heap(|heap| heap.dynamic_get_function(namespace, "toLocaleString")).is_none()
                {
                    if let Some(rid) = self.free_function_rid("__zunArrayToLocaleString") {
                        let result = self.call(rid, vec![Value::Object(namespace)], None)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                if is_array {
                    use zun_natives::ArrayHandling;
                    // The base native name comes from the shared table; a member
                    // the table does not name keeps the open-set `array{Method}`
                    // fallback so any other member still resolves (or errors and
                    // falls through) exactly as before.
                    let mut native_name = match array_spec {
                        Some(spec) => spec.native.to_string(),
                        None => {
                            let mut chars = member.chars();
                            match chars.next() {
                                Some(first) => {
                                    format!("array{}{}", first.to_ascii_uppercase(), chars.as_str())
                                }
                                None => member.to_string(),
                            }
                        }
                    };
                    // `reduce` / `reduceRight` without an initial value seed
                    // from the first present element: a separate native, as in
                    // the typed form (an explicit `undefined` seed is a seed).
                    if call_args.len() == 1 {
                        match member.as_ref() {
                            "reduce" => native_name = "arrayReduceNoInit".to_string(),
                            "reduceRight" => native_name = "arrayReduceRightNoInit".to_string(),
                            _ => {}
                        }
                    }
                    let mut native_args = Vec::with_capacity(call_args.len() + 1);
                    native_args.push(Value::Object(namespace));
                    native_args.extend(call_args.iter().cloned());
                    // Methods whose typed form takes defaulted/variadic arguments
                    // are normalised to the matching runtime-argument natives,
                    // driven by the shared table's handling classification.
                    match array_handling {
                        // `sort` has two natives, chosen by the comparator's presence.
                        Some(ArrayHandling::Sort) => {
                            native_name = if call_args.is_empty() {
                                "arraySortDefault"
                            } else {
                                "arraySortBy"
                            }
                            .to_string();
                        }
                        Some(ArrayHandling::SliceRange) => {
                            native_args.resize(3, Value::Void);
                            for slot in native_args.iter_mut().skip(1) {
                                *slot = match slot.clone() {
                                    Value::Str(text) => Value::Float(
                                        text.trim().parse::<f64>().unwrap_or(f64::NAN),
                                        zun_il::NumKind::F64,
                                    ),
                                    Value::Bool(b) => {
                                        Value::Float(f64::from(u8::from(b)), zun_il::NumKind::F64)
                                    }
                                    Value::Null => Value::Float(0.0, zun_il::NumKind::F64),
                                    other => other,
                                };
                            }
                        }
                        Some(ArrayHandling::JoinDefault) => {
                            // A non-string separator runs ToString; an omitted or
                            // `undefined` one means a comma.
                            let separator = match native_args.get(1).cloned() {
                                Some(Value::Str(text)) => text,
                                None | Some(Value::Void) => ",".into(),
                                Some(other) => self.dyn_to_string(&other)?.into(),
                            };
                            native_args.truncate(1);
                            native_args.push(Value::Str(separator));
                        }
                        Some(ArrayHandling::ToStringJoin) => {
                            native_args.truncate(1);
                            native_args.push(Value::Str(",".into()));
                        }
                        Some(ArrayHandling::FillResize) => native_args.resize(4, Value::Void),
                        Some(ArrayHandling::ConcatList) => {
                            let list = zun_gc::with_current_heap_mut(|heap| {
                                let list = heap.alloc_array(0);
                                for value in &call_args {
                                    let boxed =
                                        value.box_any(heap).unwrap_or_else(zun_core::Any::void);
                                    heap.array_push(list, boxed);
                                }
                                list
                            });
                            native_args = vec![Value::Object(namespace), Value::Object(list)];
                        }
                        Some(ArrayHandling::ToSplicedCounts) => {
                            // `(array, start, deleteCount, items)`: the counts
                            // as integers, the variadic inserts packed.
                            let length = zun_gc::with_current_heap(|heap| {
                                heap.array_len(namespace).unwrap_or(0)
                            });
                            let start = match call_args.first().cloned() {
                                None | Some(Value::Void) => 0,
                                Some(other) => self.to_number_value(other)? as i64,
                            };
                            let start = if start < 0 {
                                (length as i64 + start).max(0)
                            } else {
                                start.min(length as i64)
                            };
                            let remaining = length as i64 - start;
                            // Per spec: with no arguments at all `skipCount` is
                            // 0, so `toSpliced()` is a full copy; only once
                            // `start` is present does an absent `skipCount`
                            // mean "to the end".
                            let delete_count = match call_args.get(1).cloned() {
                                None | Some(Value::Void) if call_args.is_empty() => 0,
                                None | Some(Value::Void) => remaining,
                                Some(other) => self.to_number_value(other)? as i64,
                            }
                            .clamp(0, remaining);
                            let items = zun_gc::with_current_heap_mut(|heap| {
                                let items = heap.alloc_array(0);
                                for value in call_args.iter().skip(2) {
                                    let boxed =
                                        value.box_any(heap).unwrap_or_else(zun_core::Any::void);
                                    heap.array_push(items, boxed);
                                }
                                items
                            });
                            native_args = vec![
                                Value::Object(namespace),
                                Value::Int(start, zun_il::NumKind::I64),
                                Value::Int(delete_count, zun_il::NumKind::I64),
                                Value::Object(items),
                            ];
                        }
                        Some(ArrayHandling::SpliceList) => {
                            let list = zun_gc::with_current_heap_mut(|heap| {
                                let list = heap.alloc_array(0);
                                for value in &call_args {
                                    let boxed =
                                        value.box_any(heap).unwrap_or_else(zun_core::Any::void);
                                    heap.array_push(list, boxed);
                                }
                                list
                            });
                            native_args = vec![Value::Object(namespace), Value::Object(list)];
                        }
                        _ => {}
                    }
                    match self.call_managed_collection(&native_name, &native_args) {
                        Ok(result) => {
                            let result =
                                self.apply_species(species_receiver.get(), &member, result)?;
                            let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any_cell(heap))
                                .unwrap_or_else(zun_core::Any::null);
                            return Ok(Value::Any(boxed));
                        }
                        // A refused mutation (frozen/sealed array) or a throwing
                        // callback must surface; other failures fall through.
                        Err(error @ RunError::Thrown(_)) => return Err(error),
                        Err(RunError::Native(message)) if message.starts_with("TypeError: ") => {
                            return Err(RunError::Native(message));
                        }
                        Err(_) => {}
                    }
                }
                // A Map / Set reached through `any` answers its collection methods
                // through the same natives typed code uses.
                {
                    let (is_map, is_set) = zun_gc::with_current_heap(|heap| {
                        (
                            heap.map_len(namespace).is_some(),
                            heap.set_len(namespace).is_some(),
                        )
                    });
                    // ES2025 Set operations run the prelude helper that follows
                    // the set-like protocol on the argument. The seven member
                    // names and their helper selection come from the one
                    // authoritative `zun_natives` Set-operation table shared
                    // with the typed sema path.
                    if let Some(set_op) = is_set
                        .then(|| zun_natives::set_operation(member.as_ref()))
                        .flatten()
                    {
                        if let Some(rid) = self.free_function_rid(set_op.helper()) {
                            let other = call_args
                                .first()
                                .and_then(|value| {
                                    zun_gc::with_current_heap_mut(|heap| value.box_any_cell(heap))
                                })
                                .unwrap_or_else(zun_core::Any::void);
                            let helper_args = vec![
                                Value::Object(namespace),
                                Value::Str(member.as_ref().into()),
                                Value::Any(other),
                            ];
                            let result = self.call(rid, helper_args, None)?;
                            let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                .unwrap_or_else(zun_core::Any::void);
                            return Ok(Value::Any(boxed));
                        }
                    }
                    // A WeakMap / WeakSet reached through `any`. WeakSet and WeakMap
                    // share a store; `add` / `set` tell them apart by name.
                    if !is_map
                        && !is_set
                        && zun_gc::with_current_heap(|heap| heap.is_weak_collection(namespace))
                    {
                        let weak_native = match member.as_ref() {
                            "set" => Some("weakMapSet"),
                            "get" => Some("weakMapGet"),
                            "has" => Some("weakMapHas"),
                            "delete" => Some("weakMapDelete"),
                            "add" => Some("weakSetAdd"),
                            _ => None,
                        };
                        if let Some(native) = weak_native {
                            let mut native_args = Vec::with_capacity(call_args.len() + 1);
                            native_args.push(Value::Object(namespace));
                            native_args.extend(call_args.iter().cloned());
                            if native == "weakMapSet" {
                                native_args.resize(3, Value::Void);
                            }
                            let result = self.call_managed_collection(native, &native_args)?;
                            let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                .unwrap_or_else(zun_core::Any::void);
                            return Ok(Value::Any(boxed));
                        }
                    }
                    // The member → native mapping (and the `mapSetChain`
                    // argument reshaping) comes from the one authoritative
                    // `zun_natives` collection-method table shared with the
                    // typed sema path.
                    let spec = if is_map {
                        zun_natives::map_method(member.as_ref())
                    } else if is_set {
                        zun_natives::set_method(member.as_ref())
                    } else {
                        None
                    };
                    if let Some(spec) = spec {
                        let mut native_args = Vec::with_capacity(call_args.len() + 1);
                        native_args.push(Value::Object(namespace));
                        native_args.extend(call_args.iter().cloned());
                        if spec.handling == zun_natives::CollectionHandling::SetChain {
                            native_args.resize(3, Value::Void);
                        }
                        let result = self.call_managed_collection(spec.native, &native_args)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                }
                // A typed class instance reached through `any`: dispatch the
                // named method (virtually) with the instance as receiver.
                if !is_array
                    && !zun_gc::with_current_heap(|heap| {
                        heap.is_dynamic_object(namespace) && !heap.is_instance_bag(namespace)
                    })
                {
                    if let Some(rid) = self.instance_method_rid(namespace, &member) {
                        let mut method_args = Vec::with_capacity(call_args.len() + 1);
                        method_args.push(Value::Object(namespace));
                        method_args.extend(call_args.iter().cloned());
                        let method_args = self.pack_rest_call_args(rid, method_args);
                        // A generator method mints its iterator instead of running.
                        let result = if self.method_contains_yield(rid) {
                            self.create_generator(rid, &method_args)?
                        } else {
                            self.call(rid, method_args, None)?
                        };
                        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                            .unwrap_or_else(zun_core::Any::void);
                        return Ok(Value::Any(boxed));
                    }
                    // A function stored on `Class.prototype` (or a base class's)
                    // at run time (`C.prototype[key] = fn`) is found by walking
                    // the class prototype chain; it runs with the instance as `this`.
                    if let Some(first) = self.instance_class_prototype(namespace) {
                        let mut current = Some(first);
                        for _ in 0..256 {
                            let Some(proto) = current else { break };
                            if let Some(handle) = zun_gc::with_current_heap(|heap| {
                                heap.dynamic_get_function(proto, &member)
                            }) {
                                let result = self.invoke_function_value(
                                    handle,
                                    self.with_this_arg(handle, namespace, call_args),
                                )?;
                                let boxed =
                                    zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                        .unwrap_or_else(zun_core::Any::void);
                                return Ok(Value::Any(boxed));
                            }
                            current = zun_gc::with_current_heap(|heap| {
                                heap.dynamic_prototype(proto)
                                    .and_then(|any| any.as_object())
                                    .map(|address| unsafe {
                                        zun_gc::Obj::from_addr(address as *mut u64)
                                    })
                            });
                        }
                    }
                }
                // A dynamically-imported class instance (built by
                // `construct_namespace_class`) keeps its real typed instance in a
                // hidden `__instance` slot and mirrors its methods as module
                // functions. Route a method call to the real instance in its
                // defining scope (the method runs on the current heap, so an
                // object result is valid — unlike the namespace-function path).
                let is_instance =
                    zun_gc::with_current_heap(|heap| heap.dynamic_get(namespace, "__instance"))
                        .is_some();
                if is_instance {
                    let loader = self.loader.ok_or(RunError::BadToken)?;
                    let result = crate::namespace::call_namespace_instance_method(
                        loader,
                        &Value::Object(namespace),
                        &member,
                        call_args,
                        &mut *self.output,
                    )?;
                    let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any_cell(heap))
                        .unwrap_or_else(zun_core::Any::null);
                    return Ok(Value::Any(boxed));
                }
                let is_module_member = zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get_module_function(namespace, &member)
                        .is_some()
                });
                if !is_module_member {
                    // An `Object.prototype` method inherited by a plain object.
                    // Reading `o.valueOf` already answers a receiver-bound
                    // stand-in (see `dyn_member`); calling `o.valueOf()`
                    // directly reaches here instead, so it builds the same
                    // stand-in rather than a second implementation.
                    if let Some(mode) = crate::object_proto_mode(&member) {
                        // A typed instance inherits them from `Object.prototype` too.
                        let typed_instance = self.instance_class_prototype(namespace).is_some();
                        if typed_instance
                            || zun_gc::with_current_heap(|heap| {
                                heap.is_dynamic_object(namespace)
                                    && heap.array_len(namespace).is_none()
                            })
                        {
                            let handle = self.make_promise_settler(namespace, mode);
                            let result = self.invoke_function_value(handle, call_args)?;
                            let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap))
                                .unwrap_or_else(zun_core::Any::null);
                            return Ok(Value::Any(boxed));
                        }
                    }
                    // A callable *property* -- not a method -- reached as a
                    // member call. `class A { f: any }` holding an arrow was
                    // readable (`const g = a.f; g()`) and callable through
                    // `a.f.call(..)`, but `a.f()` on an `any` receiver came
                    // here and threw `f is not a function` while `typeof a.f`
                    // answered `"function"`: lists-and-does-not-call, the
                    // mirror of the shape this engine keeps producing. A
                    // function in an `any` field is stored as a cell, which
                    // `unbox_any` resolves, so reading the property and
                    // invoking it is all that was missing.
                    let property = self
                        .instance_member_get(namespace, &member)?
                        .or_else(|| {
                            zun_gc::with_current_heap(|heap| heap.dynamic_get(namespace, &member))
                                .and_then(|any| {
                                    zun_gc::with_current_heap(|heap| {
                                        Value::unbox_any(any, heap)
                                    })
                                })
                        });
                    if let Some(handle) =
                        property.as_ref().and_then(super::callback_handle)
                    {
                        let result = self.invoke_function_value(handle, call_args)?;
                        let boxed = zun_gc::with_current_heap_mut(|heap| {
                            result.box_any_cell(heap)
                        })
                        .unwrap_or_else(zun_core::Any::null);
                        return Ok(Value::Any(boxed));
                    }
                    // Neither a method nor a namespace export: the member is
                    // missing or not callable.
                    if OPTIONAL_PROBE.with(|flag| flag.get()) {
                        return Err(RunError::BadToken);
                    }
                    return match self
                        .new_error_object("TypeError", &format!("{member} is not a function"))
                    {
                        Some(object) => Err(RunError::Thrown(Value::Object(object))),
                        None => Err(RunError::BadToken),
                    };
                }
                let loader = self.loader.ok_or(RunError::BadToken)?;
                let result = crate::namespace::call_namespace_member(
                    loader,
                    &Value::Object(namespace),
                    &member,
                    call_args,
                    &mut *self.output,
                )?;
                if matches!(result, Value::Object(_)) {
                    // The callee ran on its own heap; references cannot cross.
                    return Err(RunError::BadToken);
                }
                Ok(Value::Any(any(&result).ok_or(RunError::BadToken)?))
            }
            _ => Err(RunError::BadToken),
        }
    }
}
