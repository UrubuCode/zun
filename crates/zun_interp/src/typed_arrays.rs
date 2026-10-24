//! The interpreter's share of the typed-array wiring, generated from the
//! master table in `zun_core`.
//!
//! Tier 0 is the always-correct fallback, so a kind that the checker accepts
//! and the interpreter has no arm for is a program that compiles and then
//! fails to run. Generating the dispatch from the same table the checker
//! reads removes that possibility.

use super::value::Value;
use super::RunError;
use zun_il::NumKind;

/// How one element crosses between the heap's Rust representation and an
/// interpreter [`Value`].
///
/// Written once per Rust primitive rather than once per typed-array kind:
/// `Uint8Array` and `Uint8ClampedArray` both store `u8`, and both cross here
/// the same way.
pub(crate) trait ElementValue: Sized {
    fn into_value(self) -> Value;
    fn from_value(value: &Value) -> Option<Self>;
}

macro_rules! integer_element_value {
    ($($repr:ty => $kind:ident),+ $(,)?) => {
        $(
            impl ElementValue for $repr {
                fn into_value(self) -> Value {
                    Value::Int(self as i64, NumKind::$kind)
                }

                fn from_value(value: &Value) -> Option<Self> {
                    match value {
                        // The interpreter carries every integer width in an
                        // `i64`; narrowing back is the same truncation the
                        // IL's store already performed.
                        Value::Int(value, _) => Some(*value as $repr),
                        _ => None,
                    }
                }
            }
        )+
    };
}

integer_element_value! {
    i8 => I8, i16 => I16, i32 => I32, i64 => I64,
    u8 => U8, u16 => U16, u32 => U32, u64 => U64,
}

impl ElementValue for f32 {
    fn into_value(self) -> Value {
        Value::Float(f64::from(self), NumKind::F32)
    }

    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Float(value, _) => Some(*value as f32),
            _ => None,
        }
    }
}

impl ElementValue for f64 {
    fn into_value(self) -> Value {
        Value::Float(self, NumKind::F64)
    }

    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::Float(value, _) => Some(*value),
            _ => None,
        }
    }
}

fn object(value: &Value) -> Option<zun_gc::Obj> {
    match value {
        Value::Object(object) => Some(*object),
        Value::Any(any) => any
            .as_object()
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
        _ => None,
    }
}

fn index(value: Option<&Value>) -> Result<usize, RunError> {
    match value.ok_or(RunError::BadToken)? {
        Value::Int(value, _) => Ok(*value as usize),
        _ => Err(RunError::BadToken),
    }
}

macro_rules! typed_array_dispatch {
    ($({
        variant: $variant:ident, repr: $repr:ty, arg: $arg:ty, width: $width:expr,
        js: $js:literal, native: $native:literal,
        elem_ty: $elem_ty:literal, arg_ty: $arg_ty:literal,
        view_ty: $view_ty:ident, elem_native: $elem_native:ident,
        arg_native: $arg_native:ident,
        encode: $encode:path,
        read: $read:ident, write: $write:ident,
        get: $get:ident, set: $set:ident
    }),+ $(,)?) => {
        /// Every `zun:buffer` intrinsic the typed-array kinds contribute.
        pub(crate) const NATIVES: &[&str] = &[
            $(
                concat!($native, "New"),
                concat!($native, "Length"),
                concat!($native, "ByteLength"),
                concat!($native, "Get"),
                concat!($native, "Set"),
            )+
        ];

        /// Runs a typed-array intrinsic, or answers `None` when `name` is not
        /// one — the caller then carries on with the rest of its dispatch.
        pub(crate) fn call(name: &str, args: &[Value]) -> Option<Result<Value, RunError>> {
            // `length` and `byteLength` do not depend on the kind: the view
            // knows its own element width. They are matched together so the
            // per-kind arms below are only the ones that need the kind.
            if let Some(measure) = measurement(name, args) {
                return Some(measure);
            }
            match name {
                $(
                    concat!($native, "New") => Some(new_view(args, zun_core::TypedElement::$variant)),
                    concat!($native, "Get") => Some((|| {
                        let view = object(args.first()?)?;
                        let index = index(args.get(1)).ok()?;
                        let element =
                            zun_gc::with_current_heap(|heap| heap.$get(view, index))?;
                        Some(ElementValue::into_value(element))
                    })()
                    .ok_or(RunError::BadToken)),
                    concat!($native, "Set") => Some((|| {
                        let view = object(args.first()?)?;
                        let index = index(args.get(1)).ok()?;
                        let value = <$arg as ElementValue>::from_value(args.get(2)?)?;
                        let stored = zun_gc::with_current_heap_mut(|heap| {
                            heap.$set(view, index, value)
                        })?;
                        Some(Value::Bool(stored))
                    })()
                    .ok_or(RunError::BadToken)),
                )+
                _ => None,
            }
        }

        /// `length`/`byteLength` for any kind.
        fn measurement(name: &str, args: &[Value]) -> Option<Result<Value, RunError>> {
            let elements = match name {
                $(concat!($native, "Length"))|+ => true,
                $(concat!($native, "ByteLength"))|+ => false,
                _ => return None,
            };
            Some((|| {
                let view = object(args.first()?)?;
                let measure = zun_gc::with_current_heap(|heap| {
                    if elements {
                        heap.typed_array_len(view)
                    } else {
                        heap.typed_array_byte_len(view)
                    }
                })?;
                Some(Value::Int(measure as i64, NumKind::U64))
            })()
            .ok_or(RunError::BadToken))
        }
    };
}

/// `new <Kind>Array(buffer, offset, length)`, where `length` counts elements.
fn new_view(args: &[Value], element: zun_core::TypedElement) -> Result<Value, RunError> {
    let buffer = args.first().and_then(object).ok_or(RunError::BadToken)?;
    let offset = index(args.get(1))?;
    let length = index(args.get(2))?;
    let view = zun_gc::with_current_heap_mut(|heap| {
        heap.alloc_typed_array(buffer, offset, length, element)
    })
    .ok_or(RunError::BadToken)?;
    Ok(Value::Object(view))
}

zun_core::typed_element_table!(typed_array_dispatch);
