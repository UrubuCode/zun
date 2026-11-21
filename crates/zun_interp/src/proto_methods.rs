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
/// The `ARRAY_READ_ONLY` methods that take a callback as their first argument.
const ARRAY_TAKES_CALLBACK: &[&str] = &[
    "every", "filter", "find", "findIndex", "findLast", "findLastIndex", "flatMap", "forEach",
    "map", "reduce", "reduceRight", "some",
];

const ARRAY_READ_ONLY: &[&str] = &[
    "at", "concat", "entries", "every", "filter", "find", "findIndex", "findLast",
    "findLastIndex", "flat", "flatMap", "forEach", "includes", "indexOf", "join", "keys",
    "lastIndexOf", "map", "reduce", "reduceRight", "slice", "some", "toReversed", "toSorted",
    "toSpliced", "values", "with", "toLocaleString",
];

impl Interp<'_> {
    /// A typed field slot cannot hold `null`/`undefined`: name the field and
    /// the value rather than aborting the engine (see
    /// `value::unboxed_field_rejects`).
    /// Whether `object` carries the brand for a **private** field, i.e. its
    /// class chain actually reaches the type that declares the field.
    ///
    /// Without this, a prototype swap was **type confusion**, not merely a
    /// wrong answer. `class Money { #amount: number }` and
    /// `class Label { #text: string }`, then
    /// `Object.setPrototypeOf(money, Label.prototype)` and `money.read()`:
    /// dispatch correctly follows the swapped prototype into `Label.read`,
    /// whose `LoadField` then read slot 0 of a *Money* object as a string,
    /// interpreting the bits of the float `1234.5` as an intern id and
    /// answering the unrelated live string `"Math"`. A larger float indexes
    /// out of range. Reachable from ordinary JS, no `any` needed.
    ///
    /// The dynamic-object guard at the use site does not cover it: a typed
    /// instance of the *wrong* class is not a dynamic object. Privates are
    /// installed by the declaring constructor, so a subclass instance does
    /// carry them, which is why this walks the base chain instead of
    /// comparing one rid.
    pub(crate) fn private_field_brand_ok(&self, field_rid: u32, object: zun_gc::Obj) -> bool {
        let Some(owner) = self
            .module
            .tables
            .type_defs
            .iter()
            .position(|ty| ty.field_start <= field_rid && field_rid < ty.field_end)
            .map(|index| index as u32 + 1)
        else {
            return true;
        };
        let mut current = object.type_rid();
        for _ in 0..64 {
            if current == 0 {
                return false;
            }
            if current == owner {
                return true;
            }
            let Some(type_def) = self.module.tables.type_defs.get(current as usize - 1) else {
                return false;
            };
            current = type_def.base.rid();
        }
        false
    }

    /// Whether the field this token names is declared private.
    pub(crate) fn field_rid_is_private(&self, field_rid: u32) -> bool {
        self.module
            .tables
            .field_defs
            .get((field_rid as usize).wrapping_sub(1))
            .is_some_and(|field| field.is_private)
    }

    pub(crate) fn field_slot_message(&self, field_rid: u32, value: &Value) -> String {
        let name = self
            .module
            .tables
            .field_defs
            .get((field_rid as usize).wrapping_sub(1))
            .map(|field| self.module.strings.get(field.name).to_owned())
            .unwrap_or_default();
        // Name what was actually offered: a string slot now refuses every
        // non-string, so "undefined" for a number would misdescribe it.
        let shown = match value {
            Value::Null => "null".to_owned(),
            Value::Void => "undefined".to_owned(),
            Value::Bool(_) => "a boolean".to_owned(),
            Value::Int(..) | Value::Float(..) => "a number".to_owned(),
            Value::Str(_) => "a string".to_owned(),
            Value::Function(_) => "a function".to_owned(),
            _ => "that value".to_owned(),
        };
        format!("TypeError: Cannot store {shown} in the typed field `{name}`")
    }

    /// The source name of a field token, or `None` when the field is
    /// `#private`. A public field addressed on a plain dynamic object is an
    /// ordinary property read or write by that name; a private one is a brand
    /// mismatch and must still throw.
    pub(crate) fn public_field_name(&self, token: zun_metadata::Token) -> Option<String> {
        let field = self
            .module
            .tables
            .field_defs
            .get((token.rid() as usize).wrapping_sub(1))?;
        if field.is_private {
            return None;
        }
        let name = self.module.strings.get(field.name);
        if name.starts_with('#') {
            return None;
        }
        Some(name.to_owned())
    }

    /// Node's wording for a typed field access whose receiver is not an
    /// object: `Cannot read properties of undefined (reading 'tag')`, or
    /// `Cannot set properties of null (setting 'tag')`.
    pub(crate) fn field_receiver_error(
        &mut self,
        token: zun_metadata::Token,
        receiver: &Value,
        storing: bool,
    ) -> RunError {
        let field = self
            .module
            .tables
            .field_defs
            .get((token.rid() as usize).wrapping_sub(1))
            .map(|field| self.module.strings.get(field.name).to_owned())
            .unwrap_or_default();
        let shown = match receiver {
            Value::Void => "undefined".to_owned(),
            Value::Null => "null".to_owned(),
            other => crate::collections::describe_received(other),
        };
        let (verb, gerund) = if storing {
            ("set", "setting")
        } else {
            ("read", "reading")
        };
        self.proto_type_error(&format!(
            "Cannot {verb} properties of {shown} ({gerund} '{field}')"
        ))
    }

    pub(crate) fn proto_type_error(&mut self, message: &str) -> RunError {
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
    pub(crate) fn proto_dynamic_call(
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
                    // Node names a symbol-keyed method as
                    // `Symbol.prototype [ @@toPrimitive ]`, not with a dot.
                    let shown = if name == "[Symbol.toPrimitive]" {
                        "Symbol.prototype [ @@toPrimitive ]".to_owned()
                    } else {
                        format!("Symbol.prototype.{name}")
                    };
                    self.proto_type_error(&format!("{shown} requires that 'this' be a Symbol"))
                })?;
                // `Symbol.prototype[Symbol.toPrimitive](hint)` answers the
                // symbol itself whatever the hint, exactly as `valueOf` does.
                // It is a *separate* function object, not an alias: node
                // reports its `name` as `[Symbol.toPrimitive]` and its `length`
                // as 1, so aliasing `valueOf` here would have answered
                // `valueOf:0` -- right behaviour, wrong reflection.
                if name == "valueOf" || name == "[Symbol.toPrimitive]" {
                    return Ok(recv);
                }
                let description = zun_gc::with_current_heap(|heap| {
                    heap.symbol_description(id).map(str::to_owned)
                })
                .unwrap_or_default();
                Ok(Value::Str(format!("Symbol({description})").into()))
            }
            "BigInt" => {
                // thisBigIntValue: a BigInt, or a BigInt object's payload.
                let unwrapped = match object {
                    Some(candidate) => self.bigint_wrapper_payload(candidate)?,
                    None => None,
                };
                let recv = match unwrapped {
                    Some(big) => Value::Object(big),
                    None => recv,
                };
                let object = unwrapped.or(object);
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
                        // ToIntegerOrInfinity: a fraction truncates, NaN is 0.
                        let radix = self.to_number_value(value)?;
                        if radix.is_nan() {
                            0.0
                        } else {
                            radix.trunc()
                        }
                    }
                };
                if radix == 10.0 {
                    return self.call_managed_collection("bigIntToString", &[Value::Object(big)]);
                }
                if !(2.0..=36.0).contains(&radix) {
                    return Err(self.proto_range_error("toString() radix argument must be between 2 and 36"));
                }
                let value = zun_gc::with_current_heap(|heap| heap.bigint_value(big).cloned())
                    .ok_or(RunError::BadToken)?;
                Ok(Value::Str(bigint_to_radix(&value, radix as u64).into()))
            }
            "Boolean" => {
                let recv = self.wrapper_primitive(recv, "Boolean")?;
                let Value::Bool(flag) = recv else {
                    return Err(self.proto_type_error(&format!(
                        "Boolean.prototype.{name} requires that 'this' be a Boolean"
                    )));
                };
                Ok(if name == "valueOf" {
                    Value::Bool(flag)
                } else {
                    Value::Str(if flag { "true" } else { "false" }.into())
                })
            }
            "Number" => {
                // `Number.prototype` is itself a Number object whose value is +0,
                // and a `Number` wrapper object (or an instance of a subclass of
                // it) carries its primitive in `value`.
                let recv = self.wrapper_primitive(recv, "Number")?;
                if !matches!(recv, Value::Float(..) | Value::Int(..)) {
                    return Err(self.proto_type_error(&format!(
                        "Number.prototype.{name} requires that 'this' be a Number"
                    )));
                }
                self.proto_dynamic_call(recv, name, args)
            }
            "String" => {
                // `String.prototype` is a String object whose value is `""`.
                let recv = self.wrapper_primitive(recv, "String")?;
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
                // `Function.prototype.toString`. A built-in constructor value
                // (`Object`, `Date`) reads as its native-function text; any
                // other function keeps the generic object text, since no source
                // text is retained for it.
                if name == "toString" {
                    if let Some(text) = self.native_function_text(&recv) {
                        return Ok(Value::Str(text.into()));
                    }
                    if matches!(recv, Value::Function(_) | Value::Any(_)) {
                        return self.call_object_prototype_method("toString", recv, args);
                    }
                }
                // A constructor callable without `new` (`Symbol`) acts through
                // its call behaviour, as `Symbol.call(...)` does.
                let recv = match self.callable_ctor_handle(&recv) {
                    Some(handle) => Value::Function(handle),
                    None => recv,
                };
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
                crate::collections::request_builtin_member_only();
                self.proto_dynamic_call(recv, name, args)
            }
            "Array" => self.call_array_prototype_method(name, recv, args),
            "Object" => self.call_object_prototype_method(name, recv, args),
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
            let message = zun_natives::array_null_receiver_message(name);
            return Err(self.proto_type_error(&message));
        }
        let is_array = matches!(&recv, Value::Object(o)
            if zun_gc::with_current_heap(|heap| heap.array_len(*o).is_some()));
        let mut args = args;
        if name == "flat" && matches!(args.first(), None | Some(Value::Void)) {
            // `flat` defaults its depth to 1.
            args = vec![Value::Float(1.0, zun_il::NumKind::F64)];
        }
        // A typed array is an array-like whose writes must land in its own
        // elements, never in a temporary copy.
        if let Value::Object(view) = &recv {
            if crate::typed_arrays::element_len(*view).is_some() && name != "concat" {
                return self.array_method_on_typed_array(name, *view, args);
            }
        }
        if is_array {
            crate::collections::request_builtin_member_only();
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
                // The spec validates a callback with IsCallable before reading
                // any element, so for the callback-taking methods the check has
                // to happen before the array-like is materialized below -- the
                // conversion reads index 0, which a Proxy observes. The static
                // `Array.prototype.m.call(...)` form is guarded in sema; this is
                // the run-time path a computed `(Array.prototype as any)[m]`
                // takes, where sema never sees the method name.
                if ARRAY_TAKES_CALLBACK.contains(&name)
                    && !matches!(
                        proto_concrete(args.first().cloned().unwrap_or(Value::Void)),
                        Value::Function(_)
                    )
                {
                    return Err(self.proto_type_error(&format!(
                        "Array.prototype.{name} callback is not a function"
                    )));
                }
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
pub(crate) fn bigint_to_radix(value: &zun_core::BigInt, radix: u64) -> String {
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
    /// Whether `object` is an instance of the prelude `<ctor>Object` wrapper
    /// class (`NumberObject`, `BooleanObject`, `StringObject`) or of a class
    /// derived from it.
    fn is_wrapper_object(&self, object: zun_gc::Obj, wrapper: &str) -> bool {
        let mut rid = object.type_rid();
        for _ in 0..64 {
            let Some(def) = self.module.tables.type_defs.get((rid as usize).wrapping_sub(1)) else {
                return false;
            };
            let name = self.module.strings.get(def.name);
            // `class C extends Number` derives from the builtin-base class
            // `__BuiltinNumber` rather than from `NumberObject`; its instances
            // hold the [[NumberData]] all the same (in `inner`).
            if name == wrapper
                || (wrapper == "NumberObject" && name == "__BuiltinNumber")
                || (wrapper == "BooleanObject" && name == "__BuiltinBoolean")
            {
                return true;
            }
            if def.base.table() != Some(zun_metadata::TableKind::TypeDef) || def.base.is_null() {
                return false;
            }
            rid = def.base.rid();
        }
        false
    }

    /// Which of `"Boolean"`, `"Number"`, `"String"` the primitive-wrapper
    /// object `object` is, if any: an instance of the prelude wrapper class, or
    /// the constructor's own `prototype` (the specification makes each of those
    /// a wrapper object holding `false`, `+0` and `""`). Allocates nothing.
    pub(crate) fn wrapper_kind(&self, object: zun_gc::Obj) -> Option<&'static str> {
        const KINDS: [(&str, &str); 3] =
            [("Boolean", "BooleanObject"), ("Number", "NumberObject"), ("String", "StringObject")];
        if object.type_rid() != 0 {
            if let Some((ctor, _)) =
                KINDS.iter().find(|(_, wrapper)| self.is_wrapper_object(object, wrapper))
            {
                return Some(ctor);
            }
            // A BigInt object (`Object(1n)`). Only the instance: unlike the
            // other three, `BigInt.prototype` is not itself a wrapper.
            if self.is_wrapper_object(object, "BigIntObject") {
                return Some("BigInt");
            }
        }
        KINDS.iter().map(|(ctor, _)| *ctor).find(|ctor| {
            zun_gc::with_current_heap_mut(|heap| {
                let global = heap.global_object();
                let key = if *ctor == "String" {
                    "$String.prototype".to_owned()
                } else {
                    let Some(token) =
                        heap.dynamic_get(global, ctor).and_then(|any| any.as_class_descriptor())
                    else {
                        return false;
                    };
                    format!("$builtinProto#{token}")
                };
                heap.dynamic_get(global, &key).and_then(|any| any.as_object())
                    == Some(object.addr() as usize)
            })
        })
    }

    /// The `prototype` object of the primitive-wrapper constructor `ctor`.
    /// `String.prototype` is the realm singleton the typed `String.prototype`
    /// reads; `Number` and `Boolean` own a per-descriptor prototype.
    pub(crate) fn wrapper_prototype_object(&self, ctor: &str) -> Option<zun_gc::Obj> {
        if ctor == "String" {
            Some(Self::string_prototype_singleton())
        } else {
            self.builtin_prototype_by_name(ctor)
        }
    }

    /// The prototype object instances of the prelude wrapper class `rid`
    /// inherit from: `StringObject` stands in for `String`, so its instances
    /// share `String.prototype` (and likewise `Number` / `Boolean`) instead of
    /// a class prototype of their own.
    pub(crate) fn wrapper_class_prototype(&self, rid: u32) -> Option<zun_gc::Obj> {
        let def = self.module.tables.type_defs.get((rid as usize).wrapping_sub(1))?;
        let ctor = match self.module.strings.get(def.name) {
            "StringObject" => "String",
            "NumberObject" => "Number",
            "BooleanObject" => "Boolean",
            "BigIntObject" => "BigInt",
            _ => return None,
        };
        self.wrapper_prototype_object(ctor)
    }

    /// Which wrapper constructor the *instance* `object` belongs to (`new
    /// String("ab")`, or an instance of a class derived from it). The
    /// constructors' own prototypes are wrapper objects too but are not
    /// instances, and answer `None`.
    pub(crate) fn wrapper_instance_kind(&self, object: zun_gc::Obj) -> Option<&'static str> {
        // A core runtime type id aliases the `TypeDef` rid space, so the rid
        // alone does not make an object an instance.
        if !self.type_rid_describes_object(object)
            || zun_gc::with_current_heap(|heap| {
                heap.is_dynamic_object(object) && !heap.is_instance_bag(object)
            })
        {
            return None;
        }
        ["Boolean", "Number", "String", "BigInt"]
            .into_iter()
            .find(|ctor| self.is_wrapper_object(object, &format!("{ctor}Object")))
    }

    /// The text of the `String` wrapper instance `object`.
    pub(crate) fn string_wrapper_text(&mut self, object: zun_gc::Obj) -> Option<String> {
        if self.wrapper_instance_kind(object) != Some("String") {
            return None;
        }
        match self.wrapper_primitive(Value::Object(object), "String") {
            Ok(Value::Str(text)) => Some(text.to_string()),
            _ => None,
        }
    }

    /// Whether `key` names one of the properties a `String` wrapper instance
    /// owns by construction -- a code-unit index or `length`. They are
    /// non-writable and non-configurable, exactly as on the primitive.
    pub(crate) fn string_wrapper_owns(&mut self, object: zun_gc::Obj, key: &str) -> bool {
        let Some(text) = self.string_wrapper_text(object) else {
            return false;
        };
        key == "length"
            || key
                .parse::<usize>()
                .ok()
                .filter(|index| index.to_string() == key)
                .is_some_and(|index| index < zun_core::wtf16::len16(&text))
    }

    /// [`Self::string_wrapper_owns`] for a computed key.
    pub(crate) fn string_wrapper_owns_key(&mut self, object: zun_gc::Obj, key: &Value) -> bool {
        match key {
            Value::Str(name) => self.string_wrapper_owns(object, name),
            Value::Int(index, _) => self.string_wrapper_owns(object, &index.to_string()),
            Value::Float(index, _) if index.fract() == 0.0 && *index >= 0.0 => {
                self.string_wrapper_owns(object, &(*index as u64).to_string())
            }
            _ => false,
        }
    }

    /// A function a program stored on the prototype of the wrapper constructor
    /// `ctor` under `member` (`String.prototype.shout = function () {..}`). The
    /// built-in method values installed there are not reported: a primitive
    /// receiver runs those on its natives.
    pub(crate) fn installed_wrapper_method(
        &self,
        ctor: &str,
        member: &str,
    ) -> Option<zun_core::FunctionHandle> {
        let proto = self.wrapper_prototype_object(ctor)?;
        let found = zun_gc::with_current_heap(|heap| {
            heap.dynamic_get_function(proto, member)
                .map(Value::Function)
                .or_else(|| heap.dynamic_get(proto, member).map(Value::Any))
        })?;
        match proto_concrete(found) {
            Value::Function(handle) if handle.builtin_method().is_none() => Some(handle),
            _ => None,
        }
    }

    /// The primitive a `this` value of the wrapper constructor `ctor` stands
    /// for: a wrapper instance yields its `value`, the constructor's prototype
    /// its default (`false`, `+0`, `""`), anything else is returned unchanged.
    pub(crate) fn wrapper_primitive(&mut self, recv: Value, ctor: &str) -> Result<Value, RunError> {
        let Value::Object(object) = recv else { return Ok(recv) };
        if self.wrapper_kind(object) != Some(ctor) {
            return Ok(Value::Object(object));
        }
        if self.wrapper_prototype_object(ctor) == Some(object) {
            return Ok(match ctor {
                "Boolean" => Value::Bool(false),
                "Number" => Value::Float(0.0, zun_il::NumKind::F64),
                _ => Value::Str("".into()),
            });
        }
        let held = match self.dynamic_member_get(object, "value")? {
            Value::Any(any) => {
                zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap)).unwrap_or(Value::Void)
            }
            other => other,
        };
        if !matches!(held, Value::Void) {
            return Ok(held);
        }
        // A subclass of the builtin base keeps the primitive in `inner`.
        Ok(self.instance_member_get(object, "inner")?.unwrap_or(Value::Void))
    }
}

