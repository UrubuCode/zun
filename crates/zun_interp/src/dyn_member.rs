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
                // A script-defined own `name` / `length` shadows the metadata.
                let bag = crate::function_expando_bag(handle);
                if let Some(own) =
                    zun_gc::with_current_heap(|heap| heap.dynamic_get(bag, &name))
                {
                    if let Some(value) =
                        zun_gc::with_current_heap(|heap| Value::unbox_any(own, heap))
                    {
                        stack.push(value);
                        return Ok(ArmFlow::Continue);
                    }
                }
                if let Some(meta) = self.function_meta(handle, &name) {
                    stack.push(meta);
                    return Ok(ArmFlow::Continue);
                }
                // An anonymous closure has no name metadata: the same `""` / `0`
                // the compile-time fold used to answer for a dynamic function.
                if handle.builtin_method().is_none() && handle.generator_method().is_none() {
                    stack.push(if name == "name" {
                        Value::Str("".into())
                    } else {
                        Value::Float(0.0, zun_il::NumKind::F64)
                    });
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        if name == "prototype" {
            let callee = match &popped {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)),
                other => Some(other.clone()),
            };
            if let Some(Value::Function(handle)) = callee {
                self.ensure_function_prototype(handle);
            }
        }
        if let Some(Value::Function(handle)) = match &popped {
            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)),
            other => Some(other.clone()),
        } {
            // A constant (`Number.EPSILON`) or a static method read as a value
            // (`Number.isInteger`). `Number`/`String`/`Boolean` arrive here as
            // the coercion function sema lowers them to, not as a class
            // descriptor, so the static tables are reached through that name.
            if let Some(member) = self.coerce_ctor_static_value(handle, &name) {
                stack.push(member);
                return Ok(ArmFlow::Continue);
            }
        }
        // A static a constructor function inherits from a class it was
        // chained to (`Object.setPrototypeOf(F, Class)`, as a run-time class
        // over a static base is): the read continues on the class value. A
        // function parent needs nothing here -- its property bag is already
        // the link the ordinary lookup follows.
        let function_handle = match &popped {
            Value::Function(handle) => Some(*handle),
            Value::Any(any) => {
                match zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)) {
                    Some(Value::Function(handle)) => Some(handle),
                    _ => None,
                }
            }
            _ => None,
        };
        let popped = match function_handle
            .and_then(|handle| self.function_inherited_static(handle, &name))
        {
            Some(crate::collections::InheritedStatic::Class(class)) => Value::Any(class),
            _ => popped,
        };
        let function_receiver = Self::is_function_value(&popped);
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
        // `gen.map` / `setIterator.toArray` ... read off a built-in iterator
        // are the shared `%Iterator.prototype%` helper method values.
        if let Some(index) = crate::collections::iterator_helpers::iter_helper_index(&name) {
            if let Value::Object(obj) = &concrete {
                if crate::collections::iterator_helpers::is_builtin_iterator_object(*obj) {
                    stack.push(Value::Function(zun_core::FunctionHandle::new_builtin_method(
                        crate::collections::iterator_helpers::BUILTIN_ITER_HELPER_BASE + index,
                    )));
                    return Ok(ArmFlow::Continue);
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
                    // The accessor lives on `Symbol.prototype`; once a program
                    // deletes it there, a symbol no longer answers.
                    let present = self.builtin_prototype_by_name("Symbol").map_or(true, |proto| {
                        zun_gc::with_current_heap(|heap| {
                            heap.dynamic_accessor_getter(proto, "description").is_some()
                        })
                    });
                    let description = zun_gc::with_current_heap(|heap| {
                        heap.symbol_description(id).filter(|_| present).map(|text| text.to_string())
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
        // A native-backed subclass instance (or a native with its own
        // `constructor`) answers from the value and its prototype chain.
        if name == "constructor" {
            if let Value::Object(obj) = &concrete {
                if let Some(found) = Self::native_chain_constructor(*obj) {
                    stack.push(Value::Any(found));
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        // `view.constructor` of a typed array / ArrayBuffer / DataView.
        if name == "constructor" {
            if let Value::Object(obj) = &concrete {
                let class_name = zun_gc::with_current_heap(|heap| {
                    if let Some(kind) = heap.typed_array_element(*obj) {
                        Some(kind.js_name())
                    } else if heap.buffer_len(*obj).is_some() {
                        Some("ArrayBuffer")
                    } else if heap.data_view_buffer(*obj).is_some() {
                        Some("DataView")
                    } else {
                        None
                    }
                });
                if let Some(class_name) = class_name {
                    let record = Self::builtin_constructor_record(class_name);
                    stack.push(Value::Object(record));
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        // An ordinary object reads `constructor` like any other property:
        // its own, else through the prototype chain it was linked to (an
        // instance of a constructor function, or of a run-time class).
        if name == "constructor" {
            if let Value::Object(obj) = &concrete {
                match self.explicit_constructor(*obj) {
                    Ok(Some(found)) => {
                        stack.push(found);
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
        // Typed array / ArrayBuffer / DataView properties.
        if let Value::Object(view) = &concrete {
            if let Some(found) = self.typed_view_get(*view, &name) {
                let boxed = zun_gc::with_current_heap_mut(|heap| found.box_any(heap));
                stack.push(match boxed {
                    Some(any) => Value::Any(any),
                    None => found,
                });
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
        // A named read off a primitive resolves on its wrapper's prototype
        // (`(1).toFixed`, `"s".charAt`), the same lookup `Number.prototype.x`
        // performs; a primitive owns no properties itself.
        let primitive_proto = {
            let class = match &concrete {
                Value::Str(_) => Some("String"),
                Value::Bool(_) => Some("Boolean"),
                Value::Float(..) | Value::Int(..) => Some("Number"),
                Value::Object(o)
                    if zun_gc::with_current_heap(|heap| heap.bigint_value(*o).is_some()) =>
                {
                    Some("BigInt")
                }
                Value::Any(any) if any.as_symbol().is_some() => Some("Symbol"),
                _ => None,
            };
            class.and_then(|class| self.builtin_prototype_by_name(class))
        };
        // Any other named read off a primitive string / number / boolean has no
        // own properties here: it yields `undefined` rather than faulting.
        if primitive_proto.is_none()
            && matches!(
                concrete,
                Value::Str(_) | Value::Int(..) | Value::Float(..) | Value::Bool(_)
            )
        {
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
                    "flags" => Some(Value::Str(zun_core::ordered_flags(&flags).into())),
                    "global" => flag_of('g'),
                    "ignoreCase" => flag_of('i'),
                    "multiline" => flag_of('m'),
                    "dotAll" => flag_of('s'),
                    "unicode" => flag_of('u'),
                    "unicodeSets" => flag_of('v'),
                    "sticky" => flag_of('y'),
                    "hasIndices" => flag_of('d'),
                    "lastIndex" => zun_gc::with_current_heap(|heap| {
                        heap.regex_last_index_any(*regex)
                            .and_then(|any| Value::unbox_any(any, heap))
                    }),
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
                // A property a script stored on the class value itself shadows
                // a declared static method of the same name (a declared static
                // field is written in place and never reaches the bag).
                if user_class && name != "prototype" {
                    if let Some(value) =
                        Self::class_expando_read_own(token, &Value::Str(name.as_str().into()))
                    {
                        stack.push(value);
                        return Ok(ArmFlow::Continue);
                    }
                }
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
                if name != "prototype" {
                    if let Some(value) =
                        self.class_expando_read(token, &Value::Str(name.as_str().into()))
                    {
                        stack.push(value);
                        return Ok(ArmFlow::Continue);
                    }
                }
                if name == "BYTES_PER_ELEMENT" && !user_class {
                    if let Some(kind) = self.typed_ctor_kind(token) {
                        stack.push(Value::Float(kind.size() as f64, zun_il::NumKind::F64));
                        return Ok(ArmFlow::Continue);
                    }
                }
                if !user_class && (name == "length" || name == "name") {
                    if let Some(value) = self.builtin_ctor_own_value(token, &name) {
                        stack.push(value);
                        return Ok(ArmFlow::Continue);
                    }
                }
                if !user_class {
                    if let Some(value) = self.builtin_constant(token, &name) {
                        stack.push(value);
                        return Ok(ArmFlow::Continue);
                    }
                    if let Some(kind) = self.builtin_static_kind(token, &name) {
                        stack.push(Value::Function(zun_core::FunctionHandle::new_builtin_method(kind)));
                        return Ok(ArmFlow::Continue);
                    }
                }
                stack.push(if name == "prototype" && user_class {
                    let proto = self.class_prototype_object(token & !zun_natives::USER_CLASS_FLAG);
                    Value::Any(zun_core::Any::object(proto.addr() as usize))
                } else if name == "prototype" && !self.builtin_ctor_has_prototype(token) {
                    Value::Void
                } else if name == "prototype" {
                    let proto = self.builtin_prototype_object(token);
                    Value::Any(zun_core::Any::object(proto.addr() as usize))
                } else if self.callable_ctor_handle(&Value::Any(*descriptor)).is_some() {
                    // A constructor that is callable without `new` (`Symbol`)
                    // reads `call` / `apply` / `bind` exactly where calling
                    // them works.
                    self.function_prototype_member(&name).unwrap_or(Value::Void)
                } else {
                    Value::Void
                });
                return Ok(ArmFlow::Continue);
            }
        }
        // A primitive number/boolean/undefined/null receiver has no own
        // properties here: the read yields `undefined`.
        if primitive_proto.is_none()
            && matches!(
                concrete,
                Value::Bool(_) | Value::Int(..) | Value::Float(..) | Value::Void | Value::Null
            )
        {
            stack.push(Value::Void);
            return Ok(ArmFlow::Continue);
        }
        let object = primitive_proto.unwrap_or_else(|| concrete.as_object());
        // An accessor property invokes its getter (NUCLEUS §6);
        // a data property (or an absent key) reads the value slot.
        let getter = zun_gc::with_current_heap(|heap| heap.dynamic_accessor_getter(object, &name));
        // A typed class instance read through `any` resolves a
        // registered getter (`get_<name>`, e.g. `Error.name`).
        let typed_getter = if getter.is_none() && self.type_rid_describes_object(object) {
            self.class_named_methods
                .get(&object.type_rid())
                .and_then(|methods| methods.get(&format!("get_{name}")))
                .copied()
                // Not when the class's prototype chain was relinked away from
                // the ancestor that declares the getter.
                .filter(|getter| {
                    !self.inherited_member_detached(object.type_rid(), getter.method_rid())
                })
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
        // A function inherits `call` / `apply` / `bind` / `toString` (and
        // anything a program installs) from `Function.prototype`.
        let value = if matches!(value, Value::Void) && function_receiver {
            self.function_prototype_member(&name).unwrap_or(Value::Void)
        } else {
            value
        };
        // A `Map` / `Set` inherits its methods from the builtin prototype:
        // `map.entries` read as a value is `Map.prototype.entries`. The
        // collection is native storage with no prototype link to walk, so the
        // read answered `undefined`.
        let value = if matches!(value, Value::Void) {
            let builtin = zun_gc::with_current_heap(|heap| {
                if heap.map_entries(object).is_some() {
                    Some("Map")
                } else if heap.set_values(object).is_some() {
                    Some("Set")
                } else {
                    None
                }
            });
            builtin
                .and_then(|class| self.builtin_prototype_by_name(class))
                .and_then(|proto| {
                    zun_gc::with_current_heap(|heap| heap.dynamic_get_function(proto, &name))
                })
                .map_or(Value::Void, Value::Function)
        } else {
            value
        };
        // An inherited `Object.prototype` method read off a plain dynamic
        // object is a callable function value.
        let proto_mode = crate::object_proto_mode(name.as_str());
        let value = if matches!(value, Value::Void)
            && proto_mode.is_some()
            && zun_gc::with_current_heap(|heap| {
                heap.is_dynamic_object(object)
                    && !heap.is_instance_bag(object)
                    && heap.array_len(object).is_none()
                    && !matches!(heap.dynamic_prototype(object), Some(p) if p.tag() == zun_core::AnyTag::Null)
            }) {
            self.object_proto_method_value(object, name.as_str())
                .unwrap_or(Value::Void)
        } else {
            value
        };
        stack.push(value);
        Ok(ArmFlow::Next)
    }

    /// A store whose receiver is `undefined` / `null` raises a TypeError.
    /// `None` means the receiver is fine and the store proceeds. Kept out of
    /// line so the inlined store arm does not grow the interpreter frame.
    #[inline(never)]
    fn reject_nullish_store(
        &mut self,
        stack: &mut Vec<Value>,
        pc: &mut usize,
        body: &zun_module::MethodBody<'_>,
        name: &str,
        next: usize,
        size: usize,
    ) -> Option<Result<ArmFlow, RunError>> {
        let concrete = match stack.last().cloned()? {
            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)),
            other => Some(other),
        };
        let kind = match concrete {
            Some(Value::Null) => "null",
            Some(Value::Void) => "undefined",
            _ => return None,
        };
        let message = format!("Cannot set properties of {kind} (setting '{name}')");
        Some(match self.new_error_object("TypeError", &message) {
            Some(error) => {
                let exception = Value::Object(error);
                if self.route_exception(body, stack, pc, next - size, &exception) {
                    Ok(ArmFlow::Continue)
                } else {
                    Err(RunError::Thrown(exception))
                }
            }
            None => Err(RunError::Native(format!("TypeError: {message}"))),
        })
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
        // Writing a property of `undefined` / `null` is a TypeError, never an
        // engine panic (e.g. `this.x = 1` in a declared function called with
        // an undefined receiver).
        if let Some(flow) = self.reject_nullish_store(stack, pc, body, &name, next, size) {
            return flow;
        }
        // A class value: the write lands on its static field.
        if let Some(Value::Any(descriptor)) = stack.last() {
            let descriptor = *descriptor;
            if let Some(rid) = descriptor
                .as_class_descriptor()
                .and_then(|t| self.user_class_rid(t))
            {
                // A name the class declares no static for becomes an own
                // property of the class value, kept in its expando bag below.
                match self.class_static_store(rid, &name, &value) {
                    Ok(true) => {
                        stack.pop();
                        stack.push(value);
                        return Ok(ArmFlow::Continue);
                    }
                    Ok(false) => {}
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
            let Some(object) = store_receiver(pop(stack)?) else {
                // A primitive receiver keeps no own property.
                stack.push(Value::Function(handle));
                return Ok(ArmFlow::Next);
            };
            // A receiver with no property table yet gets one on its first
            // named store: an array, a typed class instance (its declared
            // fields stay in the fixed layout), a Map, Set, RegExp ...
            zun_gc::with_current_heap_mut(|heap| {
                if heap.is_dynamic_object(object) {
                    return;
                }
                if heap.array_len(object).is_none() && object.type_rid() >= 1 {
                    heap.ensure_instance_bag(object);
                } else {
                    heap.ensure_dynamic_entry(object);
                }
            });
            // A frozen, sealed or non-extensible object refuses a function
            // value exactly as it refuses a data value.
            let refusal =
                zun_gc::with_current_heap(|heap| integrity::named_store_refusal(heap, object, &name));
            if let Some(message) = refusal {
                throw_native_arm!(self, body, stack, pc, next - size, message);
            }
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
            let receiver = pop(stack)?;
            let function_receiver = matches!(receiver, Value::Function(_));
            let Some(object) = store_receiver(receiver) else {
                // A primitive receiver keeps no own property.
                stack.push(value);
                return Ok(ArmFlow::Next);
            };
            // A function's `name` and `length` are read-only, and a typed
            // array's `length` is an accessor without a setter: the write is
            // ignored rather than kept as an expando that would then list.
            let read_only = (function_receiver && matches!(name.as_str(), "name" | "length"))
                || (name == "length"
                    && zun_gc::with_current_heap(|heap| {
                        heap.typed_array_element(object).is_some()
                    }))
                || self.string_wrapper_owns(object, &name);
            if read_only {
                stack.push(value);
                return Ok(ArmFlow::Next);
            }
            // `re.lastIndex = n` on a RegExp reached through `any` writes the
            // real match cursor (the one `test` / `exec` advance and the dynamic
            // read answers from), never an expando of the same name.
            if name == "lastIndex"
                && zun_gc::with_current_heap(|heap| heap.regex_source(object).is_some())
            {
                if let Err(error) = zun_gc::with_current_heap_mut(|heap| {
                    heap.regex_assign_last_index(object, any)
                }) {
                    throw_native_arm!(self, body, stack, pc, next - size, error.to_string());
                }
                stack.push(value);
                return Ok(ArmFlow::Continue);
            }
            // `arr.length = n` on an array reached through `any`.
            if name == "length" {
                let level_len = zun_gc::with_current_heap(|heap| {
                    heap.array_len(object)
                        .map(|len| (len, heap.array_integrity(object)))
                });
                if let Some((len, level)) = level_len {
                    let wanted = any.as_int().map(|n| n as f64).or_else(|| any.as_float());
                    if let Some(bad) = wanted.filter(|w| {
                        !(w.is_finite() && *w >= 0.0 && w.fract() == 0.0 && *w <= 4294967295.0)
                    }) {
                        let _ = bad;
                        throw_native_arm!(
                            self,
                            body,
                            stack,
                            pc,
                            next - size,
                            "RangeError: Invalid array length".to_string()
                        );
                    }
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
            let refusal = self.member_store_refusal(object, &name);
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
                    (!heap.is_dynamic_object(object) || heap.is_instance_bag(object))
                        && heap.array_len(object).is_none()
                        && object.type_rid() >= 1
                });
                let handled = typed_instance && self.instance_member_set(object, &name, &value);
                // A declared `set <name>` accessor takes the write, as the read
                // path resolves `get <name>`; it must never be shadowed by an expando.
                let handled = if typed_instance && !handled {
                    match self.instance_member_setter_call(object, &name, &value) {
                        Ok(ran) => ran,
                        Err(RunError::Thrown(exception)) => {
                            if self.route_exception(body, stack, pc, next - size, &exception) {
                                return Ok(ArmFlow::Continue);
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    handled
                };
                // An accessor defined on the class prototype chain
                // (`Object.defineProperty(C.prototype, ...)`) takes the write.
                let proto_accessor = if typed_instance && !handled {
                    self.proto_chain_accessor_setter(object, &name)
                } else if !handled {
                    // A native-backed subclass instance inherits the setters
                    // its class declares, and an ordinary object the setters
                    // on its prototype chain (an instance of a run-time class).
                    Self::native_chain_accessor_setter(object, &name)
                        .or_else(|| Self::dynamic_chain_accessor_setter(object, &name))
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
    /// The stable index of a well-known symbol by name, mirroring the fold in
    /// `zun_sema`'s `check/expr/field.rs`. The two must stay in step: the fold
    /// answers a literal key at compile time and this answers a computed one.
    pub(crate) fn well_known_symbol_index(name: &str) -> Option<u64> {
        Some(match name {
            "iterator" => 0,
            "asyncIterator" => 1,
            "hasInstance" => 2,
            "toPrimitive" => 3,
            "toStringTag" => 4,
            "match" => 5,
            "replace" => 6,
            "search" => 7,
            "split" => 8,
            "species" => 9,
            "isConcatSpreadable" => 10,
            "unscopables" => 11,
            "matchAll" => 12,
            _ => return None,
        })
    }

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
        let receiver = pop(stack)?;
        // `Number["isInteger"]` / `Number["EPSILON"]` on the coercion function value.
        // A well-known symbol read with a **computed** key: `Symbol[n]` where
        // `n` is a variable answered `undefined`, while `Symbol.iterator` and
        // `Symbol["iterator"]` both work because sema folds a literal key.
        // Nothing answered at run time, and this is the computed-key arm
        // (`lddyn.value`) such a read takes.
        if let Value::Str(name) = &key {
            let descriptor = match &receiver {
                Value::Any(any) => any.as_class_descriptor(),
                _ => None,
            };
            if descriptor
                .is_some_and(|token| Self::builtin_ctor_global_name(token) == Some("Symbol"))
            {
                if let Some(index) = Self::well_known_symbol_index(name) {
                    let id = zun_core::SymbolId::from_raw(index);
                    stack.push(Value::Any(zun_core::Any::symbol(id)));
                    return Ok(ArmFlow::Continue);
                }
            }
        }
        if let Value::Str(name) = &key {
            let callee = match &receiver {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)),
                other => Some(other.clone()),
            };
            if let Some(Value::Function(handle)) = callee {
                if let Some(member) = self.coerce_ctor_static_value(handle, name) {
                    stack.push(member);
                    return Ok(ArmFlow::Continue);
                }
                // `fn["prototype"]` reads the same lazily created prototype
                // object `fn.prototype` does.
                if &**name == "prototype" {
                    self.ensure_function_prototype(handle);
                }
            }
        }
        // `Class["prototype"]` on a class descriptor answers the class's
        // prototype object, the same value `Class.prototype` reads.
        if let Value::Str(name) = &key {
            if &**name == "prototype" {
                let descriptor = match &receiver {
                    Value::Any(any) => any.as_class_descriptor(),
                    _ => None,
                };
                if let Some(token) = descriptor {
                    let user_class = token & zun_natives::USER_CLASS_FLAG != 0
                        && ((token & !zun_natives::USER_CLASS_FLAG) as usize) >= 1
                        && ((token & !zun_natives::USER_CLASS_FLAG) as usize)
                            <= self.module.tables.type_defs.len();
                    if user_class {
                        let proto =
                            self.class_prototype_object(token & !zun_natives::USER_CLASS_FLAG);
                        stack.push(Value::Any(zun_core::Any::object(proto.addr() as usize)));
                        return Ok(ArmFlow::Continue);
                    }
                    if self.builtin_ctor_has_prototype(token) {
                        let proto = self.builtin_prototype_object(token);
                        stack.push(Value::Any(zun_core::Any::object(proto.addr() as usize)));
                        return Ok(ArmFlow::Continue);
                    }
                }
            }
        }
        let function_receiver = Self::is_function_value(&receiver);
        let object_raw = expando_receiver(receiver);
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
        // A typed array receiver: integer-indexed element read.
        {
            let concrete = match &object_raw {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                    .unwrap_or(Value::Void),
                other => other.clone(),
            };
            if let Value::Object(view) = &concrete {
                if let Some(answer) = self.typed_view_index_get(*view, &key) {
                    stack.push(match zun_gc::with_current_heap_mut(|heap| answer.box_any(heap)) {
                        Some(any) => Value::Any(any),
                        None => answer,
                    });
                    return Ok(ArmFlow::Continue);
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
        if let Value::Any(descriptor) = &object_raw {
            if let Some(token) = descriptor.as_class_descriptor() {
                // A property a script stored on the class value itself shadows
                // a declared static method of the same name.
                if self.user_class_rid(token).is_some() && matches!(key, Value::Str(_)) {
                    if let Some(value) = Self::class_expando_read_own(token, &key) {
                        stack.push(value);
                        return Ok(ArmFlow::Continue);
                    }
                }
                // A static field or method of a user class, read by a computed key.
                if let (Some(rid), Value::Str(name)) = (self.user_class_rid(token), &key) {
                    match self.class_static_read(rid, name) {
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
                if let Some(value) = self.class_expando_read(token, &key) {
                    stack.push(value);
                    return Ok(ArmFlow::Continue);
                }
                let kind = match &key {
                    Value::Str(name) => self.builtin_static_kind(token, name),
                    _ => None,
                };
                stack.push(match kind {
                    Some(kind) => {
                        Value::Function(zun_core::FunctionHandle::new_builtin_method(kind))
                    }
                    None => Value::Void,
                });
                return Ok(ArmFlow::Continue);
            }
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
            if let Some(index) = crate::collections::iterator_helpers::iter_helper_index(name) {
                if crate::collections::iterator_helpers::is_builtin_iterator_object(object) {
                    stack.push(Value::Function(zun_core::FunctionHandle::new_builtin_method(
                        crate::collections::iterator_helpers::BUILTIN_ITER_HELPER_BASE + index,
                    )));
                    return Ok(ArmFlow::Continue);
                }
            }
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
                        None if symbol == zun_core::WellKnownSymbol::ToStringTag.id()
                            && zun_gc::with_current_heap(|heap| heap.array_iterator_mode(object).is_some()) =>
                        {
                            let text = if zun_gc::with_current_heap(|heap| heap.array_iterator_mode(object)) == Some(3) {
                                "String Iterator"
                            } else {
                                "Array Iterator"
                            };
                            Value::Any(zun_gc::with_current_heap_mut(|heap| {
                                zun_core::Any::string(heap.intern_string(text))
                            }))
                        }
                        None if symbol == zun_core::WellKnownSymbol::ToStringTag.id()
                            && zun_gc::with_current_heap(|heap| heap.typed_array_element(object).is_some()) =>
                        {
                            let text = zun_gc::with_current_heap(|heap| {
                                heap.typed_array_element(object).map(|k| k.js_name())
                            })
                            .unwrap_or("");
                            Value::Any(zun_gc::with_current_heap_mut(|heap| {
                                zun_core::Any::string(heap.intern_string(text))
                            }))
                        }
                        // Built-in iterables expose `[Symbol.iterator]` as a
                        // callable that produces their iterator.
                        // `arr[Symbol.iterator]` **is** `Array.prototype.values`,
                        // the same function object. The receiver-bound generator
                        // handle built below cannot be, so an array answers the
                        // prototype's own handle instead.
                        None if symbol == zun_core::WellKnownSymbol::Iterator.id()
                            && zun_gc::with_current_heap(|heap| heap.array_len(object).is_some())
                            && crate::intrinsic_meta::array_values_kind().is_some() =>
                        {
                            Value::Function(zun_core::FunctionHandle::new_builtin_method(
                                crate::intrinsic_meta::array_values_kind()
                                    .expect("array values kind"),
                            ))
                        }
                        // Likewise `map[Symbol.iterator]` **is**
                        // `Map.prototype.entries` and `set[Symbol.iterator]` is
                        // `Set.prototype.values`: the prototype's own handle.
                        None if symbol == zun_core::WellKnownSymbol::Iterator.id()
                            && zun_gc::with_current_heap(|heap| {
                                heap.map_entries(object).is_some()
                            }) =>
                        {
                            Value::Function(zun_core::FunctionHandle::new_builtin_method(
                                crate::property_ops::BUILTIN_MAP_ITERATOR,
                            ))
                        }
                        None if symbol == zun_core::WellKnownSymbol::Iterator.id()
                            && zun_gc::with_current_heap(|heap| {
                                heap.set_values(object).is_some()
                            }) =>
                        {
                            Value::Function(zun_core::FunctionHandle::new_builtin_method(
                                crate::property_ops::BUILTIN_SET_ITERATOR,
                            ))
                        }
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
                            })
                            .or_else(|| {
                                crate::collections::iterator_helpers::symbol_function_in_chain(
                                    object, symbol,
                                )
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
                                            // An instance `get [Symbol.x]()` is an accessor
                                            // on the class prototype: run it with the
                                            // instance as receiver.
                                            if let Some(getter) = zun_gc::with_current_heap(|heap| {
                                                heap.dynamic_symbol_getter(proto, symbol)
                                            }) {
                                                let call_args =
                                                    self.with_this_arg(getter, object, Vec::new());
                                                match self.invoke_function_value(getter, call_args) {
                                                    Ok(v) => {
                                                        found = v;
                                                        break;
                                                    }
                                                    Err(RunError::Thrown(exception)) => {
                                                        if self.route_exception(
                                                            body,
                                                            stack,
                                                            pc,
                                                            next - size,
                                                            &exception,
                                                        ) {
                                                            return Ok(ArmFlow::Continue);
                                                        }
                                                        return Err(RunError::Thrown(exception));
                                                    }
                                                    Err(error) => return Err(error),
                                                }
                                            }
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
                                    // A symbol-keyed data property (or accessor) on a
                                    // prototype object of the dynamic chain
                                    // (`Object.create(p)[sym]`, a helper's
                                    // `@@toStringTag`).
                                    if matches!(found, Value::Void) {
                                        let mut current = zun_gc::with_current_heap(|heap| {
                                            heap.dynamic_prototype(object)
                                                .and_then(|any| any.as_object())
                                        });
                                        for _ in 0..256 {
                                            let Some(address) = current else { break };
                                            let holder = unsafe {
                                                zun_gc::Obj::from_addr(address as *mut u64)
                                            };
                                            if let Some(getter) = zun_gc::with_current_heap(|heap| {
                                                heap.dynamic_symbol_getter(holder, symbol)
                                            }) {
                                                let call_args =
                                                    self.with_this_arg(getter, object, Vec::new());
                                                match self.invoke_function_value(getter, call_args) {
                                                    Ok(v) => {
                                                        found = v;
                                                        break;
                                                    }
                                                    Err(RunError::Thrown(exception)) => {
                                                        if self.route_exception(
                                                            body,
                                                            stack,
                                                            pc,
                                                            next - size,
                                                            &exception,
                                                        ) {
                                                            return Ok(ArmFlow::Continue);
                                                        }
                                                        return Err(RunError::Thrown(exception));
                                                    }
                                                    Err(error) => return Err(error),
                                                }
                                            }
                                            if let Some(any) = zun_gc::with_current_heap(|heap| {
                                                heap.dynamic_get_symbol(holder, symbol)
                                            }) {
                                                found = Value::Any(any);
                                                break;
                                            }
                                            current = zun_gc::with_current_heap(|heap| {
                                                heap.dynamic_prototype(holder)
                                                    .and_then(|any| any.as_object())
                                            });
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
        } else if let Some(unit) = self.string_wrapper_unit(object, &key) {
            unit
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
                // `fn["call"]`: inherited from `Function.prototype`.
                Ok(Value::Void)
                    if function_receiver && self.function_prototype_member(key).is_some() =>
                {
                    self.function_prototype_member(key).unwrap_or(Value::Void)
                }
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
                    self.object_proto_method_value(object, key)
                        .unwrap_or(Value::Void)
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
        let receiver = pop(stack)?;
        // A user class value: a declared static takes the write in place; any
        // other key becomes an own property kept in the class's expando bag.
        if let (Value::Any(descriptor), Value::Str(name)) = (&receiver, &key) {
            if let Some(rid) = descriptor
                .as_class_descriptor()
                .and_then(|token| self.user_class_rid(token))
            {
                match self.class_static_store(rid, name, &value) {
                    Ok(true) => {
                        stack.push(value);
                        return Ok(ArmFlow::Next);
                    }
                    Ok(false) => {}
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
        // The indices and `length` of a `String` wrapper are read-only: the
        // write is ignored, as on the primitive.
        if let Some(wrapper) = store_receiver(receiver.clone()) {
            if self.string_wrapper_owns_key(wrapper, &key) {
                stack.push(value);
                return Ok(ArmFlow::Next);
            }
        }
        let Some(object) = store_receiver(receiver.clone()) else {
            let concrete = match receiver {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                    .unwrap_or(Value::Void),
                other => other,
            };
            let kind = match concrete {
                Value::Null => "null",
                Value::Void => "undefined",
                _ => {
                    // A primitive receiver keeps no own property.
                    stack.push(value);
                    return Ok(ArmFlow::Next);
                }
            };
            throw_native_arm!(
                self,
                body,
                stack,
                pc,
                next - size,
                format!("TypeError: Cannot set properties of {kind}")
            );
        };
        // A typed array receiver: integer-indexed element write.
        if crate::typed_arrays::element_of(object).is_some() {
            match self.typed_view_index_set(object, &key, &value) {
                Ok(true) => {
                    // The assignment expression evaluates to the stored value.
                    stack.push(value);
                    return Ok(ArmFlow::Next);
                }
                Ok(false) => {}
                Err(RunError::Thrown(exception)) => {
                    if self.route_exception(body, stack, pc, next - size, &exception) {
                        return Ok(ArmFlow::Continue);
                    }
                    return Err(RunError::Thrown(exception));
                }
                Err(error) => return Err(error),
            }
        }
        {
            let key_name: Option<String> = match &key {
                Value::Str(text) => Some(text.to_string()),
                Value::Int(..) | Value::Float(..) => None,
                _ => None,
            };
            let declared = key_name
                .as_deref()
                .and_then(|name| self.instance_declared_member(object, name));
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
                    // A named (non-index) property of an array obeys the
                    // array's integrity level like any other own property.
                    return match key_name.as_deref() {
                        Some(name) if name != "length" && name.parse::<usize>().is_err() => {
                            integrity::named_store_refusal(heap, object, name)
                        }
                        _ => None,
                    };
                }
                if let Value::Any(key_any) = &key {
                    // A symbol key: an own non-writable data property or
                    // function slot refuses the write.
                    let symbol = key_any.as_symbol()?;
                    if heap.is_dynamic_object(object)
                        && heap.dynamic_has_symbol_key(object, symbol)
                        && heap.dynamic_symbol_getter(object, symbol).is_none()
                        && !heap.dynamic_is_symbol_writable(object, symbol)
                    {
                        let label = heap.symbol_description(symbol).unwrap_or("");
                        return Some(format!(
                            "TypeError: Cannot assign to read only property 'Symbol({label})' of object '#<Object>'"
                        ));
                    }
                    return None;
                }
                let name = key_name.as_deref()?;
                match declared {
                    // A declared field of a typed class instance stays writable
                    // until the instance is frozen; a declared setter always runs.
                    Some(true) => heap.dynamic_is_frozen(object).then(|| {
                        format!(
                            "TypeError: Cannot assign to read only property '{name}' of object '#<Object>'"
                        )
                    }),
                    Some(false) => None,
                    None => integrity::named_store_refusal(heap, object, name),
                }
            });
            if let Some(message) = refusal {
                throw_native_arm!(self, body, stack, pc, next - size, message);
            }
        }
        // A symbol-keyed accessor with a setter runs it instead of storing.
        if let Value::Any(key_any) = &key {
            if let Some(setter) = key_any
                .as_symbol()
                .and_then(|symbol| self.symbol_setter_lookup(object, symbol))
            {
                let call_args = self.with_this_arg(setter, object, vec![value.clone()]);
                match self.invoke_function_value(setter, call_args) {
                    Ok(_) => {
                        stack.push(value);
                        return Ok(ArmFlow::Next);
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
        match &key {
            Value::Any(key_any) if key_any.as_symbol().is_some() => {
                let symbol = key_any.as_symbol().expect("symbol key");
                zun_gc::with_current_heap_mut(|heap| {
                    if heap.array_len(object).is_some() || heap.regex_source(object).is_some() {
                        heap.ensure_dynamic_entry(object);
                    }
                    // A typed class instance, Map, Set ... keeps a symbol-keyed
                    // property in the table it gains on its first store.
                    integrity::ensure_property_table(heap, object);
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
                    // A receiver with no property table yet gets one: a typed
                    // class instance keeps its declared fields in the fixed
                    // layout, a Map, Set or RegExp gains an expando table.
                    if !heap.is_dynamic_object(object) {
                        if object.type_rid() >= 1 {
                            heap.ensure_instance_bag(object);
                        } else {
                            heap.ensure_dynamic_entry(object);
                        }
                    }
                    if heap.dynamic_get_function(object, key).is_none() {
                        heap.dynamic_remove(object, key);
                    }
                    heap.dynamic_set_function(object, key.as_ref().to_owned(), handle);
                });
            }
            Value::Str(key)
                if zun_gc::with_current_heap(|heap| {
                    // An instance that already took an expando is dynamic too,
                    // but its declared fields still live in the fixed layout.
                    (!heap.is_dynamic_object(object) || heap.is_instance_bag(object))
                        && heap.array_len(object).is_none()
                        && object.type_rid() >= 1
                }) =>
            {
                // A typed class instance: a declared field takes the
                // value in place; any other name becomes an expando.
                let setter_ran = if self.instance_member_set(object, key, &value) {
                    true
                } else {
                    match self.instance_member_setter_call(object, key, &value) {
                        Ok(ran) => ran,
                        Err(RunError::Thrown(exception)) => {
                            if self.route_exception(body, stack, pc, next - size, &exception) {
                                return Ok(ArmFlow::Continue);
                            }
                            return Err(RunError::Thrown(exception));
                        }
                        Err(error) => return Err(error),
                    }
                };
                if !setter_ran {
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
                    } else if !heap.is_dynamic_object(object) {
                        // A Map, Set, RegExp ...: the write becomes an own
                        // expando property rather than vanishing.
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
                        // A numeric key on a receiver with no property table yet
                        // (a typed class instance, a Map ...) is an expando too.
                        // A typed array never grows an integer-keyed expando: an
                        // out-of-range element write is ignored.
                        if !heap.is_dynamic_object(object)
                            && heap.typed_array_element(object).is_none()
                        {
                            if object.type_rid() >= 1 {
                                heap.ensure_instance_bag(object);
                            } else {
                                heap.ensure_dynamic_entry(object);
                            }
                        }
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

impl Interp<'_> {
    /// Whether `method_rid`'s body yields, which is how a generator is
    /// recognized -- there is no metadata flag, so `create_generator` scans for
    /// `Yield` the same way and calls this rather than repeating the walk.
    /// Needed because a generator function is **not** a constructor: once a
    /// `Ty::Function` callee started reaching the runtime construct path,
    /// `new (g as any)()` built an object where node throws a TypeError.
    pub(crate) fn method_yields(&self, method_rid: u32) -> bool {
        let Some(method) = self
            .module
            .tables
            .method_defs
            .get((method_rid as usize).wrapping_sub(1))
        else {
            return false;
        };
        let Ok(body) = crate::read_body(&self.module.code, method.body_offset) else {
            return false;
        };
        let mut offset = 0;
        while offset < body.code.len() {
            let Ok((instruction, size)) = zun_il::decode_one(&body.code[offset..]) else {
                return false;
            };
            if instruction.opcode == zun_il::Opcode::Yield {
                return true;
            }
            offset += size;
        }
        false
    }

    /// Whether `handle` is an arrow function (its closure name carries `@arrow`).
    /// Whether `handle` owns a `prototype` key, which is node's shape: a plain
    /// function and a generator do; an arrow, an async function and a method
    /// shorthand do not. The exclusions are explicit because
    /// `function_is_constructor` does not reject an arrow (that rejection lives
    /// in the construct path) and it *does* reject a generator, which owns one.
    pub(crate) fn function_owns_prototype(&self, handle: zun_core::FunctionHandle) -> bool {
        // A **bound** function owns no `prototype`: `function_is_constructor`
        // answers true for one by an early return, so exclude it here.
        !handle.is_bound()
            && !self.is_arrow_closure(handle)
            && !self.is_method_closure(handle)
            && (self.is_generator_function(handle)
                || (self.function_is_constructor(handle)
                    && !self.is_async_function(handle)))
    }

    /// Whether `handle` is a `function*` (its declared name carries the
    /// `@generator` marker). Needed beside `method_yields` because a generator
    /// with no `yield` in its body is invisible to that scan.
    pub(crate) fn is_generator_function(&self, handle: zun_core::FunctionHandle) -> bool {
        !handle.is_bound()
            && handle.builtin_method().is_none()
            && self
                .module
                .tables
                .method_defs
                .get((handle.method_rid() as usize).wrapping_sub(1))
                .is_some_and(|method| {
                    self.module.strings.get(method.name).contains("@generator")
                })
    }

    /// Whether `handle` is an `async function` (its declared name carries the
    /// `@async` marker). An async function is not a constructor and owns no
    /// `prototype`, and nothing else at run time distinguishes one: zun
    /// compiles it as a generator, so `method_yields` answers the same for an
    /// async function and a real generator, and an empty body defeats that
    /// scan outright.
    pub(crate) fn is_async_function(&self, handle: zun_core::FunctionHandle) -> bool {
        !handle.is_bound()
            && handle.builtin_method().is_none()
            && self
                .module
                .tables
                .method_defs
                .get((handle.method_rid() as usize).wrapping_sub(1))
                .is_some_and(|method| self.module.strings.get(method.name).contains("@async"))
    }

    pub(crate) fn is_arrow_closure(&self, handle: zun_core::FunctionHandle) -> bool {
        !handle.is_bound()
            && handle.builtin_method().is_none()
            && self
                .module
                .tables
                .method_defs
                .get((handle.method_rid() as usize).wrapping_sub(1))
                .is_some_and(|method| self.module.strings.get(method.name).contains("@arrow"))
    }

    /// Whether `handle` is an object-literal method shorthand closure (`@method`).
    pub(crate) fn is_method_closure(&self, handle: zun_core::FunctionHandle) -> bool {
        !handle.is_bound()
            && handle.builtin_method().is_none()
            && self
                .module
                .tables
                .method_defs
                .get((handle.method_rid() as usize).wrapping_sub(1))
                .is_some_and(|method| self.module.strings.get(method.name).contains("@method"))
    }

    /// An ordinary function owns a `prototype` object (with a non-enumerable
    /// back-reference `constructor`) from the moment it is created. Functions
    /// here materialise it lazily, on the first read of `fn.prototype`, unless
    /// the script already stored its own value.
    /// `w[i]` on a `String` wrapper object: the code unit at the canonical
    /// index `key`, an own property of the wrapper exactly as on its primitive.
    fn string_wrapper_unit(&mut self, object: zun_gc::Obj, key: &Value) -> Option<Value> {
        let index = match key {
            Value::Int(n, _) if *n >= 0 => *n as usize,
            Value::Float(n, _) if *n >= 0.0 && n.fract() == 0.0 && *n < 4_294_967_295.0 => {
                *n as usize
            }
            Value::Str(text) => text.parse::<usize>().ok().filter(|i| i.to_string() == **text)?,
            _ => return None,
        };
        let text = self.string_wrapper_text(object)?;
        let unit = zun_core::wtf16::encode(&text).nth(index)?;
        Some(Value::Str(zun_core::wtf16::from_units(&[unit]).into()))
    }

    /// Whether `value` is a function value, bare or carried through `any`
    /// (a callable rides in a `__boxedFn` record there).
    fn is_function_value(value: &Value) -> bool {
        let concrete = match value {
            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)),
            other => Some(other.clone()),
        };
        concrete.is_some_and(|value| {
            matches!(
                zun_gc::with_current_heap(|heap| crate::collections::unbox_function_box(value, heap)),
                Value::Function(_)
            )
        })
    }

    /// The member `name` a function value inherits from `Function.prototype`:
    /// one of the built-in method values (`call`, `apply`, `bind`, `toString`)
    /// or whatever a program stored there. Read off the prototype object, so a
    /// write to or a `delete` from `Function.prototype` is seen.
    pub(crate) fn function_prototype_member(&self, name: &str) -> Option<Value> {
        let proto = Self::function_prototype_singleton();
        zun_gc::with_current_heap(|heap| {
            if let Some(any) = heap.dynamic_get(proto, name) {
                return Some(Value::Any(any));
            }
            heap.dynamic_get_function(proto, name).map(Value::Function)
        })
    }

    pub(crate) fn ensure_function_prototype(&self, handle: zun_core::FunctionHandle) {
        if handle.builtin_method().is_some() || handle.is_bound() {
            return;
        }
        // An arrow function owns no `prototype` (and is not constructible); its
        // closure name carries the `@arrow` marker.
        if self.is_arrow_closure(handle) || self.is_method_closure(handle) {
            return;
        }
        let bag = crate::function_expando_bag(handle);
        zun_gc::with_current_heap_mut(|heap| {
            if heap.dynamic_get(bag, "prototype").is_some() {
                return;
            }
            let proto = heap.alloc_dynamic();
            // `F.prototype.constructor === F`, writable, non-enumerable and
            // configurable. A function value does not box into an `Any`, so
            // it is installed as a (non-enumerable) function slot; boxing it
            // here used to yield nothing and leave the property undefined.
            heap.dynamic_set_method(proto, "constructor", handle);
            let proto_any = zun_core::Any::object(proto.addr() as usize);
            heap.dynamic_define_data(bag, "prototype", proto_any, true, false, false);
        });
    }
}
