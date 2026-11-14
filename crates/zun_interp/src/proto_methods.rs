//! Calling a built-in prototype method as a first-class function value.
//!
//! `X.prototype.m` is a real function object: it can be read, inspected and
//! invoked on any receiver (`Array.prototype.map.call(arrayLike, fn)`). The
//! values themselves are installed by [`crate::intrinsic_meta`]; this module
//! runs one. It checks the receiver the way the specification does (a wrong
//! receiver is a `TypeError`, never an engine panic) and then dispatches
//! through the same member-call path an `any`-typed receiver uses, so a method
//! has one implementation whether it is called directly or borrowed.

use super::value::Value;
use super::{Interp, RunError};
use crate::intrinsic_meta::prototype_method_at;

/// Array methods that only read the receiver: an array-like receiver is
/// converted to a temporary array first (`CreateListFromArrayLike`).
const ARRAY_READ_ONLY: &[&str] = &[
    "at", "concat", "entries", "every", "filter", "find", "findIndex", "findLast",
    "findLastIndex", "flat", "flatMap", "forEach", "includes", "indexOf", "join", "keys",
    "lastIndexOf", "map", "reduce", "reduceRight", "slice", "some", "toReversed", "toSorted",
    "toSpliced", "values", "with",
];

impl Interp<'_> {
    fn proto_type_error(&mut self, message: &str) -> RunError {
        match self.new_error_object("TypeError", message) {
            Some(object) => RunError::Thrown(Value::Object(object)),
            None => RunError::Native(format!("TypeError: {message}")),
        }
    }

    /// `Method X.prototype.m called on incompatible receiver <receiver>`.
    fn proto_incompatible(&mut self, class: &str, name: &str, recv: &Value) -> RunError {
        let shown = match recv {
            Value::Void => "undefined".to_owned(),
            Value::Null => "null".to_owned(),
            Value::Str(text) => text.to_string(),
            Value::Bool(flag) => flag.to_string(),
            Value::Float(n, _) => zun_natives::js_to_radix_string(*n, None).unwrap_or_default(),
            Value::Int(n, _) => n.to_string(),
            Value::Function(_) => "function".to_owned(),
            Value::Object(object) => {
                if zun_gc::with_current_heap(|heap| heap.array_len(*object).is_some()) {
                    "[object Array]".to_owned()
                } else {
                    "#<Object>".to_owned()
                }
            }
            Value::Any(_) => "#<Object>".to_owned(),
        };
        self.proto_type_error(&format!(
            "Method {class}.prototype.{name} called on incompatible receiver {shown}"
        ))
    }

    /// Dispatch `recv.name(...args)` through the dynamic member-call path.
    fn proto_dynamic_call(
        &mut self,
        recv: Value,
        name: &str,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        let mut call = Vec::with_capacity(args.len() + 2);
        call.push(recv);
        call.push(Value::Str(name.into()));
        call.extend(args);
        self.call_managed_collection("dynamicCall0", &call)
    }

    /// Run the prototype method value of `kind` with `this` and `args`.
    pub(crate) fn call_prototype_method(
        &mut self,
        kind: u32,
        this: Value,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        let Some((class, name, _)) = prototype_method_at(kind) else {
            return Err(RunError::BadToken);
        };
        let recv = proto_concrete(this);
        let args: Vec<Value> = args.into_iter().map(proto_concrete).collect();
        let object = match &recv {
            Value::Object(object) => Some(*object),
            _ => None,
        };
        match class {
            "WeakRef" => {
                let target = object
                    .filter(|o| zun_gc::with_current_heap(|heap| heap.is_weak_ref(*o)))
                    .ok_or_else(|| self.proto_incompatible(class, name, &recv))?;
                self.call_managed_collection("weakRefDeref", &[Value::Object(target)])
            }
            "WeakMap" | "WeakSet" => {
                let ok = object
                    .is_some_and(|o| zun_gc::with_current_heap(|heap| heap.is_weak_collection(o)));
                if !ok {
                    return Err(self.proto_incompatible(class, name, &recv));
                }
                self.proto_dynamic_call(recv, name, args)
            }
            "Promise" => {
                let ok = object
                    .is_some_and(|o| zun_gc::with_current_heap(|heap| heap.promise_state(o).is_some()));
                if !ok {
                    return Err(self.proto_incompatible(class, name, &recv));
                }
                self.proto_dynamic_call(recv, name, args)
            }
            "RegExp" => {
                let regex = object
                    .filter(|o| zun_gc::with_current_heap(|heap| heap.regex_source(*o).is_some()))
                    .ok_or_else(|| self.proto_incompatible(class, name, &recv))?;
                let input = match args.first() {
                    Some(value) => {
                        let value = match value {
                            Value::Any(any) => {
                                zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                    .unwrap_or(Value::Void)
                            }
                            other => other.clone(),
                        };
                        self.dyn_to_string(&value)?
                    }
                    None => "undefined".to_owned(),
                };
                let native = if name == "test" { "regexpTest" } else { "regexpExec" };
                self.call_managed_collection(native, &[Value::Object(regex), Value::Str(input.into())])
            }
            "Symbol" => {
                let id = match &recv {
                    Value::Any(any) => any.as_symbol(),
                    _ => None,
                }
                .ok_or_else(|| {
                    self.proto_type_error(&format!(
                        "Symbol.prototype.{name} requires that 'this' be a Symbol"
                    ))
                })?;
                if name == "valueOf" {
                    return Ok(recv);
                }
                let description = zun_gc::with_current_heap(|heap| {
                    heap.symbol_description(id).map(str::to_owned)
                })
                .unwrap_or_default();
                Ok(Value::Str(format!("Symbol({description})").into()))
            }
            "BigInt" => {
                let big = object
                    .filter(|o| zun_gc::with_current_heap(|heap| heap.bigint_value(*o).is_some()))
                    .ok_or_else(|| {
                        self.proto_type_error(&format!(
                            "BigInt.prototype.{name} requires that 'this' be a BigInt"
                        ))
                    })?;
                if name == "valueOf" {
                    return Ok(recv);
                }
                let radix = match args.first() {
                    None | Some(Value::Void) => 10.0,
                    Some(value) => {
                        let value = value.clone();
                        self.to_number_value(value)?
                    }
                };
                if radix == 10.0 {
                    return self.call_managed_collection("bigIntToString", &[Value::Object(big)]);
                }
                if !(2.0..=36.0).contains(&radix) || radix.fract() != 0.0 {
                    return Err(self.proto_range_error("toString() radix argument must be between 2 and 36"));
                }
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(big).cloned())
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Str(bigint_to_radix(&value, radix as u64).into()))
            }
            "Number" => {
                // `Number.prototype` is itself a Number object whose value is +0.
                let recv = match recv {
                    Value::Object(o)
                        if self.builtin_prototype_by_name("Number") == Some(o) =>
                    {
                        Value::Float(0.0, zun_il::NumKind::F64)
                    }
                    other => other,
                };
                // A `Number` wrapper object (or an instance of a subclass of it)
                // carries its primitive in `value`.
                let recv = match recv {
                    Value::Object(o) if self.is_number_object(o) => {
                        match self.dynamic_member_get(o, "value")? {
                            Value::Any(any) => zun_gc::with_current_heap(|heap| {
                                Value::unbox_any(any, heap)
                            })
                            .unwrap_or(Value::Void),
                            other => other,
                        }
                    }
                    other => other,
                };
                if !matches!(recv, Value::Float(..) | Value::Int(..)) {
                    return Err(self.proto_type_error(&format!(
                        "Number.prototype.{name} requires that 'this' be a Number"
                    )));
                }
                self.proto_dynamic_call(recv, name, args)
            }
            "String" => {
                let text = match &recv {
                    Value::Str(_) => recv.clone(),
                    Value::Void | Value::Null => {
                        return Err(self.proto_type_error(&format!(
                            "String.prototype.{name} called on null or undefined"
                        )));
                    }
                    other if matches!(name, "toString" | "valueOf") => {
                        let _ = other;
                        return Err(self.proto_type_error(&format!(
                            "String.prototype.{name} requires that 'this' be a String"
                        )));
                    }
                    other => Value::Str(self.dyn_to_string(other)?.into()),
                };
                if name == "concat" {
                    // `concat` takes any number of values, each converted with ToString.
                    let mut joined = match &text {
                        Value::Str(s) => s.to_string(),
                        _ => String::new(),
                    };
                    for arg in &args {
                        let arg = match arg {
                            Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap))
                                .unwrap_or(Value::Void),
                            other => other.clone(),
                        };
                        joined.push_str(&self.dyn_to_string(&arg)?);
                    }
                    return Ok(Value::Str(joined.into()));
                }
                self.proto_dynamic_call(text, name, args)
            }
            "Function" => {
                if !matches!(recv, Value::Function(_)) {
                    return Err(self.proto_type_error(&format!(
                        "Function.prototype.{name} requires that 'this' be a Function"
                    )));
                }
                self.proto_dynamic_call(recv, name, args)
            }
            "TypedArray" => {
                let ok = object.is_some_and(|o| {
                    zun_gc::with_current_heap(|heap| heap.typed_array_element(o).is_some())
                });
                if !ok {
                    return Err(self.proto_incompatible("TypedArray", name, &recv));
                }
                self.proto_dynamic_call(recv, name, args)
            }
            "Array" => self.call_array_prototype_method(name, recv, args),
            _ => Err(RunError::BadToken),
        }
    }

    fn proto_range_error(&mut self, message: &str) -> RunError {
        match self.new_error_object("RangeError", message) {
            Some(object) => RunError::Thrown(Value::Object(object)),
            None => RunError::Native(format!("RangeError: {message}")),
        }
    }

    /// `Array.prototype.<name>` on any receiver: an array runs the typed path, an
    /// array-like goes through the generic algorithms.
    fn call_array_prototype_method(
        &mut self,
        name: &str,
        recv: Value,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        if matches!(recv, Value::Void | Value::Null) {
            return Err(self.proto_type_error(&format!(
                "Array.prototype.{name} called on null or undefined"
            )));
        }
        let is_array = matches!(&recv, Value::Object(o)
            if zun_gc::with_current_heap(|heap| heap.array_len(*o).is_some()));
        let mut args = args;
        if name == "flat" && matches!(args.first(), None | Some(Value::Void)) {
            // `flat` defaults its depth to 1.
            args = vec![Value::Float(1.0, zun_il::NumKind::F64)];
        }
        if is_array {
            return self.proto_dynamic_call(recv, name, args);
        }
        if name == "toString" {
            // Generic `toString` calls `join` when the receiver has one.
            let join = match &recv {
                Value::Object(o) => self.dynamic_member_get(*o, "join")?,
                _ => Value::Void,
            };
            let join = match join {
                Value::Any(any) => zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
                    .unwrap_or(Value::Void),
                other => other,
            };
            return match (join, &recv) {
                (Value::Function(handle), Value::Object(o)) => {
                    let call = self.with_this_arg(handle, *o, Vec::new());
                    self.invoke_function_value(handle, call)
                }
                _ => Ok(Value::Str("[object Object]".into())),
            };
        }
        match name {
            "push" | "fill" | "reverse" | "pop" => {
                let native = match name {
                    "push" => "arrayLikePush",
                    "fill" => "arrayLikeFill",
                    "reverse" => "arrayLikeReverse",
                    _ => "arrayLikePop",
                };
                let mut call = vec![self.proto_box(recv)];
                if name != "pop" {
                    let list = zun_gc::with_current_heap_mut(|heap| {
                        let list = heap.alloc_array(args.len());
                        for (index, value) in args.iter().enumerate() {
                            let boxed = value.box_any(heap).unwrap_or_else(zun_core::Any::void);
                            heap.array_set(list, index, boxed);
                        }
                        list
                    });
                    call.push(Value::Object(list));
                }
                self.call_managed_collection(native, &call)
            }
            "concat" => {
                // A non-array receiver is not spread: it becomes the first element.
                let boxed = self.proto_box(recv);
                let first = zun_gc::with_current_heap_mut(|heap| {
                    let list = heap.alloc_array(1);
                    if let Value::Any(any) = boxed {
                        heap.array_set(list, 0, any);
                    }
                    list
                });
                self.proto_dynamic_call(Value::Object(first), name, args)
            }
            _ if ARRAY_READ_ONLY.contains(&name) => {
                let temp = match &recv {
                    // A primitive other than a string has no indexed properties.
                    Value::Float(..) | Value::Int(..) | Value::Bool(_) | Value::Function(_) => {
                        Value::Object(zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0)))
                    }
                    _ => {
                        let boxed = self.proto_box(recv);
                        self.call_managed_collection("arrayFromArrayLike", &[boxed])?
                    }
                };
                self.proto_dynamic_call(temp, name, args)
            }
            _ => Err(self.proto_type_error(&format!(
                "Array.prototype.{name} on a non-array receiver is not supported in this runtime"
            ))),
        }
    }

    pub(crate) fn proto_box(&mut self, value: Value) -> Value {
        match value {
            Value::Any(_) => value,
            other => Value::Any(
                zun_gc::with_current_heap_mut(|heap| other.box_any(heap))
                    .unwrap_or_else(zun_core::Any::void),
            ),
        }
    }
}

