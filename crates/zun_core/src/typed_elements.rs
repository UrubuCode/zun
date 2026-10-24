//! The one table of typed-array element kinds.
//!
//! A typed-array kind has to appear in the element enum, the heap's
//! accessors, the native declarations, the interpreter's dispatch, the
//! checker's type and method tables, and the realm's constructor lowering.
//! Writing that list seven times is how kinds end up half-wired — accepted by
//! the compiler and wrong (or absent) at run time.
//!
//! So the list lives here once and every site generates its share of the
//! wiring from it, the way the opcode set is generated from its master table.
//! Adding a kind means adding one row below and nothing else.
//!
//! The fields are named rather than positional because every consumer has to
//! repeat the whole matcher: with eighteen columns, a consumer that silently
//! swapped two of them would reintroduce exactly the class of bug this table
//! exists to remove.
//!
//! - `variant` — the [`crate::TypedElement`] variant.
//! - `repr` — how one element is stored, and what reading one yields.
//! - `arg` — what *writing* one accepts at the language boundary. Equal to
//!   `repr` except for the clamped kind, which takes a `number` as JS does.
//! - `width` — the element's size in bytes.
//! - `js` — the JavaScript constructor name.
//! - `native` — the prefix of this kind's `zun:buffer` intrinsics; the full
//!   names are `<native>New`, `<native>Length`, `<native>ByteLength`,
//!   `<native>Get` and `<native>Set`.
//! - `elem_ty` / `arg_ty` — the Zun type names of a read and of a written
//!   element, as the checker spells them.
//! - `view_ty` / `elem_native` / `arg_native` — the `zun_natives::NativeTy`
//!   variants for a view of this kind, a read element, and a written one.
//! - `encode` — converts the `arg` into the stored `repr`. [`same`] for every
//!   kind that stores what it is given.
//! - `read` / `write` / `get` / `set` — the Rust method names
//!   generated for this kind on the view and on the heap. They are spelled
//!   out rather than pasted together so every generated name is greppable
//!   from this table.
//!
//! The 64-bit kinds are named `BigInt64Array`/`BigUint64Array` in JS because
//! JS numbers cannot hold 64 bits; their elements are BigInts there. Zun has
//! native 64-bit integers, so the elements are plain `i64`/`u64` — the same
//! decision already taken for `DataView.getInt64`.
#[macro_export]
macro_rules! typed_element_table {
    ($callback:path) => {
        $callback! {
            {
                variant: I8, repr: i8, arg: i8, width: 1,
                js: "Int8Array", native: "i8Array",
                elem_ty: "i8", arg_ty: "i8",
                view_ty: I8Array, elem_native: I8, arg_native: I8,
                encode: $crate::same,
                read: read_i8, write: write_i8,
                get: i8_array_get, set: i8_array_set
            },
            {
                variant: U8, repr: u8, arg: u8, width: 1,
                js: "Uint8Array", native: "u8Array",
                elem_ty: "u8", arg_ty: "u8",
                view_ty: U8Array, elem_native: U8, arg_native: U8,
                encode: $crate::same,
                read: read_u8, write: write_u8,
                get: u8_array_get, set: u8_array_set
            },
            {
                variant: U8Clamped, repr: u8, arg: f64, width: 1,
                js: "Uint8ClampedArray", native: "u8ClampedArray",
                elem_ty: "u8", arg_ty: "number",
                view_ty: U8ClampedArray, elem_native: U8, arg_native: F64,
                encode: $crate::clamp_to_u8,
                read: read_u8_clamped, write: write_u8_clamped,
                get: u8_clamped_array_get, set: u8_clamped_array_set
            },
            {
                variant: I16, repr: i16, arg: i16, width: 2,
                js: "Int16Array", native: "i16Array",
                elem_ty: "i16", arg_ty: "i16",
                view_ty: I16Array, elem_native: I16, arg_native: I16,
                encode: $crate::same,
                read: read_i16, write: write_i16,
                get: i16_array_get, set: i16_array_set
            },
            {
                variant: U16, repr: u16, arg: u16, width: 2,
                js: "Uint16Array", native: "u16Array",
                elem_ty: "u16", arg_ty: "u16",
                view_ty: U16Array, elem_native: U16, arg_native: U16,
                encode: $crate::same,
                read: read_u16, write: write_u16,
                get: u16_array_get, set: u16_array_set
            },
            {
                variant: I32, repr: i32, arg: i32, width: 4,
                js: "Int32Array", native: "i32Array",
                elem_ty: "i32", arg_ty: "i32",
                view_ty: I32Array, elem_native: I32, arg_native: I32,
                encode: $crate::same,
                read: read_i32, write: write_i32,
                get: i32_array_get, set: i32_array_set
            },
            {
                variant: U32, repr: u32, arg: u32, width: 4,
                js: "Uint32Array", native: "u32Array",
                elem_ty: "u32", arg_ty: "u32",
                view_ty: U32Array, elem_native: U32, arg_native: U32,
                encode: $crate::same,
                read: read_u32, write: write_u32,
                get: u32_array_get, set: u32_array_set
            },
            {
                variant: F32, repr: f32, arg: f32, width: 4,
                js: "Float32Array", native: "f32Array",
                elem_ty: "f32", arg_ty: "f32",
                view_ty: F32Array, elem_native: F32, arg_native: F32,
                encode: $crate::same,
                read: read_f32, write: write_f32,
                get: f32_array_get, set: f32_array_set
            },
            {
                variant: F64, repr: f64, arg: f64, width: 8,
                js: "Float64Array", native: "f64Array",
                elem_ty: "number", arg_ty: "number",
                view_ty: F64Array, elem_native: F64, arg_native: F64,
                encode: $crate::same,
                read: read_f64, write: write_f64,
                get: f64_array_get, set: f64_array_set
            },
            {
                variant: I64, repr: i64, arg: i64, width: 8,
                js: "BigInt64Array", native: "i64Array",
                elem_ty: "i64", arg_ty: "i64",
                view_ty: I64Array, elem_native: I64, arg_native: I64,
                encode: $crate::same,
                read: read_i64, write: write_i64,
                get: i64_array_get, set: i64_array_set
            },
            {
                variant: U64, repr: u64, arg: u64, width: 8,
                js: "BigUint64Array", native: "u64Array",
                elem_ty: "u64", arg_ty: "u64",
                view_ty: U64Array, elem_native: U64, arg_native: U64,
                encode: $crate::same,
                read: read_u64, write: write_u64,
                get: u64_array_get, set: u64_array_set
            },
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
