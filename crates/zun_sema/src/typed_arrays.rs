//! The checker's share of the typed-array wiring, generated from the master
//! table in `zun_core`.
//!
//! Three things used to be written out per kind here — the constructor name
//! the realm accepts, the type a `Uint8Array` annotation denotes, and the
//! `length`/`byteLength`/`get`/`set` signatures — and a kind that reached only
//! two of the three was accepted by the compiler and broken at run time.
//! Generating all three from one row removes that gap.

use crate::hir::{BufferViewKind, IntKind, Ty};
use crate::realm::CtorLowering;

/// The `Ty` a Zun scalar type name denotes, for the element types named in
/// the master table. Only the names that table uses are accepted; anything
/// else is a table that has grown a type this does not know, which is a bug
/// rather than a user error.
fn scalar_ty(name: &str) -> Ty {
    match name {
        "i8" => Ty::Int(IntKind::I8),
        "i16" => Ty::Int(IntKind::I16),
        "i32" => Ty::Int(IntKind::I32),
        "i64" => Ty::Int(IntKind::I64),
        "u8" => Ty::Int(IntKind::U8),
        "u16" => Ty::Int(IntKind::U16),
        "u32" => Ty::Int(IntKind::U32),
        "u64" => Ty::Int(IntKind::U64),
        "f32" => Ty::F32,
        "number" => Ty::F64,
        other => unreachable!("the element table names an unknown scalar type `{other}`"),
    }
}

macro_rules! typed_array_surface {
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
        /// The view kind a JavaScript typed-array constructor name denotes,
        /// or `None` for a name that is not one.
        pub(crate) fn view_kind(name: &str) -> Option<BufferViewKind> {
            match name {
                $($js => Some(BufferViewKind::$variant),)+
                _ => None,
            }
        }

        /// The JavaScript constructor name a view kind came from. `DataView`
        /// is not in the table and answers `None`.
        pub fn js_name(kind: BufferViewKind) -> Option<&'static str> {
            match kind {
                $(BufferViewKind::$variant => Some($js),)+
                BufferViewKind::DataView => None,
            }
        }

        /// How `new <Name>(buffer, offset, length)` lowers.
        pub(crate) fn constructor(name: &str) -> Option<(Ty, CtorLowering)> {
            let kind = view_kind(name)?;
            let native = match kind {
                $(BufferViewKind::$variant => concat!($native, "New"),)+
                BufferViewKind::DataView => return None,
            };
            Some((Ty::BufferView(kind), CtorLowering::BufferView { native, kind }))
        }

        /// The `NativeTy` for a view of each kind, as the intrinsic
        /// signatures spell it.
        pub(crate) fn view_native_ty(native: zun_natives::NativeTy) -> Option<Ty> {
            use zun_natives::NativeTy as N;
            match native {
                $(N::$view_ty => Some(Ty::BufferView(BufferViewKind::$variant)),)+
                _ => None,
            }
        }

        /// The signature of a method on a typed-array view: the intrinsic it
        /// calls, what it returns, and what it takes.
        ///
        /// `get` yields the element type and `set` takes it, except for the
        /// clamped kind, which takes a `number` and clamps it — JS accepts
        /// out-of-range numbers there rather than rejecting them.
        pub(crate) fn method(
            kind: BufferViewKind,
            name: &str,
        ) -> Option<(&'static str, Ty, Vec<Ty>)> {
            let index = Ty::Int(IntKind::U64);
            match (kind, name) {
                $(
                    (BufferViewKind::$variant, "length") => {
                        Some((concat!($native, "Length"), index, vec![]))
                    }
                    (BufferViewKind::$variant, "byteLength") => {
                        Some((concat!($native, "ByteLength"), index, vec![]))
                    }
                    (BufferViewKind::$variant, "get") => Some((
                        concat!($native, "Get"),
                        scalar_ty($elem_ty),
                        vec![index],
                    )),
                    (BufferViewKind::$variant, "set") => Some((
                        concat!($native, "Set"),
                        Ty::Boolean,
                        vec![index, scalar_ty($arg_ty)],
                    )),
                    // `subarray` yields another view of the same kind over
                    // the same bytes; `fill` writes one value across a range.
                    // Both clamp their element bounds the way `slice` does.
                    (BufferViewKind::$variant, "subarray") => Some((
                        concat!($native, "Subarray"),
                        Ty::BufferView(BufferViewKind::$variant),
                        vec![index, index],
                    )),
                    (BufferViewKind::$variant, "fill") => Some((
                        concat!($native, "Fill"),
                        Ty::Boolean,
                        vec![scalar_ty($arg_ty), index, index],
                    )),
                    // `byteOffset`/`buffer`: the view's position in its
                    // backing buffer and a reference to that buffer. Methods
                    // here, as the rest of the surface is.
                    (BufferViewKind::$variant, "byteOffset") => {
                        Some((concat!($native, "ByteOffset"), index, vec![]))
                    }
                    (BufferViewKind::$variant, "buffer") => {
                        Some((concat!($native, "Buffer"), Ty::Buffer, vec![]))
                    }
                )+
                _ => None,
            }
        }

        /// The `[Symbol.iterator]` constructor intrinsic for a typed-array
        /// kind, or `None` for `DataView`, which is not iterable. `next`/`done`
        /// are the `arrayIterator*` intrinsics, as for strings.
        pub(crate) fn iterator_new(kind: BufferViewKind) -> Option<&'static str> {
            match kind {
                $(BufferViewKind::$variant => Some(concat!($native, "IteratorNew")),)+
                BufferViewKind::DataView => None,
            }
        }

        /// The table-order discriminant of a view kind, as
        /// [`zun_core::TypedElement::index`] numbers it. `from`/`of` carry
        /// their target kind across the IL boundary as this integer, so the
        /// interpreter can build a view of a kind it is told rather than one
        /// it reads back from an existing view. Paired by shared variant name,
        /// so the two enums cannot drift. `DataView` has no element kind.
        pub(crate) fn element_index(kind: BufferViewKind) -> Option<u32> {
            match kind {
                $(BufferViewKind::$variant => Some(zun_core::TypedElement::$variant.index()),)+
                BufferViewKind::DataView => None,
            }
        }

        /// A view kind's element size in bytes (`Uint32Array` → 4). `DataView`
        /// has no element kind. Used to turn `new T(n)` (n elements) into a
        /// fresh buffer of `n * size` bytes.
        pub(crate) fn element_size(kind: BufferViewKind) -> Option<usize> {
            match kind {
                $(BufferViewKind::$variant => Some(zun_core::TypedElement::$variant.size()),)+
                BufferViewKind::DataView => None,
            }
        }

        /// Every kind's JavaScript constructor name, in table order — for
        /// the guards that must not fall behind the table.
        pub const CONSTRUCTOR_NAMES: &[&str] = &[$($js),+];

        /// The prefix of every typed-array intrinsic, so a native name can
        /// be routed to `zun:buffer` without a second list of kinds.
        pub(crate) const NATIVE_PREFIXES: &[&str] = &[$($native),+];
    };
}

zun_core::typed_element_table!(typed_array_surface);

