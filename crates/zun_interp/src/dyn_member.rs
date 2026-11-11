//! Cold dynamic member opcode arms extracted from the Tier 0 dispatch loop.
//!
//! Each method mirrors exactly one `match instr.opcode` arm from
//! [`Interp::call_inner`]; it borrows the same live frame storage so the GC
//! still scans and relocates through the one set of raw pointers.

use super::*;
use crate::throw_native_arm;
use zun_il::Instruction;

impl Interp<'_> {
    #[inline(always)]
    pub(super) fn op_load_dynamic(
        &mut self,
        stack: &mut Vec<Value>,
        pc: &mut usize,
        body: &zun_module::MethodBody<'_>,
        instr: Instruction,
        next: usize,
        size: usize,
    ) -> Result<ArmFlow, RunError> {
        let name = self.dynamic_name(&instr.operand)?.to_owned();
        let popped = pop(stack)?;
        if name == "length" || name == "name" {
            let callee = match &popped {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)),
                other => Some(other.clone()),
            };
            let callee = callee.map(|value| {
                zun_gc::with_current_heap(|heap| {
                    crate::collections::unbox_function_box(value, heap)
                })
            });
            if let Some(Value::Function(handle)) = callee {
                if let Some(meta) = self.function_meta(handle, &name) {
                    stack.push(meta);
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        let raw = expando_receiver(popped);
        // A dynamic receiver that unboxes to a primitive string or an
        // array exposes `.length` the same as the typed forms, so a
        // value read through `any` (e.g. a module global) keeps it.
        let concrete = match &raw {
            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                .unwrap_or(Value::Void),
            other => other.clone(),
        };
        if matches!(concrete, Value::Void | Value::Null) {
            let kind = if matches!(concrete, Value::Null) {
                "null"
            } else {
                "undefined"
            };
            let message = format!("Cannot read properties of {kind} (reading '{name}')");
            match self.new_error_object("TypeError", &message) {
                Some(error) => {
                    let exception = Value::Object(error);
                    if self.route_exception(body, stack, pc, next - size, &exception) {
                        return Ok(ArmFlow::Continue);
                    }
                    return Err(RunError::Thrown(exception));
                }
                None => return Err(RunError::Native(format!("TypeError: {message}"))),
            }
        }
        // A Proxy reached through `any`: reads go through its `get` trap.
        if let Value::Object(obj) = &concrete {
            if zun_gc::with_current_heap(|heap| heap.proxy_target(*obj).is_some()) {
                match self.proxy_dynamic_get(*obj, &name) {
                    Ok(result) => {
                        stack.push(result);
                        return Ok(ArmFlow::Continue);
                    }
                    Err(RunError::Thrown(exception)) => {
                        if self.route_exception(body, stack, pc, next - size, &exception) {
                            return Ok(ArmFlow::Continue);
                        }
                        return Err(RunError::Thrown(exception));
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        // `gen.next` / `gen.return` / `gen.throw` read off a generator
        // object are callable method values.
        if matches!(name.as_str(), "next" | "return" | "throw") {
            if let Value::Object(obj) = &concrete {
                if zun_gc::with_current_heap(|heap| {
                    heap.generator_state(*obj).is_some()
                        || (name == "next" && heap.array_iterator_done(*obj).is_some())
                }) {
                    let kind = match name.as_str() {
                        "next" => 0,
                        "return" => 1,
                        _ => 2,
                    };
                    stack.push(Value::Function(
                        zun_core::FunctionHandle::new_generator_method(kind, obj.addr() as usize),
                    ));
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        // `symbol.description` read through `any`.
        if name == "description" {
            if let Value::Any(any) = &concrete {
                if let Some(id) = any.as_symbol() {
                    let description = zun_gc::with_current_heap(|heap| {
                        heap.symbol_description(id).map(|text| text.to_string())
                    });
                    stack.push(match description {
                        Some(text) => Value::Str(text.into()),
                        None => Value::Void,
                    });
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        // BigInt / Symbol primitives name their wrapper class.
        if name == "constructor" {
            let is_bigint = matches!(&concrete, Value::Object(obj)
                            if zun_gc::with_current_heap(|heap| heap.bigint_value(*obj).is_some()));
            let is_symbol = matches!(&concrete, Value::Any(any) if any.as_symbol().is_some());
            let wrapper = if is_bigint {
                Some("BigInt")
            } else if is_symbol {
                Some("Symbol")
            } else {
                None
            };
            if let Some(class_name) = wrapper {
                let record = Self::builtin_constructor_record(class_name);
                stack.push(Value::Object(record));
                return Ok(ArmFlow::Continue);
            }
        }
        if name == "constructor" {
            if let Value::Object(obj) = &concrete {
                if let Some(record) = self.constructor_record(*obj) {
                    stack.push(Value::Object(record));
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        // `prim.constructor` is a record naming the wrapper class.
        if name == "constructor" {
            let wrapper = match &concrete {
                Value::Str(_) => Some("String"),
                Value::Bool(_) => Some("Boolean"),
                Value::Float(..) | Value::Int(..) => Some("Number"),
                _ => None,
            };
            if let Some(class_name) = wrapper {
                let record = Self::builtin_constructor_record(class_name);
                stack.push(Value::Object(record));
                return Ok(ArmFlow::Continue);
            }
        }
        if name == "length" {
            match &concrete {
                Value::Str(text) => {
                    stack.push(Value::Float(
                        zun_core::wtf16::len16(text) as f64,
                        zun_il::NumKind::F64,
                    ));
                    return Ok(ArmFlow::Continue);
                }
                Value::Object(obj) => {
                    if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(*obj)) {
                        stack.push(Value::Float(len as f64, zun_il::NumKind::F64));
                        return Ok(ArmFlow::Continue);
                    }
                }
                _ => {}
            }
        }
        // `size` of a Map / Set reached through `any`.
        if name == "size" {
            if let Value::Object(obj) = &concrete {
                if let Some(size) = zun_gc::with_current_heap(|heap| {
                    heap.map_len(*obj).or_else(|| heap.set_len(*obj))
                }) {
                    stack.push(Value::Float(size as f64, zun_il::NumKind::F64));
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        // Any other named read off a primitive string / number / boolean has no
        // own properties here: it yields `undefined` rather than faulting.
        if matches!(
            concrete,
            Value::Str(_) | Value::Int(..) | Value::Float(..) | Value::Bool(_)
        ) {
            stack.push(Value::Void);
            return Ok(ArmFlow::Continue);
        }
        // A RegExp object read through `any`: the flag accessors
        // and `source`/`flags`/`lastIndex`.
        if let Value::Object(regex) = &concrete {
            let info = zun_gc::with_current_heap(|heap| {
                heap.regex_source(*regex).map(|source| {
                    (
                        source.to_owned(),
                        heap.regex_flags(*regex).unwrap_or("").to_owned(),
                    )
                })
            });
            if let Some((source, flags)) = info {
                let flag_of = |c: char| Some(Value::Bool(flags.contains(c)));
                let answer = match name.as_str() {
                    "source" => Some(Value::Str(source.into())),
                    "flags" => Some(Value::Str(
                        "dgimsuvy"
                            .chars()
                            .filter(|c| flags.contains(*c))
                            .collect::<String>()
                            .into(),
                    )),
                    "global" => flag_of('g'),
                    "ignoreCase" => flag_of('i'),
                    "multiline" => flag_of('m'),
                    "dotAll" => flag_of('s'),
                    "unicode" => flag_of('u'),
                    "unicodeSets" => flag_of('v'),
                    "sticky" => flag_of('y'),
                    "hasIndices" => flag_of('d'),
                    _ => None,
                };
                if let Some(answer) = answer {
                    stack.push(answer);
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        // `Class.prototype` on a class descriptor reached through
        // `any` is the descriptor itself (identity); other reads
        // of a descriptor yield undefined.
        if let Value::Any(descriptor) = &concrete {
            if descriptor.as_class_descriptor().is_some() {
                let token = descriptor.as_class_descriptor().unwrap_or(0);
                let user_class = token & zun_natives::USER_CLASS_FLAG != 0
                    && ((token & !zun_natives::USER_CLASS_FLAG) as usize) >= 1
                    && ((token & !zun_natives::USER_CLASS_FLAG) as usize)
                        <= self.module.tables.type_defs.len();
                if user_class && name != "prototype" {
                    let rid = token & !zun_natives::USER_CLASS_FLAG;
                    match self.class_static_read(rid, &name) {
                        Ok(Some(value)) => {
                            stack.push(value);
                            return Ok(ArmFlow::Continue);
                        }
                        Ok(None) => {}
                        Err(RunError::Thrown(exception)) => {
                            if self.route_exception(body, stack, pc, next - size, &exception) {
                                return Ok(ArmFlow::Continue);
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(error) => return Err(error),
                    }
                }
                stack.push(if name == "prototype" && user_class {
                    let proto = self.class_prototype_object(token & !zun_natives::USER_CLASS_FLAG);
                    Value::Any(zun_core::Any::object(proto.addr() as usize))
                } else if name == "prototype" {
                    Value::Any(*descriptor)
                } else {
                    Value::Void
                });
                return Ok(ArmFlow::Continue);
            }
        }
        // A primitive number/boolean/undefined/null receiver has no own
        // properties here: the read yields `undefined`.
        if matches!(
            concrete,
            Value::Bool(_) | Value::Int(..) | Value::Float(..) | Value::Void | Value::Null
        ) {
            stack.push(Value::Void);
            return Ok(ArmFlow::Continue);
        }
        let object = concrete.as_object();
        // An accessor property invokes its getter (NUCLEUS §6);
        // a data property (or an absent key) reads the value slot.
        let getter = zun_gc::with_current_heap(|heap| heap.dynamic_accessor_getter(object, &name));
        // A typed class instance read through `any` resolves a
        // registered getter (`get_<name>`, e.g. `Error.name`).
        let typed_getter = if getter.is_none() {
            self.class_named_methods
                .get(&object.type_rid())
                .and_then(|methods| methods.get(&format!("get_{name}")))
                .copied()
        } else {
            None
        };
        let value = if let Some(getter) = getter {
            match self.invoke_function_value(getter, self.with_this_arg(getter, object, Vec::new()))
            {
                Ok(v) => v,
                Err(RunError::Thrown(exception)) => {
                    if self.route_exception(body, stack, pc, next - size, &exception) {
                        return Ok(ArmFlow::Continue);
                    }
                    return Err(RunError::Thrown(exception));
                }
                Err(error) => return Err(error),
            }
        } else if let Some(getter) = typed_getter {
            match self.invoke_function_value(getter, vec![Value::Object(object)]) {
                Ok(v) => v,
                Err(RunError::Thrown(exception)) => {
                    if self.route_exception(body, stack, pc, next - size, &exception) {
                        return Ok(ArmFlow::Continue);
                    }
                    return Err(RunError::Thrown(exception));
                }
                Err(error) => return Err(error),
            }
        } else {
            zun_gc::with_current_heap(|heap| {
                if let Some(any) = heap.dynamic_get(object, &name) {
                    Value::Any(any)
                } else if let Some(handle) = heap.dynamic_get_function(object, &name) {
                    // No data slot, but a function slot exists (an
                    // object method, or a `Promise.withResolvers`
                    // settler). Read it out as a callable value so a
                    // binding such as `const { resolve } = ...` is
                    // callable (function values do not box to `Any`).
                    Value::Function(handle)
                } else {
                    Value::Void
                }
            })
        };
        // A typed class instance reached through `any`: read its
        // declared field or getter by name.
        let value = if matches!(value, Value::Void) {
            match self.instance_member_get(object, &name)? {
                Some(member) => match zun_gc::with_current_heap_mut(|heap| member.box_any(heap)) {
                    Some(any) => Value::Any(any),
                    None => member,
                },
                None => Value::Void,
            }
        } else {
            value
        };
        // An inherited property: walk the explicit prototype chain
        // (`Object.create(proto)`); getters run with the original
        // receiver as `this`.
        let value = if matches!(value, Value::Void) {
            match self.prototype_chain_get(object, &name)? {
                Some(inherited) => inherited,
                None => Value::Void,
            }
        } else {
            value
        };
        // An inherited `Object.prototype` method read off a plain dynamic
        // object is a callable function value.
        let value = if matches!(value, Value::Void)
            && matches!(
                name.as_str(),
                "toString"
                    | "toLocaleString"
                    | "valueOf"
                    | "hasOwnProperty"
                    | "isPrototypeOf"
                    | "propertyIsEnumerable"
            )
            && zun_gc::with_current_heap(|heap| {
                heap.is_dynamic_object(object)
                    && !heap.is_instance_bag(object)
                    && heap.array_len(object).is_none()
                    && !matches!(heap.dynamic_prototype(object), Some(p) if p.tag() == zun_core::AnyTag::Null)
            }) {
            let mode = 100.0
                + [
                    "toString",
                    "toLocaleString",
                    "valueOf",
                    "hasOwnProperty",
                    "isPrototypeOf",
                    "propertyIsEnumerable",
                ]
                .iter()
                .position(|n| *n == name.as_str())
                .unwrap_or(0) as f64;
            Value::Function(self.make_promise_settler(object, mode))
        } else {
            value
        };
        stack.push(value);
        Ok(ArmFlow::Next)
    }

    #[inline(always)]
    pub(super) fn op_store_dynamic(
        &mut self,
        mut stack: &mut Vec<Value>,
        mut pc: &mut usize,
        body: &zun_module::MethodBody<'_>,
        instr: Instruction,
        next: usize,
        size: usize,
    ) -> Result<ArmFlow, RunError> {
        let name = self.dynamic_name(&instr.operand)?.to_owned();
        let value = pop(stack)?;
        // A class value: the write lands on its static field.
        if let Some(Value::Any(descriptor)) = stack.last() {
            let descriptor = *descriptor;
            if let Some(rid) = descriptor
                .as_class_descriptor()
                .and_then(|t| self.user_class_rid(t))
            {
                stack.pop();
                self.class_static_write(rid, &name, &value);
                stack.push(value);
                return Ok(ArmFlow::Continue);
            }
        }
        // A Proxy reached through `any`: writes go through its `set` trap.
        {
            let receiver = expando_receiver(stack.last().cloned().ok_or(RunError::BadToken)?);
            let proxy = match &receiver {
                Value::Object(obj) => Some(*obj),
                Value::Any(any) => any
                    .as_object()
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
                _ => None,
            }
            .filter(|obj| zun_gc::with_current_heap(|heap| heap.proxy_target(*obj).is_some()));
            if let Some(proxy) = proxy {
                stack.pop();
                match self.proxy_dynamic_set(proxy, &name, value.clone()) {
                    Ok(()) => {
                        stack.push(value);
                        return Ok(ArmFlow::Continue);
                    }
                    Err(RunError::Thrown(exception)) => {
                        if self.route_exception(body, stack, pc, next - size, &exception) {
                            return Ok(ArmFlow::Continue);
                        }
                        return Err(RunError::Thrown(exception));
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        if let Value::Function(handle) = value {
            // Function values do not box to `Any`, so they ride the
            // object's function slot (a closure environment is a
            // DynamicObject); `LoadDynamic`'s function-slot fallback
            // reads them back as `Value::Function`. Capture names are
            // never accessors, so no accessor lookup happens here.
            let object = expando_receiver(pop(stack)?).as_object();
            // An array gets an expando table on first named store.
            zun_gc::with_current_heap_mut(|heap| {
                if heap.array_len(object).is_some() {
                    heap.ensure_dynamic_entry(object);
                }
            });
            zun_gc::with_current_heap_mut(|heap| {
                if heap.dynamic_get_function(object, &name).is_none() {
                    heap.dynamic_remove(object, &name);
                }
                heap.dynamic_set_function(object, name, handle);
            });
            stack.push(Value::Function(handle));
        } else {
            // Any non-function value boxes into `Any` for the slot
            // (a primitive assigned to an `any`-typed local reaches
            // here unboxed, e.g. `o.k = 5.0` or `o.k = null`).
            let any = zun_gc::with_current_heap_mut(|heap| value.box_any(heap))
                .ok_or(RunError::BadToken)?;
            let object = expando_receiver(pop(stack)?).as_object();
            // `arr.length = n` on an array reached through `any`.
            if name == "length" {
                let level_len = zun_gc::with_current_heap(|heap| {
                    heap.array_len(object)
                        .map(|len| (len, heap.array_integrity(object)))
                });
                if let Some((len, level)) = level_len {
                    let wanted = any.as_int().map(|n| n as f64).or_else(|| any.as_float());
                    if let Some(wanted) = wanted {
                        let wanted = wanted as usize;
                        if wanted != len {
                            let op = if wanted < len {
                                integrity::ArrayOp::Remove
                            } else {
                                integrity::ArrayOp::Write
                            };
                            if let Some(message) = integrity::array_refusal(level, op) {
                                throw_native_arm!(
                                    self,
                                    body,
                                    stack,
                                    pc,
                                    next - size,
                                    message.to_string()
                                );
                            }
                            zun_gc::with_current_heap_mut(|heap| heap.array_resize(object, wanted));
                        }
                    }
                    stack.push(value);
                    return Ok(ArmFlow::Continue);
                }
            }
            let refusal = zun_gc::with_current_heap(|heap| {
                if heap.dynamic_is_accessor(object, &name) {
                    if heap.dynamic_accessor_setter(object, &name).is_none() {
                        Some(format!("TypeError: Cannot set property {name} of #<Object> which has only a getter"))
                    } else {
                        None
                    }
                } else if !heap.dynamic_can_set(object, &name) {
                    if heap.dynamic_has(object, &name) {
                        Some(format!("TypeError: Cannot assign to read only property '{name}' of object '#<Object>'"))
                    } else {
                        Some(format!(
                            "TypeError: Cannot add property {name}, object is not extensible"
                        ))
                    }
                } else {
                    None
                }
            });
            if let Some(message) = refusal {
                throw_native_arm!(self, body, stack, pc, next - size, message);
            }
            // An accessor property invokes its setter; a data property
            // writes the value slot (rejected silently if non-writable).
            let setter = zun_gc::with_current_heap(|heap| {
                if heap.dynamic_is_accessor(object, &name) {
                    heap.dynamic_accessor_setter(object, &name)
                } else {
                    None
                }
            });
            let is_accessor =
                zun_gc::with_current_heap(|heap| heap.dynamic_is_accessor(object, &name));
            if is_accessor {
                if let Some(setter) = setter {
                    match self.invoke_function_value(
                        setter,
                        self.with_this_arg(setter, object, vec![Value::Any(any)]),
                    ) {
                        Ok(_) => {}
                        Err(RunError::Thrown(exception)) => {
                            if self.route_exception(body, stack, pc, next - size, &exception) {
                                return Ok(ArmFlow::Continue);
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(error) => return Err(error),
                    }
                }
            } else {
                // A typed class instance: a declared field takes the
                // value in place; any other name becomes an expando.
                let typed_instance = zun_gc::with_current_heap(|heap| {
                    !heap.is_dynamic_object(object)
                        && heap.array_len(object).is_none()
                        && object.type_rid() >= 1
                });
                let handled = typed_instance && self.instance_member_set(object, &name, &value);
                // An accessor defined on the class prototype chain
                // (`Object.defineProperty(C.prototype, ...)`) takes the write.
                let proto_accessor = if typed_instance && !handled {
                    self.proto_chain_accessor_setter(object, &name)
                } else {
                    None
                };
                if let Some(setter) = proto_accessor {
                    if let Some(setter) = setter {
                        match self.invoke_function_value(
                            setter,
                            self.with_this_arg(setter, object, vec![Value::Any(any)]),
                        ) {
                            Ok(_) => {}
                            Err(RunError::Thrown(exception)) => {
                                if self.route_exception(body, stack, pc, next - size, &exception) {
                                    return Ok(ArmFlow::Continue);
                                }
                                return Err(RunError::Thrown(exception));
                            }
                            Err(error) => return Err(error),
                        }
                    }
                } else if !handled {
                    zun_gc::with_current_heap_mut(|heap| {
                        if heap.array_len(object).is_some() {
                            heap.ensure_dynamic_entry(object);
                        } else if typed_instance {
                            heap.ensure_instance_bag(object);
                        } else if !heap.is_dynamic_object(object) {
                            // A Map, Set, RegExp, typed array ...: the write
                            // becomes an own expando property.
                            heap.ensure_dynamic_entry(object);
                        }
                        heap.dynamic_assign(object, name, any);
                    });
                }
            }
            stack.push(value);
        }
        Ok(ArmFlow::Next)
    }

    #[inline(always)]
    pub(super) fn op_load_dynamic_value(
        &mut self,
        stack: &mut Vec<Value>,
        pc: &mut usize,
        body: &zun_module::MethodBody<'_>,
        next: usize,
        size: usize,
    ) -> Result<ArmFlow, RunError> {
        // A Symbol key reads the symbol-keyed slot; a string key the
        // ordinary dynamic property.
        let key = pop(stack)?;
        let key = match key {
            Value::Any(any) if any.as_symbol().is_none() => {
                zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)).unwrap_or(Value::Void)
            }
            other => other,
        };
        // ToPropertyKey: booleans, null/undefined and objects become
        // their string form (objects run toString first).
        let key = if matches!(
            key,
            Value::Bool(_) | Value::Null | Value::Void | Value::Object(_)
        ) {
            match self.dyn_to_string(&key) {
                Ok(text) => Value::Str(text.into()),
                Err(RunError::Thrown(exception)) => {
                    if self.route_exception(body, stack, pc, next - size, &exception) {
                        return Ok(ArmFlow::Continue);
                    }
                    return Err(RunError::Thrown(exception));
                }
                Err(error) => return Err(error),
            }
        } else {
            key
        };
        let object_raw = expando_receiver(pop(stack)?);
        {
            let concrete = match &object_raw {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                    .unwrap_or(Value::Void),
                other => other.clone(),
            };
            if matches!(concrete, Value::Void | Value::Null) {
                let kind = if matches!(concrete, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                let message = format!("Cannot read properties of {kind}");
                match self.new_error_object("TypeError", &message) {
                    Some(error) => {
                        let exception = Value::Object(error);
                        if self.route_exception(body, stack, pc, next - size, &exception) {
                            return Ok(ArmFlow::Continue);
                        }
                        return Err(RunError::Thrown(exception));
                    }
                    None => return Err(RunError::Native(format!("TypeError: {message}"))),
                }
            }
        }
        // A string receiver: `str[i]` is the character, `str.length` its length.
        {
            let concrete = match &object_raw {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                    .unwrap_or(Value::Void),
                other => other.clone(),
            };
            if let Value::Str(text) = &concrete {
                let answer = match &key {
                    Value::Int(..) | Value::Float(..) => {
                        let at = match &key {
                            Value::Int(n, _) => *n as f64,
                            other => other.as_number(),
                        };
                        if at >= 0.0 && at.fract() == 0.0 {
                            zun_core::wtf16::encode(text)
                                .nth(at as usize)
                                .map(|u| Value::Str(zun_core::wtf16::from_units(&[u]).into()))
                        } else {
                            None
                        }
                    }
                    Value::Str(name) if &**name == "length" => Some(Value::Float(
                        zun_core::wtf16::len16(text) as f64,
                        zun_il::NumKind::F64,
                    )),
                    Value::Str(name) => name
                        .parse::<usize>()
                        .ok()
                        .and_then(|at| zun_core::wtf16::encode(text).nth(at))
                        .map(|u| Value::Str(zun_core::wtf16::from_units(&[u]).into())),
                    _ => None,
                };
                stack.push(answer.unwrap_or(Value::Void));
                return Ok(ArmFlow::Continue);
            }
        }
        if matches!(
            object_raw,
            Value::Int(..) | Value::Float(..) | Value::Bool(_)
        ) || matches!(&object_raw, Value::Any(a) if matches!(zun_gc::with_current_heap(|heap| Value::unbox_any(*a, heap)), Some(Value::Int(..) | Value::Float(..) | Value::Bool(_))))
        {
            stack.push(Value::Void);
            return Ok(ArmFlow::Continue);
        }
        // `Class[Symbol.species]`: the class's static species getter, else
        // the constructor itself (the default `get [Symbol.species]() { return this; }`).
        // A `{ name, __classDesc }` constructor record stands for its class.
        let species_receiver = match &object_raw {
            Value::Any(any) if any.tag() == zun_core::AnyTag::ClassDescriptor => Some(*any),
            Value::Any(any) => any.as_object().and_then(|address| {
                let obj = unsafe { zun_gc::Obj::from_addr(address as *mut u64) };
                zun_gc::with_current_heap(|heap| {
                    if heap.is_dynamic_object(obj) {
                        heap.dynamic_get(obj, "__classDesc")
                    } else {
                        None
                    }
                })
            }),
            Value::Object(obj) => zun_gc::with_current_heap(|heap| {
                if heap.is_dynamic_object(*obj) {
                    heap.dynamic_get(*obj, "__classDesc")
                } else {
                    None
                }
            }),
            _ => None,
        };
        if let (Some(descriptor), Value::Any(key_any)) = (&species_receiver, &key) {
            if descriptor.tag() == zun_core::AnyTag::ClassDescriptor
                && key_any.as_symbol().is_some_and(|symbol| {
                    symbol.get() == zun_core::WellKnownSymbol::Species.id().get()
                })
            {
                let species_symbol = key_any.as_symbol().map_or(0, |symbol| symbol.get());
                let handle = descriptor
                    .as_class_descriptor()
                    .and_then(|token| self.user_class_rid(token))
                    .and_then(|rid| {
                        self.class_symbol_methods
                            .get(&(rid, species_symbol))
                            .copied()
                    });
                match handle {
                    Some(handle) => match self.invoke_function_value(handle, Vec::new()) {
                        Ok(value) => stack.push(value),
                        Err(RunError::Thrown(exception)) => {
                            if self.route_exception(body, stack, pc, next - size, &exception) {
                                return Ok(ArmFlow::Continue);
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(error) => return Err(error),
                    },
                    None => stack.push(Value::Any(*descriptor)),
                }
                return Ok(ArmFlow::Continue);
            }
        }
        // A builtin class descriptor (e.g. `Array.prototype`) has no
        // dynamic properties to read: the lookup yields `undefined`.
        if matches!(&object_raw, Value::Any(a) if a.tag() == zun_core::AnyTag::ClassDescriptor) {
            stack.push(Value::Void);
            return Ok(ArmFlow::Continue);
        }
        let object = object_raw.as_object();
        // A numeric key on a dynamic object is the canonical string key, so
        // accessors and the ordinary property path apply (`o[0]` -> "0").
        let key = if matches!(key, Value::Int(..) | Value::Float(..))
            && zun_gc::with_current_heap(|heap| heap.is_dynamic_object(object))
        {
            Value::Str(number_index_key(&key).into())
        } else {
            key
        };
        // `gen["next" | "return" | "throw"]` read off a generator object
        // is a callable method value, like the static-name read.
        if let Value::Str(name) = &key {
            if matches!(&**name, "next" | "return" | "throw")
                && zun_gc::with_current_heap(|heap| {
                    heap.generator_state(object).is_some()
                        || (&**name == "next" && heap.array_iterator_done(object).is_some())
                })
            {
                let kind = match &**name {
                    "next" => 0,
                    "return" => 1,
                    _ => 2,
                };
                stack.push(Value::Function(
                    zun_core::FunctionHandle::new_generator_method(kind, object.addr() as usize),
                ));
                return Ok(ArmFlow::Continue);
            }
        }
        let value = if let Value::Any(any) = &key {
            if let Some(symbol) = any.as_symbol() {
                if let Some(getter) =
                    zun_gc::with_current_heap(|heap| heap.dynamic_symbol_getter(object, symbol))
                {
                    let call_args = self.with_this_arg(getter, object, Vec::new());
                    match self.invoke_function_value(getter, call_args) {
                        Ok(v) => v,
                        Err(RunError::Thrown(exception)) => {
                            if self.route_exception(body, stack, pc, next - size, &exception) {
                                return Ok(ArmFlow::Continue);
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    let own = zun_gc::with_current_heap(|heap| {
                        heap.dynamic_get_symbol(object, symbol).map(Value::Any)
                    });
                    match own {
                        Some(found) => found,
                        // Built-in iterables expose `[Symbol.iterator]` as a
                        // callable that produces their iterator.
                        None if symbol == zun_core::WellKnownSymbol::Iterator.id()
                            && zun_gc::with_current_heap(|heap| {
                                heap.array_len(object).is_some()
                                    || heap.set_values(object).is_some()
                                    || heap.map_entries(object).is_some()
                                    || heap.array_iterator_done(object).is_some()
                                    || heap.set_iterator_done(object).is_some()
                                    || heap.map_iterator_done(object).is_some()
                            }) =>
                        {
                            Value::Function(zun_core::FunctionHandle::new_generator_method(
                                3,
                                object.addr() as usize,
                            ))
                        }
                        None => {
                            // A symbol-keyed *method* lives in the function
                            // slot (own) or the per-class symbol table.
                            let handle = zun_gc::with_current_heap(|heap| {
                                heap.dynamic_get_symbol_function(object, symbol)
                            })
                            .or_else(|| {
                                self.class_symbol_methods
                                    .get(&(object.type_rid(), symbol.get()))
                                    .copied()
                            });
                            match handle {
                                Some(handle) => Value::Function(handle),
                                None => {
                                    // A symbol-keyed member installed on
                                    // `Class.prototype` at run time is found
                                    // by walking the class prototype chain.
                                    let mut found = Value::Void;
                                    if self.instance_class_prototype(object).is_some() {
                                        let mut rid = object.type_rid();
                                        while rid >= 1 {
                                            let proto = self.class_prototype_object(rid);
                                            let hit = zun_gc::with_current_heap(|heap| {
                                                heap.dynamic_get_symbol_function(proto, symbol)
                                                    .map(Value::Function)
                                                    .or_else(|| {
                                                        heap.dynamic_get_symbol(proto, symbol).map(
                                                            |any| match any.as_object().and_then(
                                                                |addr| {
                                                                    heap.function_of_cell(unsafe {
                                                                        zun_gc::Obj::from_addr(
                                                                            addr as *mut u64,
                                                                        )
                                                                    })
                                                                },
                                                            ) {
                                                                Some(function) => {
                                                                    Value::Function(function)
                                                                }
                                                                None => Value::Any(any),
                                                            },
                                                        )
                                                    })
                                            });
                                            if let Some(hit) = hit {
                                                found = hit;
                                                break;
                                            }
                                            rid = match self
                                                .module
                                                .tables
                                                .type_defs
                                                .get(rid as usize - 1)
                                            {
                                                Some(def)
                                                    if def.base.table()
                                                        == Some(
                                                            zun_metadata::TableKind::TypeDef,
                                                        )
                                                        && !def.base.is_null() =>
                                                {
                                                    def.base.rid()
                                                }
                                                _ => 0,
                                            };
                                        }
                                    }
                                    found
                                }
                            }
                        }
                    }
                }
            } else {
                return Err(RunError::BadToken);
            }
        } else if let Some(element) = match &key {
            // A canonical index string on an array addresses the element.
            Value::Str(text) => zun_gc::with_current_heap(|heap| {
                let len = heap.array_len(object)?;
                if &**text == "length" {
                    return Some(Value::Float(len as f64, zun_il::NumKind::F64));
                }
                let index = text
                    .parse::<usize>()
                    .ok()
                    .filter(|i| i.to_string() == **text)?;
                Some(if index < len {
                    heap.array_get(object, index)
                        .and_then(|any| Value::unbox_any(any, heap))
                        .unwrap_or(Value::Void)
                } else {
                    Value::Void
                })
            }),
            _ => None,
        } {
            element
        } else if let Value::Str(key) = &key {
            match self.dynamic_member_get(object, key) {
                // An inherited `Object.prototype` method read off a plain
                // dynamic object is a callable function value.
                Ok(Value::Void)
                    if matches!(
                        &**key,
                        "toString"
                            | "toLocaleString"
                            | "valueOf"
                            | "hasOwnProperty"
                            | "isPrototypeOf"
                            | "propertyIsEnumerable"
                    ) && zun_gc::with_current_heap(|heap| {
                        heap.is_dynamic_object(object)
                            && !heap.is_instance_bag(object)
                            && heap.array_len(object).is_none()
                    }) =>
                {
                    let mode = 100.0
                        + [
                            "toString",
                            "toLocaleString",
                            "valueOf",
                            "hasOwnProperty",
                            "isPrototypeOf",
                            "propertyIsEnumerable",
                        ]
                        .iter()
                        .position(|n| *n == &**key)
                        .unwrap_or(0) as f64;
                    Value::Function(self.make_promise_settler(object, mode))
                }
                Ok(v) => v,
                Err(RunError::Thrown(exception)) => {
                    if self.route_exception(body, stack, pc, next - size, &exception) {
                        return Ok(ArmFlow::Continue);
                    }
                    return Err(RunError::Thrown(exception));
                }
                Err(error) => return Err(error),
            }
        } else if matches!(key, Value::Int(..) | Value::Float(..)) {
            // A numeric computed key: an array index on an array,
            // otherwise the stringified property on a dynamic object
            // (`obj[2]` -> property "2"). Out of range reads undefined.
            zun_gc::with_current_heap(|heap| {
                if let Some(len) = heap.array_len(object) {
                    let index = match &key {
                        Value::Int(n, _) => *n,
                        other => other.as_number() as i64,
                    };
                    if index >= 0 && (index as usize) < len {
                        heap.array_get(object, index as usize)
                            .and_then(|any| Value::unbox_any(any, heap))
                            .unwrap_or(Value::Void)
                    } else {
                        Value::Void
                    }
                } else {
                    heap.dynamic_get(object, &number_index_key(&key))
                        .map(Value::Any)
                        .unwrap_or(Value::Void)
                }
            })
        } else {
            return Err(RunError::BadToken);
        };
        stack.push(value);
        Ok(ArmFlow::Next)
    }

    #[inline(always)]
    pub(super) fn op_store_dynamic_value(
        &mut self,
        mut stack: &mut Vec<Value>,
        mut pc: &mut usize,
        body: &zun_module::MethodBody<'_>,
        next: usize,
        size: usize,
    ) -> Result<ArmFlow, RunError> {
        let value = pop(stack)?;
        // Box any non-function value for the slot (a primitive or
        // `null` assigned through a computed key, e.g. `o[k] = v`
        // where `v: any = 5.0`, reaches here unboxed).
        let fn_value = match &value {
            Value::Function(handle) => Some(*handle),
            _ => None,
        };
        let any = if fn_value.is_some() {
            zun_core::Any::void()
        } else {
            zun_gc::with_current_heap_mut(|heap| value.box_any(heap)).ok_or(RunError::BadToken)?
        };
        let key = pop(stack)?;
        let key = match key {
            Value::Any(any) if any.as_symbol().is_none() => {
                zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)).unwrap_or(Value::Void)
            }
            other => other,
        };
        // ToPropertyKey: booleans, null/undefined and objects become
        // their string form (objects run toString first).
        let key = if matches!(
            key,
            Value::Bool(_) | Value::Null | Value::Void | Value::Object(_)
        ) {
            match self.dyn_to_string(&key) {
                Ok(text) => Value::Str(text.into()),
                Err(RunError::Thrown(exception)) => {
                    if self.route_exception(body, stack, pc, next - size, &exception) {
                        return Ok(ArmFlow::Continue);
                    }
                    return Err(RunError::Thrown(exception));
                }
                Err(error) => return Err(error),
            }
        } else {
            key
        };
        let object = expando_receiver(pop(stack)?).as_object();
        {
            let key_name: Option<String> = match &key {
                Value::Str(text) => Some(text.to_string()),
                Value::Int(..) | Value::Float(..) => None,
                _ => None,
            };
            let refusal = zun_gc::with_current_heap(|heap| {
                if let Some(len) = heap.array_len(object) {
                    let level = heap.array_integrity(object);
                    if matches!(key, Value::Int(..) | Value::Float(..)) {
                        let index = match &key {
                            Value::Int(n, _) => *n,
                            other => other.as_number() as i64,
                        };
                        if index < 0 {
                            return None;
                        }
                        let op = if (index as usize) < len {
                            integrity::ArrayOp::Write
                        } else {
                            integrity::ArrayOp::Add
                        };
                        return integrity::array_refusal(level, op).map(str::to_owned);
                    }
                    return None;
                }
                let name = key_name.as_deref()?;
                if heap.dynamic_is_accessor(object, name) {
                    if heap.dynamic_accessor_setter(object, name).is_none() {
                        Some(format!("TypeError: Cannot set property {name} of #<Object> which has only a getter"))
                    } else {
                        None
                    }
                } else if !heap.dynamic_can_set(object, name) {
                    if heap.dynamic_has(object, name) {
                        Some(format!("TypeError: Cannot assign to read only property '{name}' of object '#<Object>'"))
                    } else {
                        Some(format!(
                            "TypeError: Cannot add property {name}, object is not extensible"
                        ))
                    }
                } else {
                    None
                }
            });
            if let Some(message) = refusal {
                throw_native_arm!(self, body, stack, pc, next - size, message);
            }
        }
        match &key {
            Value::Any(key_any) if key_any.as_symbol().is_some() => {
                let symbol = key_any.as_symbol().expect("symbol key");
                zun_gc::with_current_heap_mut(|heap| {
                    if heap.array_len(object).is_some() || heap.regex_source(object).is_some() {
                        heap.ensure_dynamic_entry(object);
                    }
                    if let Some(handle) = fn_value {
                        // A function value rides the symbol-function slot.
                        heap.dynamic_set_symbol_function(object, symbol, handle);
                    } else {
                        heap.dynamic_set_symbol(object, symbol, any);
                    }
                });
            }
            Value::Str(key)
                if fn_value.is_some()
                    && zun_gc::with_current_heap(|heap| heap.array_len(object).is_none()) =>
            {
                // A function value rides the object's function slot.
                let handle = fn_value.expect("function value");
                zun_gc::with_current_heap_mut(|heap| {
                    if heap.dynamic_get_function(object, key).is_none() {
                        heap.dynamic_remove(object, key);
                    }
                    heap.dynamic_set_function(object, key.as_ref().to_owned(), handle);
                });
            }
            Value::Str(key)
                if zun_gc::with_current_heap(|heap| {
                    !heap.is_dynamic_object(object)
                        && heap.array_len(object).is_none()
                        && object.type_rid() >= 1
                }) =>
            {
                // A typed class instance: a declared field takes the
                // value in place; any other name becomes an expando.
                if !self.instance_member_set(object, key, &value) {
                    zun_gc::with_current_heap_mut(|heap| {
                        heap.ensure_instance_bag(object);
                        heap.dynamic_set(object, key.as_ref().to_owned(), any);
                    });
                }
            }
            Value::Str(key) => {
                zun_gc::with_current_heap_mut(|heap| {
                    if let Some(len) = heap.array_len(object) {
                        // A canonical index string addresses the element;
                        // any other string is an expando on the array.
                        if let Ok(index) = key.parse::<usize>() {
                            if index.to_string() == **key && index < u32::MAX as usize {
                                if index >= len {
                                    heap.array_resize(object, index + 1);
                                }
                                heap.array_set(object, index, any);
                                return;
                            }
                        }
                        heap.ensure_dynamic_entry(object);
                    }
                    heap.dynamic_assign(object, key.as_ref().to_owned(), any);
                });
            }
            Value::Int(..) | Value::Float(..) => {
                // Array index on an array, else a stringified property
                // on a dynamic object (`obj[2] = v` -> property "2").
                // An out-of-range array write is a no-op (fixed-length
                // managed arrays do not auto-extend).
                zun_gc::with_current_heap_mut(|heap| {
                    if heap.array_len(object).is_some() {
                        let index = match &key {
                            Value::Int(n, _) => *n,
                            other => other.as_number() as i64,
                        };
                        if index >= 0 {
                            let len = heap.array_len(object).unwrap_or(0);
                            if index as usize >= len {
                                heap.array_resize(object, index as usize + 1);
                            }
                            heap.array_set(object, index as usize, any);
                        }
                    } else {
                        heap.dynamic_set(object, number_index_key(&key), any);
                    }
                });
            }
            _ => return Err(RunError::BadToken),
        }
        stack.push(value);
        Ok(ArmFlow::Next)
    }
}
