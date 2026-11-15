//! The `Object` statics on a primitive receiver.
//!
//! `Object.getPrototypeOf("ab")`, `Object.getOwnPropertyNames(5)` and friends
//! apply ToObject to their argument: `null` and `undefined` are a `TypeError`,
//! every other primitive behaves as its wrapper object. A string owns its code
//! units (enumerable, read-only) and `length`; a number, boolean or symbol owns
//! nothing; each inherits from its constructor's prototype. This answers those
//! cases without allocating the wrapper; objects fall through to the ordinary
//! implementations.

use super::*;

/// A primitive receiver of an `Object` static.
enum Primitive {
    Nullish,
    Str(String),
    Number,
    Boolean,
    Symbol,
}

fn classify(arg: Option<&Value>) -> Option<Primitive> {
    match unbox_arg(arg?) {
        Ok(Value::Void | Value::Null) => Some(Primitive::Nullish),
        Ok(Value::Str(text)) => Some(Primitive::Str(text.to_string())),
        Ok(Value::Int(..) | Value::Float(..)) => Some(Primitive::Number),
        Ok(Value::Bool(_)) => Some(Primitive::Boolean),
        Ok(Value::Any(any)) if any.as_symbol().is_some() => Some(Primitive::Symbol),
        _ => None,
    }
}

const NULLISH: &str = "TypeError: Cannot convert undefined or null to object";

/// The own property of a string primitive named `key`: its value and whether it
/// is enumerable (`length` is not).
fn string_own(text: &str, key: &str) -> Option<(Value, bool)> {
    let length = zun_core::wtf16::len16(text);
    if key == "length" {
        return Some((Value::Float(length as f64, zun_il::NumKind::F64), false));
    }
    // A canonical array index: digits only, no leading zero.
    let canonical = !key.is_empty()
        && key.bytes().all(|b| b.is_ascii_digit())
        && (key == "0" || !key.starts_with('0'));
    let index = key.parse::<usize>().ok().filter(|_| canonical)?;
    let unit = zun_core::wtf16::encode(text).nth(index)?;
    Some((Value::Str(zun_core::wtf16::from_units(&[unit]).into()), true))
}

