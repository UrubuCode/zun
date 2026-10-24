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

/// A number crossing *into* a typed-array element from a dynamic value (a
/// callback result, a search argument, an element of a source array). Unlike
/// [`ElementValue::from_value`], which is strict about the `Value` variant
/// because the checker has already coerced the operand, this accepts either
/// numeric representation — a callback returning a bare integer literal must
/// still store into a `Float64Array`, and vice versa.
pub(crate) trait FromNumeric: Sized {
    fn from_numeric(value: &Value) -> Option<Self>;
}

macro_rules! int_from_numeric {
    ($($repr:ty),+ $(,)?) => {
        $(
            impl FromNumeric for $repr {
                fn from_numeric(value: &Value) -> Option<Self> {
                    match value {
                        Value::Int(value, _) => Some(*value as $repr),
                        Value::Float(value, _) => Some(*value as $repr),
                        _ => None,
                    }
                }
            }
        )+
    };
}

int_from_numeric!(i8, i16, i32, i64, u8, u16, u32, u64);

impl FromNumeric for f32 {
    fn from_numeric(value: &Value) -> Option<Self> {
        match value {
            Value::Float(value, _) => Some(*value as f32),
            Value::Int(value, _) => Some(*value as f32),
            _ => None,
        }
    }
}

impl FromNumeric for f64 {
    fn from_numeric(value: &Value) -> Option<Self> {
        match value {
            Value::Float(value, _) => Some(*value),
            Value::Int(value, _) => Some(*value as f64),
            _ => None,
        }
    }
}

macro_rules! typed_array_kind_dispatch {
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
        /// Reads one element of a view of any kind as an interpreter [`Value`],
        /// dispatching on the element kind the view carries.
        pub(crate) fn read_element(view: zun_gc::Obj, index: usize) -> Option<Value> {
            let element = zun_gc::with_current_heap(|heap| heap.typed_array_element(view))?;
            match element {
                $(zun_core::TypedElement::$variant => {
                    let raw = zun_gc::with_current_heap(|heap| heap.$get(view, index))?;
                    Some(ElementValue::into_value(raw))
                })+
            }
        }

        /// Writes one element of a view of any kind, coercing the value to the
        /// view's element representation. `None` when the index is outside the
        /// view; `Some(false)` when the store itself is rejected.
        pub(crate) fn write_element(
            view: zun_gc::Obj,
            index: usize,
            value: &Value,
        ) -> Option<bool> {
            let element = zun_gc::with_current_heap(|heap| heap.typed_array_element(view))?;
            match element {
                $(zun_core::TypedElement::$variant => {
                    let coerced = <$arg as FromNumeric>::from_numeric(value)?;
                    zun_gc::with_current_heap_mut(|heap| heap.$set(view, index, coerced))
                })+
            }
        }

        /// Allocates a fresh view of `element` kind, `length` elements long,
        /// over a new zero-filled `ArrayBuffer`.
        pub(crate) fn alloc_of(element: zun_core::TypedElement, length: usize) -> Option<zun_gc::Obj> {
            zun_gc::with_current_heap_mut(|heap| {
                let bytes = length.checked_mul(element.size())?;
                let buffer = heap.alloc_buffer(bytes);
                heap.alloc_typed_array(buffer, 0, length, element)
            })
        }
    };
}

zun_core::typed_element_table!(typed_array_kind_dispatch);

/// The element count of a typed-array view, or `None` for a non-view.
pub(crate) fn element_len(view: zun_gc::Obj) -> Option<usize> {
    zun_gc::with_current_heap(|heap| heap.typed_array_len(view))
}

/// The element kind of a typed-array view, or `None` for a non-view.
pub(crate) fn element_of(view: zun_gc::Obj) -> Option<zun_core::TypedElement> {
    zun_gc::with_current_heap(|heap| heap.typed_array_element(view))
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
                concat!($native, "Subarray"),
                concat!($native, "Fill"),
                concat!($native, "ByteOffset"),
                concat!($native, "Buffer"),
                concat!($native, "IteratorNew"),
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
            if let Some(child) = subarray(name, args) {
                return Some(child);
            }
            if let Some(meta) = view_meta(name, args) {
                return Some(meta);
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
                    concat!($native, "IteratorNew") => Some((|| {
                        let view = object(args.first()?)?;
                        // Read every element first (immutable borrow, no
                        // allocation), then build the backing array and its
                        // iterator — so the view handle is never held across
                        // an allocation that could move it.
                        let length = zun_gc::with_current_heap(|heap| heap.typed_array_len(view))?;
                        let mut elements = Vec::with_capacity(length);
                        for index in 0..length {
                            let element =
                                zun_gc::with_current_heap(|heap| heap.$get(view, index))?;
                            elements.push(ElementValue::into_value(element));
                        }
                        let iterator = zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(elements.len());
                            for (index, value) in elements.iter().enumerate() {
                                if let Some(boxed) = value.box_any(heap) {
                                    heap.array_set(array, index, boxed);
                                }
                            }
                            heap.alloc_array_iterator(array)
                        })?;
                        Some(Value::Object(iterator))
                    })()
                    .ok_or(RunError::BadToken)),
                    concat!($native, "Fill") => Some((|| {
                        let view = object(args.first()?)?;
                        let value = <$arg as ElementValue>::from_value(args.get(1)?)?;
                        let begin = index(args.get(2)).ok()?;
                        let end = index(args.get(3)).ok()?;
                        let range = zun_gc::with_current_heap(|heap| {
                            heap.typed_array_fill_range(view, begin, end)
                        })?;
                        let filled = zun_gc::with_current_heap_mut(|heap| {
                            range.map(|index| heap.$set(view, index, value) == Some(true))
                                .all(|stored| stored)
                        });
                        Some(Value::Bool(filled))
                    })()
                    .ok_or(RunError::BadToken)),
                )+
                _ => None,
            }
        }

        /// `subarray` for any kind: the element width it steps by is the
        /// view's own, so one arm serves them all.
        fn subarray(name: &str, args: &[Value]) -> Option<Result<Value, RunError>> {
            if !matches!(name, $(concat!($native, "Subarray"))|+) {
                return None;
            }
            Some((|| {
                let view = object(args.first()?)?;
                let begin = index(args.get(1)).ok()?;
                let end = index(args.get(2)).ok()?;
                let child = zun_gc::with_current_heap_mut(|heap| {
                    heap.typed_array_subarray(view, begin, end)
                })?;
                Some(Value::Object(child))
            })()
            .ok_or(RunError::BadToken))
        }

        /// `byteOffset`/`buffer` for any kind: the view carries its own byte
        /// offset and backing-buffer handle, so one arm serves every kind.
        fn view_meta(name: &str, args: &[Value]) -> Option<Result<Value, RunError>> {
            enum Want { Offset, Buffer }
            let want = match name {
                $(concat!($native, "ByteOffset"))|+ => Want::Offset,
                $(concat!($native, "Buffer"))|+ => Want::Buffer,
                _ => return None,
            };
            Some((|| {
                let view = object(args.first()?)?;
                match want {
                    Want::Offset => {
                        let offset = zun_gc::with_current_heap(|heap| {
                            heap.typed_array_byte_offset(view)
                        })?;
                        Some(Value::Int(offset as i64, NumKind::U64))
                    }
                    Want::Buffer => {
                        let buffer =
                            zun_gc::with_current_heap(|heap| heap.typed_array_buffer(view))?;
                        Some(Value::Object(buffer))
                    }
                }
            })()
            .ok_or(RunError::BadToken))
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