impl Interp<'_> {
    /// `Object.prototype.<name>` on any receiver. This is the one implementation
    /// of the six methods: the unbound function value read off `Object.prototype`
    /// reaches it through `call_prototype_method`, and the receiver-bound stand-in
    /// of a plain object reaches it with that object as the receiver.
    pub(crate) fn call_object_prototype_method(
        &mut self,
        name: &str,
        recv: Value,
        args: Vec<Value>,
    ) -> Result<Value, RunError> {
        let recv = proto_concrete(recv);
        let mut args = args.into_iter().map(proto_concrete);
        let arg = args.next().unwrap_or(Value::Void);
        let second = args.next().unwrap_or(Value::Void);
        let nullish = matches!(recv, Value::Void | Value::Null);
        // Every method but `toString` / `isPrototypeOf` coerces the receiver with
        // ToObject first, which rejects `undefined` and `null`.
        let reject_nullish = |this: &mut Self| {
            this.proto_type_error("Cannot convert undefined or null to object")
        };
        match name {
            "toString" => self.call_managed_collection("objectToStringTag", &[recv]),
            "toLocaleString" => {
                if nullish {
                    return Err(reject_nullish(self));
                }
                self.proto_dynamic_call(recv, "toString", Vec::new())
            }
            "valueOf" => {
                if nullish {
                    return Err(reject_nullish(self));
                }
                // A function rides the `any` lane as a `functionBox` record.
                if matches!(recv, Value::Function(_)) {
                    return self.call_managed_collection("functionBox", &[recv]);
                }
                // ToObject: a primitive becomes its wrapper object.
                Ok(match self.primitive_wrapper_object(&recv)? {
                    Some(wrapper) => Value::Object(wrapper),
                    None => recv,
                })
            }
            "hasOwnProperty" => {
                if nullish {
                    return Err(reject_nullish(self));
                }
                self.call_managed_collection("objectHasOwn", &[recv, arg])
            }
            "isPrototypeOf" => {
                let (Value::Object(value), true) = (&arg, !nullish) else {
                    if nullish && matches!(arg, Value::Object(_)) {
                        return Err(reject_nullish(self));
                    }
                    return Ok(Value::Bool(false));
                };
                match &recv {
                    Value::Object(proto) => Ok(Value::Bool(self.proto_chain_contains(*value, *proto)?)),
                    _ => Ok(Value::Bool(false)),
                }
            }
            "propertyIsEnumerable" => {
                if nullish {
                    return Err(reject_nullish(self));
                }
                let descriptor = self.call_managed_collection(
                    "objectGetOwnPropertyDescriptor",
                    &[recv, arg],
                )?;
                let Value::Object(descriptor) = descriptor else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(zun_gc::with_current_heap(|heap| {
                    heap.dynamic_get(descriptor, "enumerable")
                        .is_some_and(|any| crate::collections::is_truthy(&Value::Any(any), heap))
                })))
            }
            "__defineGetter__" | "__defineSetter__" => {
                if nullish {
                    return Err(reject_nullish(self));
                }
                let Value::Function(handle) = second else {
                    return Err(self.proto_type_error(&format!(
                        "Object.prototype.{name}: Expecting function"
                    )));
                };
                let key = self.dyn_to_string(&arg)?;
                // A primitive receiver is wrapped by ToObject; the definition lands
                // on the temporary wrapper, so nothing observable remains.
                let Value::Object(target) = recv else {
                    return Ok(Value::Void);
                };
                let is_getter = name == "__defineGetter__";
                let defined = zun_gc::with_current_heap_mut(|heap| {
                    let getter = if is_getter {
                        Some(handle)
                    } else {
                        heap.dynamic_accessor_getter(target, &key)
                    };
                    let setter = if is_getter {
                        heap.dynamic_accessor_setter(target, &key)
                    } else {
                        Some(handle)
                    };
                    heap.dynamic_define_accessor(target, key.clone(), getter, setter, true, true)
                });
                if !defined {
                    return Err(self.proto_type_error(&format!("Cannot redefine property: {key}")));
                }
                Ok(Value::Void)
            }
            "__lookupGetter__" | "__lookupSetter__" => {
                if nullish {
                    return Err(reject_nullish(self));
                }
                let key = self.dyn_to_string(&arg)?;
                let want_getter = name == "__lookupGetter__";
                let mut current = match recv {
                    Value::Object(object) => Some(object),
                    _ => None,
                };
                let root = Self::object_prototype_singleton();
                for _ in 0..256 {
                    let Some(holder) = current else { break };
                    let step = zun_gc::with_current_heap(|heap| {
                        let accessor = if want_getter {
                            heap.dynamic_accessor_getter(holder, &key)
                        } else {
                            heap.dynamic_accessor_setter(holder, &key)
                        };
                        let is_accessor = heap.dynamic_accessor_getter(holder, &key).is_some()
                            || heap.dynamic_accessor_setter(holder, &key).is_some();
                        let shadowed = !is_accessor
                            && (heap.dynamic_has(holder, &key)
                                || heap.dynamic_get_function(holder, &key).is_some());
                        let next = heap
                            .dynamic_prototype(holder)
                            .and_then(|any| any.as_object())
                            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                        let explicit_null = heap
                            .dynamic_prototype(holder)
                            .is_some_and(|any| any.as_object().is_none());
                        (accessor, shadowed, next, explicit_null)
                    });
                    let (accessor, shadowed, next, explicit_null) = step;
                    if let Some(found) = accessor {
                        // A callable has no plain `Any` form; the function cell
                        // stands for it on the dynamic lane.
                        let cell = zun_gc::with_current_heap_mut(|heap| {
                            Value::Function(found).box_any_cell(heap)
                        });
                        return Ok(cell.map(Value::Any).unwrap_or(Value::Void));
                    }
                    if shadowed {
                        return Ok(Value::Void);
                    }
                    // No explicit link: an ordinary object inherits `Object.prototype`.
                    current = match next {
                        Some(next) => Some(next),
                        None if explicit_null || holder == root => None,
                        None => Some(root),
                    };
                }
                Ok(Value::Void)
            }
            _ => Err(RunError::BadToken),
        }
    }
}