/// `BigInt.prototype.toString(radix)` for a radix other than 10.
fn bigint_to_radix(value: &zun_core::BigInt, radix: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value.is_zero() {
        return "0".to_owned();
    }
    let divisor = zun_core::BigInt::from_u64(radix);
    let mut rest = if value.is_negative() { value.neg() } else { value.clone() };
    let mut digits = Vec::new();
    while !rest.is_zero() {
        let Some((quotient, remainder)) = rest.div_rem(&divisor) else {
            break;
        };
        digits.push(DIGITS[remainder.to_u64().unwrap_or(0) as usize]);
        rest = quotient;
    }
    if value.is_negative() {
        digits.push(b'-');
    }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_default()
}

/// The concrete value behind an `any` carrier, with a callable stored in a
/// `__boxedFn` record turned back into a function value.
pub(crate) fn proto_concrete(value: Value) -> Value {
    zun_gc::with_current_heap(|heap| {
        let inner = match value {
            Value::Any(any) => Value::unbox_any(any, heap).unwrap_or(Value::Void),
            other => other,
        };
        crate::collections::unbox_function_box(inner, heap)
    })
}

impl Interp<'_> {
    /// Whether `object` is an instance of the prelude `NumberObject` wrapper class
    /// or of a class derived from it.
    fn is_number_object(&self, object: zun_gc::Obj) -> bool {
        let mut rid = object.type_rid();
        for _ in 0..64 {
            let Some(def) = self.module.tables.type_defs.get((rid as usize).wrapping_sub(1)) else {
                return false;
            };
            if self.module.strings.get(def.name) == "NumberObject" {
                return true;
            }
            if def.base.table() != Some(zun_metadata::TableKind::TypeDef) || def.base.is_null() {
                return false;
            }
            rid = def.base.rid();
        }
        false
    }
}
