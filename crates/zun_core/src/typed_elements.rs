//! The one table of typed-array element kinds.
//!
//! A typed-array kind has to appear in the element enum, the heap's allocator
//! and accessors, the native declarations, the interpreter's dispatch, the
//! checker's type and method tables, and the realm's constructor lowering.
//! Writing that list seven times is how kinds end up half-wired — accepted by
//! the compiler and wrong (or absent) at run time.
//!
//! So the list lives here once and every site generates its share of the
//! wiring from it, the way the opcode set is generated from its master table.
//! Adding a kind means adding one row below and nothing else.
//!
//! Each row is, in order:
//!
//! 1. `variant` — the [`crate::TypedElement`] variant.
//! 2. `repr` — how one element is stored, and what reading one yields.
//! 3. `arg` — what *writing* one accepts at the language boundary. Equal to
//!    `repr` except for the clamped kind, which takes a `number` as JS does.
//! 4. `width` — the element's size in bytes.
//! 5. `js` — the JavaScript constructor name.
//! 6. `native` — the prefix of this kind's `zun:buffer` intrinsics; the full
//!    names are `<native>New`, `<native>Length`, `<native>ByteLength`,
//!    `<native>Get` and `<native>Set`.
//! 7. `elem_ty` — the Zun type name of a *read* element, as the checker
//!    spells it.
//! 8. `arg_ty` — the Zun type name a *written* element is accepted as.
//! 9. `native_ty` — the `zun_natives::NativeTy` variant naming a view of
//!    this kind.
//! 10. `encode` — converts the `arg` into the stored `repr`. [`same`] for
//!     every kind that stores what it is given.
//! 11-15. The Rust method names generated for this kind: the view's reader
//!     and writer, and the heap's allocator, getter and setter. They are
//!     spelled out rather than pasted together so that every generated name
//!     is greppable from this table.
//!
//! The 64-bit kinds are named `BigInt64Array`/`BigUint64Array` in JS because
//! JS numbers cannot hold 64 bits; their elements are BigInts there. Zun has
//! native 64-bit integers, so the elements are plain `i64`/`u64` — the same
//! decision already taken for `DataView.getInt64`.
#[macro_export]
macro_rules! typed_element_table {
    ($callback:path) => {
        $callback! {
            (I8, i8, i8, 1, "Int8Array", "i8Array", "i8", "i8", I8Array, $crate::same,
             read_i8, write_i8, alloc_i8_array, i8_array_get, i8_array_set),
            (U8, u8, u8, 1, "Uint8Array", "u8Array", "u8", "u8", U8Array, $crate::same,
             read_u8, write_u8, alloc_u8_array, u8_array_get, u8_array_set),
            (U8Clamped, u8, f64, 1, "Uint8ClampedArray", "u8ClampedArray", "u8", "number",
             U8ClampedArray, $crate::clamp_to_u8,
             read_u8_clamped, write_u8_clamped, alloc_u8_clamped_array,
             u8_clamped_array_get, u8_clamped_array_set),
            (I16, i16, i16, 2, "Int16Array", "i16Array", "i16", "i16", I16Array, $crate::same,
             read_i16, write_i16, alloc_i16_array, i16_array_get, i16_array_set),
            (U16, u16, u16, 2, "Uint16Array", "u16Array", "u16", "u16", U16Array, $crate::same,
             read_u16, write_u16, alloc_u16_array, u16_array_get, u16_array_set),
            (I32, i32, i32, 4, "Int32Array", "i32Array", "i32", "i32", I32Array, $crate::same,
             read_i32, write_i32, alloc_i32_array, i32_array_get, i32_array_set),
            (U32, u32, u32, 4, "Uint32Array", "u32Array", "u32", "u32", U32Array, $crate::same,
             read_u32, write_u32, alloc_u32_array, u32_array_get, u32_array_set),
            (F32, f32, f32, 4, "Float32Array", "f32Array", "f32", "f32", F32Array, $crate::same,
             read_f32, write_f32, alloc_f32_array, f32_array_get, f32_array_set),
            (F64, f64, f64, 8, "Float64Array", "f64Array", "number", "number", F64Array,
             $crate::same,
             read_f64, write_f64, alloc_f64_array, f64_array_get, f64_array_set),
            (I64, i64, i64, 8, "BigInt64Array", "i64Array", "i64", "i64", I64Array, $crate::same,
             read_i64, write_i64, alloc_i64_array, i64_array_get, i64_array_set),
            (U64, u64, u64, 8, "BigUint64Array", "u64Array", "u64", "u64", U64Array, $crate::same,
             read_u64, write_u64, alloc_u64_array, u64_array_get, u64_array_set),
        }
    };
}

/// The identity conversion, for every kind that stores exactly what it is
/// given. Named so the table reads as a table rather than as an exception.
pub const fn same<T: Copy>(value: T) -> T {
    value
}

/// `Uint8ClampedArray`'s conversion: clamp into `0..=255` instead of wrapping,
/// round halves to even, and send NaN to zero — ECMAScript's `ToUint8Clamp`.
/// `Uint8Array` stores `300` as `44`; this stores `255`.
pub fn clamp_to_u8(value: f64) -> u8 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if value >= 255.0 {
        return 255;
    }
    // Ties to even is exactly ToUint8Clamp's rule: 0.5 → 0, 1.5 → 2.
    value.round_ties_even() as u8
}