impl Interp<'_> {
    /// Whether `object` is a BigInt wrapper (`Object(1n)`): an instance of the
    /// prelude `BigIntObject` class.
    pub(crate) fn is_bigint_wrapper(&self, object: zun_gc::Obj) -> bool {
        self.type_rid_describes_object(object)
            && !zun_gc::with_current_heap(|heap| {
                heap.is_dynamic_object(object) && !heap.is_instance_bag(object)
            })
            && self.class_chain_has_name(object.type_rid(), "BigIntObject")
    }

    /// The BigInt a wrapper holds ([[BigIntData]]), as the BigInt value.
    pub(crate) fn bigint_wrapper_payload(
        &mut self,
        object: zun_gc::Obj,
    ) -> Result<Option<zun_gc::Obj>, RunError> {
        if !self.is_bigint_wrapper(object) {
            return Ok(None);
        }
        let held = match self.instance_member_get(object, "value")? {
            Some(Value::Any(any)) => {
                zun_gc::with_current_heap(|heap| Value::unbox_any(any, heap))
            }
            other => other,
        };
        Ok(match held {
            Some(Value::Object(big))
                if zun_gc::with_current_heap(|heap| heap.bigint_value(big).is_some()) =>
            {
                Some(big)
            }
            _ => None,
        })
    }
}