impl Interp<'_> {
    fn empty_array() -> Value {
        Value::Object(zun_gc::with_current_heap_mut(|heap| heap.alloc_array(0)))
    }

    fn string_index_names(count: usize, with_length: bool) -> Value {
        let array = zun_gc::with_current_heap_mut(|heap| {
            let array = heap.alloc_array(count + usize::from(with_length));
            for index in 0..count {
                let item = Value::Str(index.to_string().into())
                    .box_any(heap)
                    .unwrap_or_else(zun_core::Any::null);
                let _ = heap.array_set(array, index, item);
            }
            if with_length {
                let item = Value::Str("length".into())
                    .box_any(heap)
                    .unwrap_or_else(zun_core::Any::null);
                let _ = heap.array_set(array, count, item);
            }
            array
        });
        Value::Object(array)
    }

    /// The own-property key an `Object` static received as its second argument
    /// (`None` for a symbol, which no primitive owns).
    fn primitive_key(&mut self, arg: Option<&Value>) -> Result<Option<String>, RunError> {
        let Some(arg) = arg else { return Ok(Some("undefined".to_owned())) };
        match crate::property_ops::prop_key(arg) {
            Some(crate::property_ops::PropKey::Str(key)) => Ok(Some(key)),
            Some(crate::property_ops::PropKey::Sym(_)) => Ok(None),
            None => Ok(Some(self.dyn_to_string(&unbox_arg(arg)?)?)),
        }
    }

    /// Answer the `Object` static `name` when its receiver is a primitive.
    /// `None` (or `Err(BadToken)`) leaves the call to the ordinary (object)
    /// implementation.
    pub(super) fn object_primitive_receiver(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Option<Result<Value, RunError>> {
        if !matches!(
            name,
            "objectKeys"
                | "objectValues"
                | "objectEntries"
                | "objectGetPrototypeOf"
                | "objectGetOwnPropertyNames"
                | "objectGetOwnPropertySymbols"
                | "objectGetOwnPropertyDescriptor"
                | "objectGetOwnPropertyDescriptors"
                | "objectHasOwn"
        ) {
            return None;
        }
        let primitive = classify(args.first())?;
        match self.object_primitive_answer(name, args, primitive) {
            Err(RunError::BadToken) => None,
            answer => Some(answer),
        }
    }

    fn object_primitive_answer(
        &mut self,
        name: &str,
        args: &[Value],
        primitive: Primitive,
    ) -> Result<Value, RunError> {
        if matches!(primitive, Primitive::Nullish) {
            return Err(RunError::Native(NULLISH.into()));
        }
        match name {
            "objectGetPrototypeOf" => {
                let class = match primitive {
                    Primitive::Str(_) => "String",
                    Primitive::Number => "Number",
                    Primitive::Boolean => "Boolean",
                    _ => "Symbol",
                };
                let proto = if class == "Symbol" {
                    self.builtin_prototype_by_name(class).ok_or(RunError::BadToken)?
                } else {
                    self.wrapper_prototype_object(class).ok_or(RunError::BadToken)?
                };
                Ok(Value::Any(zun_core::Any::object(proto.addr() as usize)))
            }
            "objectGetOwnPropertyNames" => Ok(match primitive {
                Primitive::Str(text) => Self::string_index_names(zun_core::wtf16::len16(&text), true),
                _ => Self::empty_array(),
            }),
            // A string's keys, values and entries are built by the ordinary
            // path; a number, boolean or symbol owns nothing.
            "objectKeys" | "objectValues" | "objectEntries" => match primitive {
                Primitive::Str(_) => Err(RunError::BadToken),
                _ => Ok(Self::empty_array()),
            },
            "objectGetOwnPropertySymbols" => Ok(Self::empty_array()),
            "objectGetOwnPropertyDescriptors" => match primitive {
                // The ordinary path builds a string's descriptors.
                Primitive::Str(_) | Primitive::Number | Primitive::Boolean => {
                    Err(RunError::BadToken)
                }
                _ => Ok(Value::Object(zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic()))),
            },
            "objectGetOwnPropertyDescriptor" | "objectHasOwn" => {
                let key = self.primitive_key(args.get(1))?;
                let own = match (&primitive, &key) {
                    (Primitive::Str(text), Some(key)) => string_own(text, key),
                    _ => None,
                };
                if name == "objectHasOwn" {
                    return Ok(Value::Bool(own.is_some()));
                }
                let Some((value, enumerable)) = own else { return Ok(Value::Void) };
                let out = zun_gc::with_current_heap_mut(|heap| {
                    let out = heap.alloc_dynamic();
                    let any = value.box_any(heap).unwrap_or_else(zun_core::Any::void);
                    heap.dynamic_set(out, "value".to_owned(), any);
                    heap.dynamic_set(out, "writable".to_owned(), zun_core::Any::bool(false));
                    heap.dynamic_set(out, "enumerable".to_owned(), zun_core::Any::bool(enumerable));
                    heap.dynamic_set(out, "configurable".to_owned(), zun_core::Any::bool(false));
                    out
                });
                Ok(Value::Object(out))
            }
            _ => Err(RunError::BadToken),
        }
    }
}

impl Interp<'_> {
    /// The class token of the prelude wrapper class of the constructor `ctor`
    /// (`"Number"` -> `NumberObject`).
    pub(super) fn wrapper_class_token(&self, ctor: &str) -> Option<u32> {
        let wrapper_name = format!("{ctor}Object");
        self.module
            .tables
            .type_defs
            .iter()
            .position(|def| self.module.strings.get(def.name) == wrapper_name)
            .map(|index| zun_natives::USER_CLASS_FLAG | (index as u32 + 1))
    }

    /// ToObject for a primitive: its wrapper object (`"ab"` -> `StringObject`).
    /// `None` for an object, `null` and `undefined`, which the caller handles.
    /// A symbol has no wrapper class and becomes a plain object.
    pub(crate) fn primitive_wrapper_object(
        &mut self,
        value: &Value,
    ) -> Result<Option<zun_gc::Obj>, RunError> {
        let primitive = match unbox_arg(value)? {
            primitive @ (Value::Str(_) | Value::Int(..) | Value::Float(..) | Value::Bool(_)) => {
                primitive
            }
            Value::Any(any) if any.as_symbol().is_some() => {
                return Ok(Some(zun_gc::with_current_heap_mut(|heap| heap.alloc_dynamic())));
            }
            _ => return Ok(None),
        };
        let ctor = match &primitive {
            Value::Str(_) => "String",
            Value::Bool(_) => "Boolean",
            _ => "Number",
        };
        let token = self
            .wrapper_class_token(ctor)
            .ok_or_else(|| RunError::Native("TypeError: value is not a constructor".into()))?;
        let wrapper = self.construct_class_from_descriptor(token, vec![primitive])?;
        Ok(object(&wrapper))
    }
}
