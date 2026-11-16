//! Calling a built-in static method as a first-class function value.
//!
//! `Math.max`, `Object.keys` and `Reflect.get` are real function objects: they
//! can be read off the namespace or constructor through an `any` binding,
//! inspected (`name`, `length`) and invoked on their own. The values are
//! installed by [`crate::intrinsic_meta`]; this module runs one. A member whose
//! table entry names a native is shaped from that native's declared parameter
//! kinds and dispatched through the same managed-call path a typed call site
//! lowers to, so each method keeps exactly one implementation. `Math` runs
//! through `zun_natives::apply_math`, the name-based evaluator `Math.f.apply`
//! already uses.

use super::value::Value;
use super::{Interp, RunError};
use crate::intrinsic_meta::static_method_at;
use crate::proto_methods::proto_concrete;
use zun_natives::NativeTy;

impl Interp<'_> {
    fn static_type_error(&mut self, message: &str) -> RunError {
        match self.new_error_object("TypeError", message) {
            Some(object) => RunError::Thrown(Value::Object(object)),
            None => RunError::Native(format!("TypeError: {message}")),
        }
    }

    /// ToNumber for a `Math` argument: a Symbol or BigInt is a `TypeError`.
    fn static_to_number(&mut self, value: Value) -> Result<f64, RunError> {
        match &value {
            Value::Any(any) if any.as_symbol().is_some() => {
                return Err(self.static_type_error("Cannot convert a Symbol value to a number"));
            }
            Value::Object(object)
                if zun_gc::with_current_heap(|heap| heap.bigint_value(*object).is_some()) =>
            {
                return Err(self.static_type_error("Cannot convert a BigInt value to a number"));
            }
            _ => {}
        }
        self.to_number_value(value)
    }

    /// A property-key argument: a symbol stays boxed, anything else is
    /// converted with ToPropertyKey (string hint).
    fn static_key(&mut self, value: Value) -> Result<Value, RunError> {
        let value = match value {
            Value::Any(any) if any.as_symbol().is_some() => return Ok(Value::Any(any)),
            other => proto_concrete(other),
        };
        Ok(match value {
            Value::Str(_) => value,
            other => Value::Str(self.dyn_to_string(&other)?.into()),
        })
    }

    /// An argument-list value: an array as it is, an array-like converted to a
    /// temporary array, anything else a `TypeError`.
    fn static_list(&mut self, value: Value, what: &str) -> Result<Value, RunError> {
        let value = proto_concrete(value);
        let Value::Object(object) = &value else {
            return Err(self.static_type_error(&format!("{what} called on non-object")));
        };
        if zun_gc::with_current_heap(|heap| heap.array_len(*object).is_some()) {
            return Ok(value);
        }
        let boxed = self.proto_box(value);
        self.call_managed_collection("arrayFromArrayLike", &[boxed])
    }

    /// Run the static method value of `kind`.
    pub(crate) fn call_static_method(
        &mut self,
        kind: u32,
        _this: Value,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        let Some((class, name, _, native)) = static_method_at(kind) else {
            return Err(RunError::BadToken);
        };
        // Every argument rides in a rooted array and is re-read through it: a
        // conversion or a getter below can run managed code that moves objects.
        let count = args.len();
        let holder = zun_gc::with_current_heap_mut(|heap| {
            let list = heap.alloc_array(count);
            for (index, value) in args.iter().enumerate() {
                // A callable rides in a function cell so the array can hold it.
                let any = value.box_any_cell(heap).unwrap_or_else(zun_core::Any::void);
                heap.array_set(list, index, any);
            }
            list
        });
        drop(args);
        zun_gc::root!(holder = holder);
        let arg = |index: usize| -> Value {
            zun_gc::with_current_heap(|heap| heap.array_get(holder.get(), index))
                .map_or_else(|| Value::Any(zun_core::Any::void()), Value::Any)
        };

        if class == "Math" {
            let mut numbers = Vec::with_capacity(count);
            for index in 0..count {
                let value = proto_concrete(arg(index));
                numbers.push(self.static_to_number(value)?);
            }
            let result = zun_natives::apply_math(name, &numbers).unwrap_or(f64::NAN);
            return Ok(Value::Float(result, zun_il::NumKind::F64));
        }
        if class == "Array" {
            return self.call_array_static(name, _this, count, &arg);
        }
        if class == "Number" {
            return self.call_number_static(name, count, &arg);
        }
        if !native.is_empty() {
            let (_, signature) =
                zun_natives::resolve("collections", native).ok_or(RunError::BadToken)?;
            let mut call = Vec::with_capacity(signature.params.len());
            for (index, ty) in signature.params.iter().enumerate() {
                call.push(match ty {
                    NativeTy::Function | NativeTy::Array => proto_concrete(arg(index)),
                    NativeTy::Str => self.static_key(arg(index))?,
                    _ => self.proto_box(arg(index)),
                });
            }
            return self.call_managed_collection(native, &call);
        }
        match (class, name) {
            ("Object", "assign") => {
                // Fold the sources into the target, which is re-read each round.
                for index in 1..count.max(2) {
                    let call = [self.proto_box(arg(0)), self.proto_box(arg(index))];
                    self.call_managed_collection("objectAssign", &call)?;
                }
                Ok(self.proto_box(arg(0)))
            }
            ("Object", "create") => {
                let proto = self.proto_box(arg(0));
                let created = self.call_managed_collection("objectCreate", &[proto])?;
                let Value::Object(created) = created else {
                    return Ok(created);
                };
                let props = proto_concrete(arg(1));
                if count >= 2 && !matches!(props, Value::Void) {
                    zun_gc::root!(created = created);
                    let call = [Value::Object(created.get()), self.proto_box(arg(1))];
                    self.call_managed_collection("objectDefineProperties", &call)?;
                    return Ok(Value::Object(created.get()));
                }
                Ok(Value::Object(created))
            }
            ("Reflect", "preventExtensions") => {
                let target = self.proto_box(arg(0));
                self.call_managed_collection("objectPreventExtensions", &[target])?;
                Ok(Value::Bool(true))
            }
            ("Reflect", "get") => {
                let key = self.static_key(arg(1))?;
                self.reflect_get_value(proto_concrete(arg(0)), key)
            }
            ("Reflect", "apply") => {
                let target = proto_concrete(arg(0));
                let list = self.static_list(arg(2), "CreateListFromArrayLike")?;
                let this_arg = self.proto_box(arg(1));
                match &target {
                    Value::Function(_) => {
                        self.call_managed_collection("functionApply", &[target, this_arg, list])
                    }
                    Value::Object(object)
                        if zun_gc::with_current_heap(|heap| heap.proxy_target(*object).is_some()) =>
                    {
                        self.call_managed_collection("proxyApply", &[target, this_arg, list])
                    }
                    _ => Err(self.static_type_error(
                        "Function.prototype.apply was called on a non-function, which is not a function",
                    )),
                }
            }
            ("Reflect", "construct") => {
                let target = proto_concrete(arg(0));
                let list = self.static_list(arg(1), "CreateListFromArrayLike")?;
                if let Value::Object(object) = &target {
                    if zun_gc::with_current_heap(|heap| heap.proxy_target(*object).is_some()) {
                        return self.call_managed_collection("proxyConstruct", &[target, list]);
                    }
                }
                let boxed = self.proto_box(arg(0));
                if count >= 3 && !matches!(proto_concrete(arg(2)), Value::Void) {
                    let new_target = self.proto_box(arg(2));
                    return self
                        .call_managed_collection("reflectConstructNew", &[boxed, list, new_target]);
                }
                self.call_managed_collection("reflectConstruct", &[boxed, list])
            }
            _ => Err(RunError::BadToken),
        }
    }

    /// `Number.<name>(...)` as a function value, on the same `number` natives a
    /// typed `Number.isInteger(x)` call lowers to. The predicates inspect the
    /// argument without coercion; the parsers convert to a string first (and
    /// `parseInt`'s radix with ToInt32), exactly like the typed lowering.
    fn call_number_static(
        &mut self,
        name: &str,
        count: usize,
        arg: &dyn Fn(usize) -> Value,
    ) -> Result<Value, RunError> {
        let native = match name {
            "isNaN" => "isNaNStrict",
            "isFinite" => "isFiniteStrict",
            "isInteger" => "isIntegerStrict",
            "isSafeInteger" => "isSafeIntegerStrict",
            "parseFloat" => "parseFloatPrefix",
            "parseInt" => "parseIntRadix",
            _ => return Err(RunError::BadToken),
        };
        let (id, _) = zun_natives::resolve("number", native).ok_or(RunError::BadToken)?;
        let first = proto_concrete(arg(0));
        let call = match name {
            "parseFloat" | "parseInt" => {
                let text = Value::Str(self.dyn_to_string(&first)?.into());
                if name == "parseFloat" {
                    vec![text]
                } else {
                    let radix = if count >= 2 {
                        let radix = proto_concrete(arg(1));
                        let number = self.static_to_number(radix)?;
                        // ToInt32: NaN and the infinities are 0.
                        if number.is_finite() {
                            (number.trunc() % 4_294_967_296.0) as i64 as i32
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    vec![text, Value::Int(i64::from(radix), zun_il::NumKind::I32)]
                }
            }
            _ => vec![first],
        };
        let native_args: Vec<zun_natives::NativeArg> =
            call.iter().map(crate::native::to_native_arg).collect();
        let result = zun_natives::dispatch(id, &native_args, self.output)
            .map_err(|error| RunError::Native(error.0))?;
        Ok(crate::native::from_native_value(result, id))
    }

    /// `Reflect.get(target, key)`: the value of `key` on `target`, through the
    /// same member read an `any` receiver uses.
    fn reflect_get_value(&mut self, target: Value, key: Value) -> Result<Value, RunError> {
        let Value::Object(object) = target else {
            return Err(self.static_type_error("Reflect.get called on non-object"));
        };
        match key {
            Value::Str(name) => {
                let is_array = zun_gc::with_current_heap(|heap| heap.array_len(object).is_some());
                if is_array {
                    if &*name == "length" {
                        let length = zun_gc::with_current_heap(|heap| heap.array_len(object));
                        return Ok(Value::Float(
                            length.unwrap_or(0) as f64,
                            zun_il::NumKind::F64,
                        ));
                    }
                    if let Ok(at) = name.parse::<usize>() {
                        let element = zun_gc::with_current_heap(|heap| heap.array_get(object, at));
                        return Ok(element.map_or(Value::Void, Value::Any));
                    }
                }
                self.dynamic_member_get(object, &name)
            }
            Value::Any(symbol) => {
                let Some(symbol) = symbol.as_symbol() else {
                    return Ok(Value::Void);
                };
                Ok(zun_gc::with_current_heap(|heap| heap.dynamic_get_symbol(object, symbol))
                    .map_or(Value::Void, Value::Any))
            }
            _ => Ok(Value::Void),
        }
    }
}

impl Interp<'_> {
    /// `Array.isArray` / `Array.of` / `Array.from` as function values. `of` and
    /// `from` are generic over `this`: a constructor other than `Array` itself
    /// builds the result (`Array.of.call(Box, "x")`), anything else gets a
    /// plain array.
    fn call_array_static(
        &mut self,
        name: &str,
        this: Value,
        count: usize,
        arg: &dyn Fn(usize) -> Value,
    ) -> Result<Value, RunError> {
        match name {
            "isArray" => {
                let value = self.proto_box(arg(0));
                self.call_managed_collection("arrayIsArray", &[value])
            }
            "of" => {
                let list = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(count));
                zun_gc::root!(list = list);
                for index in 0..count {
                    if let Value::Any(any) = arg(index) {
                        zun_gc::with_current_heap_mut(|heap| heap.array_set(list.get(), index, any));
                    }
                }
                self.array_static_result(this, list.get(), Some(count))
            }
            "from" => {
                // The mapper is validated before the source is touched.
                let mapper = proto_concrete(arg(1));
                let source = proto_concrete(arg(0));
                let usable_mapper = matches!(mapper, Value::Void | Value::Function(_));
                let empty = || Value::Object(zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0)));
                let list = match (&mapper, &source) {
                    (_, Value::Void | Value::Null) if usable_mapper => {
                        let shown = if matches!(source, Value::Void) { "undefined" } else { "object null" };
                        return Err(self.static_type_error(&format!(
                            "{shown} is not iterable (cannot read property Symbol(Symbol.iterator))"
                        )));
                    }
                    // A number or boolean is neither iterable nor array-like.
                    (_, Value::Float(..) | Value::Int(..) | Value::Bool(_)) if usable_mapper => empty(),
                    (Value::Void, _) => self.call_managed_collection("arrayFrom", &[arg(0)])?,
                    (Value::Function(handle), _) => {
                        let mut callback = Value::Function(*handle);
                        if count >= 3 && !matches!(proto_concrete(arg(2)), Value::Void) {
                            let bound = zun_gc::with_current_heap_mut(|heap| heap.alloc_array(1));
                            if let Value::Any(any) = arg(2) {
                                zun_gc::with_current_heap_mut(|heap| heap.array_set(bound, 0, any));
                            }
                            callback = self
                                .call_managed_collection("functionBind", &[callback, Value::Object(bound)])?;
                        }
                        self.call_managed_collection("arrayFromMapped", &[arg(0), callback])?
                    }
                    // Not callable: the native raises the TypeError.
                    _ => self.call_managed_collection("arrayFromMapped", &[arg(0), arg(1)])?,
                };
                let Value::Object(list) = list else {
                    return Ok(list);
                };
                // An iterable source constructs with no argument, an array-like
                // with its length.
                let iterable = match proto_concrete(arg(0)) {
                    Value::Str(_) => true,
                    Value::Object(object) => zun_gc::with_current_heap(|heap| {
                        heap.array_len(object).is_some()
                            || heap.typed_array_len(object).is_some()
                            || heap.map_len(object).is_some()
                            || heap.set_len(object).is_some()
                            || heap.generator_state(object).is_some()
                            || heap
                                .dynamic_get_symbol_function(
                                    object,
                                    zun_core::WellKnownSymbol::Iterator.id(),
                                )
                                .is_some()
                    }),
                    _ => false,
                };
                let length = zun_gc::with_current_heap(|heap| heap.array_len(list)).unwrap_or(0);
                self.array_static_result(this, list, (!iterable).then_some(length))
            }
            _ => Err(RunError::BadToken),
        }
    }

    /// The result of `Array.of` / `Array.from` for the receiver `this`: `list`
    /// itself, or — when `this` is a constructor other than `Array` — a
    /// `new this(length?)` given the elements of `list` and its `length`.
    fn array_static_result(
        &mut self,
        this: Value,
        list: zun_gc::Obj,
        construct_length: Option<usize>,
    ) -> Result<Value, RunError> {
        let is_constructor = match proto_concrete(this.clone()) {
            Value::Function(handle) => {
                self.coerce_ctor_name(handle).is_none() && self.function_is_constructor(handle)
            }
            _ => Self::class_token_of(&this).is_some_and(|token| {
                token & zun_natives::USER_CLASS_FLAG != 0
                    || zun_gc::with_current_heap_mut(|heap| {
                        crate::intrinsic_meta::builtin_class_name_in(heap, token)
                    })
                    .as_deref()
                        != Some("Array")
            }),
        };
        if !is_constructor {
            return Ok(Value::Object(list));
        }
        zun_gc::root!(list = list);
        let ctor_args = zun_gc::with_current_heap_mut(|heap| {
            let args = heap.alloc_array(0);
            if let Some(length) = construct_length {
                heap.array_push(args, zun_core::Any::float(length as f64));
            }
            args
        });
        // A function has no `any` form; a class value travels boxed.
        let target = match proto_concrete(this.clone()) {
            Value::Function(handle) => Value::Function(handle),
            _ => self.proto_box(this),
        };
        let made = self.call_managed_collection("reflectConstruct", &[target, Value::Object(ctor_args)])?;
        let Some(made) = crate::collections::object(&made) else {
            return Ok(made);
        };
        zun_gc::root!(made = made);
        let length = zun_gc::with_current_heap(|heap| heap.array_len(list.get())).unwrap_or(0);
        for index in 0..=length {
            let (key, value) = if index == length {
                ("length".to_owned(), zun_core::Any::float(length as f64))
            } else {
                let element = zun_gc::with_current_heap(|heap| heap.array_get(list.get(), index))
                    .filter(|any| !any.is_hole())
                    .unwrap_or_else(zun_core::Any::void);
                (index.to_string(), element)
            };
            let key = self.proto_box(Value::Str(key.into()));
            let call = [Value::Object(made.get()), key, Value::Any(value)];
            self.reflect_set(&call)?;
        }
        Ok(Value::Object(made.get()))
    }
}
