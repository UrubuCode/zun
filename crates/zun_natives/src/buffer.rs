use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError(
        "managed buffer intrinsic requires an interpreter heap".into(),
    ))
}

/// Byte-buffer intrinsics backed by the GC-owned contiguous buffer.
/// Generates the `zun:buffer` module. The typed-array intrinsics — five per
/// kind — come from the master table in `zun_core` rather than being written
/// out eleven times; everything else is listed here directly.
macro_rules! buffer_module {
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
    pub const MODULE: crate::Module = crate::Module {
        name: "buffer",
        functions: &[
            NativeFn {
                name: "bufferNew",
                params: &[U64],
                ret: Buffer,
                imp: unavailable,
            },
            NativeFn {
                name: "bufferLength",
                params: &[Buffer],
                ret: U64,
                imp: unavailable,
            },
            NativeFn {
                name: "bufferSlice",
                params: &[Buffer, U64, U64],
                ret: Buffer,
                imp: unavailable,
            },
            NativeFn {
                name: "bufferGetU8",
                params: &[Buffer, U64],
                ret: U8,
                imp: unavailable,
            },
            NativeFn {
                name: "bufferSetU8",
                params: &[Buffer, U64, U8],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "bufferReadU32",
                params: &[Buffer, U64],
                ret: U32,
                imp: unavailable,
            },
            NativeFn {
                name: "bufferWriteU32",
                params: &[Buffer, U64, U32],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "bufferReadF64",
                params: &[Buffer, U64],
                ret: F64,
                imp: unavailable,
            },
            NativeFn {
                name: "bufferWriteF64",
                params: &[Buffer, U64, F64],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewNew",
                params: &[Opaque, U64, U64],
                ret: DataView,
                imp: unavailable,
            },
            NativeFn { name: "dataViewLength", params: &[DataView], ret: U64, imp: unavailable },
            NativeFn { name: "dataViewByteLength", params: &[DataView], ret: U64, imp: unavailable },
            NativeFn { name: "dataViewByteOffset", params: &[DataView], ret: U64, imp: unavailable },
            NativeFn { name: "dataViewBuffer", params: &[DataView], ret: Buffer, imp: unavailable },
            // `ArrayBuffer.isView(x)` — true for a DataView or typed array,
            // false for anything else. Takes an opaque value so a non-view
            // (even a non-object) is answered rather than rejected.
            NativeFn { name: "isView", params: &[Opaque], ret: Bool, imp: unavailable },
            NativeFn {
                name: "dataViewGetU32",
                params: &[DataView, U64, Bool],
                ret: U32,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetU32",
                params: &[DataView, U64, U32, Bool],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewGetInt8",
                params: &[DataView, U64],
                ret: I8,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetInt8",
                params: &[DataView, U64, I8],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewGetUint8",
                params: &[DataView, U64],
                ret: U8,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetUint8",
                params: &[DataView, U64, U8],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewGetInt16",
                params: &[DataView, U64, Bool],
                ret: I16,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetInt16",
                params: &[DataView, U64, I16, Bool],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewGetInt32",
                params: &[DataView, U64, Bool],
                ret: I32,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetInt32",
                params: &[DataView, U64, I32, Bool],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewGetFloat32",
                params: &[DataView, U64, Bool],
                ret: F32,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetFloat32",
                params: &[DataView, U64, F32, Bool],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewGetInt64",
                params: &[DataView, U64, Bool],
                ret: I64,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetInt64",
                params: &[DataView, U64, I64, Bool],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewGetUint64",
                params: &[DataView, U64, Bool],
                ret: U64,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetUint64",
                params: &[DataView, U64, U64, Bool],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewGetU16",
                params: &[DataView, U64, Bool],
                ret: U16,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetU16",
                params: &[DataView, U64, U16, Bool],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewGetF64",
                params: &[DataView, U64, Bool],
                ret: F64,
                imp: unavailable,
            },
            NativeFn {
                name: "dataViewSetF64",
                params: &[DataView, U64, F64, Bool],
                ret: Bool,
                imp: unavailable,
            },
        $(
            NativeFn {
                name: concat!($native, "New"),
                params: &[Opaque, U64, U64],
                ret: $view_ty,
                imp: unavailable,
            },
            NativeFn {
                name: concat!($native, "Length"),
                params: &[$view_ty],
                ret: U64,
                imp: unavailable,
            },
            NativeFn {
                name: concat!($native, "ByteLength"),
                params: &[$view_ty],
                ret: U64,
                imp: unavailable,
            },
            NativeFn {
                name: concat!($native, "Get"),
                params: &[$view_ty, U64],
                ret: $elem_native,
                imp: unavailable,
            },
            NativeFn {
                name: concat!($native, "Set"),
                params: &[$view_ty, U64, $arg_native],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: concat!($native, "Subarray"),
                params: &[$view_ty, U64, U64],
                ret: $view_ty,
                imp: unavailable,
            },
            NativeFn {
                name: concat!($native, "Fill"),
                params: &[$view_ty, $arg_native, U64, U64],
                ret: Bool,
                imp: unavailable,
            },
            // `byteOffset` / `buffer`: the view's position in its backing
            // buffer and a reference to that buffer. Methods here, as the
            // rest of the surface is.
            NativeFn {
                name: concat!($native, "ByteOffset"),
                params: &[$view_ty],
                ret: U64,
                imp: unavailable,
            },
            NativeFn {
                name: concat!($native, "Buffer"),
                params: &[$view_ty],
                ret: Buffer,
                imp: unavailable,
            },
            // `[Symbol.iterator]` — builds the element sequence and hands back
            // an array iterator, so `next`/`done` are the `arrayIterator*`
            // ones, exactly as the string iterator reuses them.
            NativeFn {
                name: concat!($native, "IteratorNew"),
                params: &[Opaque],
                ret: Opaque,
                imp: unavailable,
            },
        )+
            NativeFn {
                name: "arrayIteratorNew",
                params: &[Opaque],
                ret: Opaque,
                imp: unavailable,
            },
            NativeFn {
                name: "arrayIteratorNext",
                params: &[Opaque],
                ret: Opaque,
                imp: unavailable,
            },
            NativeFn {
                name: "arrayIteratorDone",
                params: &[Opaque],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "setIteratorNew",
                params: &[Opaque],
                ret: Opaque,
                imp: unavailable,
            },
            NativeFn {
                name: "setIteratorNext",
                params: &[Opaque],
                ret: Opaque,
                imp: unavailable,
            },
            NativeFn {
                name: "setIteratorDone",
                params: &[Opaque],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn {
                name: "mapIteratorNew",
                params: &[Opaque],
                ret: Opaque,
                imp: unavailable,
            },
            NativeFn {
                name: "mapIteratorNext",
                params: &[Opaque],
                ret: Opaque,
                imp: unavailable,
            },
            NativeFn {
                name: "mapIteratorDone",
                params: &[Opaque],
                ret: Bool,
                imp: unavailable,
            },
            NativeFn { name: "generatorIteratorNew", params: &[Opaque], ret: Opaque, imp: unavailable },
            NativeFn { name: "generatorIteratorNewArgs", params: &[U32, Opaque], ret: Opaque, imp: unavailable },
            NativeFn { name: "generatorIteratorNext", params: &[Opaque], ret: Opaque, imp: unavailable },
            NativeFn { name: "generatorIteratorDone", params: &[Opaque], ret: Bool, imp: unavailable },
            // `String.prototype[Symbol.iterator]` — iterates by Unicode code point.
            // Reuses the array-iterator machinery over a code-point array, so only
            // the constructor is new; `next`/`done` are the `arrayIterator*` ones.
            NativeFn { name: "stringIteratorNew", params: &[Str], ret: Opaque, imp: unavailable },
            // The second generation of the typed-array surface: bulk `set`,
            // `slice`/`copyWithin`, the functional methods, and the `from`/`of`
            // statics. These are **kind-agnostic** — one native each rather than
            // eleven — because the element kind is a runtime property of the
            // view (read back from the heap) for an instance method, and a
            // table-order discriminant argument (`TypedElement::index`) for a
            // static that has to build a view of a kind it is told. They route
            // to the Tier-0 interpreter (callbacks re-enter it), so they are not
            // JIT-eligible, and they are appended here last so no existing
            // dispatch id shifts. The search value and reduce accumulator are
            // `Opaque` so a value of any numeric kind crosses without a
            // per-kind signature.
            NativeFn { name: "typedArraySetFrom", params: &[Opaque, Opaque, U64], ret: Bool, imp: unavailable },
            NativeFn { name: "typedArraySlice", params: &[Opaque, I64, I64], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayCopyWithin", params: &[Opaque, I64, I64, I64], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayIndexOf", params: &[Opaque, Opaque, I64], ret: I64, imp: unavailable },
            NativeFn { name: "typedArrayIncludes", params: &[Opaque, Opaque, I64], ret: Bool, imp: unavailable },
            NativeFn { name: "typedArrayJoin", params: &[Opaque, Str], ret: Str, imp: unavailable },
            NativeFn { name: "typedArrayForEach", params: &[Opaque, Function], ret: Void, imp: unavailable },
            NativeFn { name: "typedArrayMap", params: &[Opaque, Function], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayFilter", params: &[Opaque, Function], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayReduce", params: &[Opaque, Function, Opaque], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayFind", params: &[Opaque, Function], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayFindIndex", params: &[Opaque, Function], ret: I64, imp: unavailable },
            NativeFn { name: "typedArraySome", params: &[Opaque, Function], ret: Bool, imp: unavailable },
            NativeFn { name: "typedArrayEvery", params: &[Opaque, Function], ret: Bool, imp: unavailable },
            NativeFn { name: "typedArrayFrom", params: &[Opaque, U32], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayFromMapped", params: &[Opaque, Function, U32], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayOf", params: &[Opaque, U32], ret: Opaque, imp: unavailable },
            // ES2024 resizable `ArrayBuffer`. Appended last so no existing
            // dispatch id shifts. `bufferResizableNew(length, maxByteLength)`
            // marks the buffer resizable; `bufferResize` grows/shrinks it (new
            // bytes zero-filled, data preserved up to min(old,new)), returning
            // `false` when the buffer is fixed-length or `newLength` exceeds
            // the maximum. Outstanding views re-resolve their window against
            // the buffer's current length on their next access, so a resize
            // walks no views. `typedArrayTrackingNew(buffer, offset, kind)`
            // builds a length-tracking typed array (no explicit length); the
            // `kind` is the `TypedElement` table-order discriminant.
            NativeFn { name: "bufferResizableNew", params: &[U64, U64], ret: Buffer, imp: unavailable },
            NativeFn { name: "bufferResize", params: &[Buffer, U64], ret: Bool, imp: unavailable },
            NativeFn { name: "bufferMaxByteLength", params: &[Buffer], ret: U64, imp: unavailable },
            NativeFn { name: "bufferResizable", params: &[Buffer], ret: Bool, imp: unavailable },
            NativeFn { name: "typedArrayTrackingNew", params: &[Opaque, U64, U32], ret: Opaque, imp: unavailable },
            // No-initial-value `reduce`/`reduceRight` (NUCLEUS #19): seed from
            // the first/last element, throwing a TypeError-equivalent on an
            // empty typed array. `typedArrayReduceRight` is the initialized
            // right fold. Appended last so no existing dispatch id shifts.
            NativeFn { name: "typedArrayReduceNoInit", params: &[Opaque, Function], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayReduceRight", params: &[Opaque, Function, Opaque], ret: Opaque, imp: unavailable },
            NativeFn { name: "typedArrayReduceRightNoInit", params: &[Opaque, Function], ret: Opaque, imp: unavailable },
            // NUCLEUS #17: `ArrayBuffer.prototype.transfer(newLength?)` and
            // `transferToFixedLength(newLength?)` detach the source buffer and
            // move its bytes into a new buffer. `newLength` is an `I64`: a
            // negative value (`-1`) is the sentinel the surface synthesizes
            // when the caller omits it, meaning "keep the source length".
            // `bufferDetached` is the `detached` getter. Appended last so no
            // existing dispatch id shifts.
            NativeFn { name: "bufferTransfer", params: &[Buffer, I64], ret: Buffer, imp: unavailable },
            NativeFn { name: "bufferTransferFixed", params: &[Buffer, I64], ret: Buffer, imp: unavailable },
            NativeFn { name: "bufferDetached", params: &[Buffer], ret: Bool, imp: unavailable },
        ],
    };
    };
}

zun_core::typed_element_table!(buffer_module);
