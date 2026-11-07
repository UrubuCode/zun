//! The single authoritative fixed-arity managed-method descriptor table.
//!
//! The typed path in `zun_sema` (`managed_method_descriptor`) routes the simple,
//! fixed-arity methods of the managed reference types — WeakMap / WeakSet /
//! WeakRef / RegExp / BigInt / the Promise accessors / the fixed-arity String
//! methods / DataView / ArrayBuffer — to natives. Each such method is pure data:
//! a `(receiver, member) -> (native, return type, argument types)` mapping with
//! no argument-shape branching and no side effects. This table names that
//! mapping exactly once, mirroring the `Array` and `Map`/`Set` tables in
//! `array_methods` / `collection_methods`.
//!
//! It only names the mapping; it does not change the `.zm` format, the object /
//! string / buffer layout, or any opcode. The genuinely conditional handlers
//! (array `sort`/`splice`/…, number `toFixed`/…, string `normalize`/`replace`,
//! buffer `transfer`, and the open-set primitive-tail fallback) stay as explicit
//! interceptors on the sema side and are deliberately *not* here.
//!
//! Types are named with the neutral [`ManagedTy`] tag so this crate stays free
//! of any `zun_sema` dependency; the consumer maps each tag back to its own
//! `Ty`. The same tag doubles as the receiver discriminant a lookup keys on.

/// A neutral type tag used both as the receiver discriminant a row is keyed on
/// and as the row's return / argument types. The consumer maps each tag to its
/// own checker type; the mapping is one-to-one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedTy {
    /// The dynamic `any` value.
    Any,
    /// `boolean`.
    Bool,
    /// `string`.
    Str,
    /// 64-bit float (`number`).
    F64,
    /// 32-bit float.
    F32,
    /// Signed 8-bit integer.
    I8,
    /// Signed 16-bit integer.
    I16,
    /// Signed 32-bit integer (`int`).
    I32,
    /// Signed 64-bit integer.
    I64,
    /// Unsigned 8-bit integer.
    U8,
    /// Unsigned 16-bit integer.
    U16,
    /// Unsigned 32-bit integer.
    U32,
    /// Unsigned 64-bit integer.
    U64,
    /// A managed contiguous array.
    Array,
    /// A GC-owned contiguous byte buffer (`ArrayBuffer`).
    Buffer,
    /// A GC ephemeron-backed weak map.
    WeakMap,
    /// A GC ephemeron-backed weak set.
    WeakSet,
    /// A GC-backed single weak reference.
    WeakRef,
    /// A compiled regular expression.
    RegExp,
    /// An arbitrary-precision integer.
    BigInt,
    /// A managed Promise object.
    Promise,
    /// A mixed-width `DataView` over a buffer.
    DataView,
}

/// One row of the authoritative fixed-arity managed-method table.
#[derive(Debug, Clone, Copy)]
pub struct ManagedMethod {
    /// The receiver type the method is called on.
    pub receiver: ManagedTy,
    /// The member name.
    pub member: &'static str,
    /// The native this method routes to.
    pub native: &'static str,
    /// The method's return type.
    pub ret: ManagedTy,
    /// The method's expected argument types, in order.
    pub args: &'static [ManagedTy],
}

use ManagedTy::*;

const fn row(
    receiver: ManagedTy,
    member: &'static str,
    native: &'static str,
    ret: ManagedTy,
    args: &'static [ManagedTy],
) -> ManagedMethod {
    ManagedMethod {
        receiver,
        member,
        native,
        ret,
        args,
    }
}

/// The authoritative table, in the exact order of the sema-side match it
/// replaces so first-match behavior for any overlapping `(receiver, member)`
/// key is preserved.
static MANAGED_METHODS: &[ManagedMethod] = &[
    row(WeakMap, "set", "weakMapSet", WeakMap, &[Any, Any]),
    row(WeakMap, "get", "weakMapGet", Any, &[Any]),
    row(WeakMap, "has", "weakMapHas", Bool, &[Any]),
    row(WeakMap, "delete", "weakMapDelete", Bool, &[Any]),
    row(WeakSet, "add", "weakSetAdd", WeakSet, &[Any]),
    row(WeakSet, "has", "weakSetHas", Bool, &[Any]),
    row(WeakSet, "delete", "weakSetDelete", Bool, &[Any]),
    row(WeakRef, "deref", "weakRefDeref", Any, &[]),
    row(RegExp, "isMatch", "regexpIsMatch", Bool, &[Str]),
    row(RegExp, "test", "regexpTest", Bool, &[Str]),
    row(RegExp, "exec", "regexpExec", Array, &[Str]),
    row(RegExp, "source", "regexpSource", Str, &[]),
    row(BigInt, "add", "bigIntAdd", BigInt, &[BigInt]),
    row(BigInt, "sub", "bigIntSub", BigInt, &[BigInt]),
    row(BigInt, "mul", "bigIntMul", BigInt, &[BigInt]),
    row(BigInt, "div", "bigIntDiv", BigInt, &[BigInt]),
    row(BigInt, "rem", "bigIntRem", BigInt, &[BigInt]),
    row(BigInt, "pow", "bigIntPow", BigInt, &[BigInt]),
    row(BigInt, "neg", "bigIntNeg", BigInt, &[]),
    row(BigInt, "and", "bigIntAnd", BigInt, &[BigInt]),
    row(BigInt, "or", "bigIntOr", BigInt, &[BigInt]),
    row(BigInt, "xor", "bigIntXor", BigInt, &[BigInt]),
    row(BigInt, "not", "bigIntNot", BigInt, &[]),
    row(BigInt, "shl", "bigIntShl", BigInt, &[BigInt]),
    row(BigInt, "shr", "bigIntShr", BigInt, &[BigInt]),
    row(BigInt, "compare", "bigIntCompare", I32, &[BigInt]),
    row(BigInt, "toString", "bigIntToString", Str, &[]),
    row(Promise, "state", "promiseState", U8, &[]),
    row(Promise, "result", "promiseResult", Any, &[]),
    row(Promise, "isReady", "promiseIsReady", Bool, &[]),
    row(Promise, "resolve", "promiseResolve", Bool, &[Any]),
    row(Promise, "reject", "promiseReject", Bool, &[Str]),
    row(Str, "toInt", "toInt", I32, &[]),
    row(Str, "toFloat", "toFloat", F64, &[]),
    row(Str, "toBoolean", "toBoolean", Bool, &[]),
    row(Str, "reverse", "reverse", Str, &[]),
    row(Str, "length", "length", U64, &[]),
    row(Str, "toUpperCase", "toUpperCase", Str, &[]),
    row(Str, "toLowerCase", "toLowerCase", Str, &[]),
    row(Str, "trim", "trim", Str, &[]),
    row(Str, "trimStart", "trimStart", Str, &[]),
    row(Str, "trimEnd", "trimEnd", Str, &[]),
    row(Str, "charCodeAt", "charCodeAt", U32, &[U64]),
    row(Str, "indexOf", "indexOf", I32, &[Str]),
    row(Str, "lastIndexOf", "lastIndexOf", I32, &[Str]),
    row(Str, "substring", "substring", Str, &[U64, U64]),
    row(Str, "repeat", "repeat", Str, &[U64]),
    row(Str, "startsWith", "startsWith", Bool, &[Str]),
    row(Str, "endsWith", "endsWith", Bool, &[Str]),
    // ECMA-402 locale surface (NUCLEUS #1), ICU4X-backed. `localeCompare`
    // returns the ECMAScript -1/0/1; v1 uses root collation and the root
    // (`und`) locale for casing.
    row(Str, "includes", "includes", Bool, &[Str]),
    // Index is a `number` so a negative value (from the end) can cross the
    // boundary; out of range yields `undefined`.
    row(Str, "at", "at", Str, &[F64]),
    row(Str, "codePointAt", "codePointAt", U32, &[U64]),
    row(Str, "replaceAll", "replaceAll", Str, &[Str, Str]),
    row(DataView, "getUint32", "dataViewGetU32", U32, &[U64, Bool]),
    row(DataView, "setUint32", "dataViewSetU32", Bool, &[U64, U32, Bool]),
    row(DataView, "getUint16", "dataViewGetU16", U16, &[U64, Bool]),
    row(DataView, "setUint16", "dataViewSetU16", Bool, &[U64, U16, Bool]),
    row(DataView, "getFloat64", "dataViewGetF64", F64, &[U64, Bool]),
    row(DataView, "setFloat64", "dataViewSetF64", Bool, &[U64, F64, Bool]),
    // The remaining DataView element widths. JS names 64-bit access
    // `getBigInt64`/`getBigUint64` and returns a BigInt; Zun has native 64-bit
    // integers, so these return `i64`/`u64` and are named without the Big.
    row(DataView, "getInt8", "dataViewGetInt8", I8, &[U64]),
    row(DataView, "setInt8", "dataViewSetInt8", Bool, &[U64, I8]),
    row(DataView, "getUint8", "dataViewGetUint8", U8, &[U64]),
    row(DataView, "setUint8", "dataViewSetUint8", Bool, &[U64, U8]),
    row(DataView, "getInt16", "dataViewGetInt16", I16, &[U64, Bool]),
    row(DataView, "setInt16", "dataViewSetInt16", Bool, &[U64, I16, Bool]),
    row(DataView, "getInt32", "dataViewGetInt32", I32, &[U64, Bool]),
    row(DataView, "setInt32", "dataViewSetInt32", Bool, &[U64, I32, Bool]),
    row(DataView, "getFloat32", "dataViewGetFloat32", F32, &[U64, Bool]),
    row(DataView, "setFloat32", "dataViewSetFloat32", Bool, &[U64, F32, Bool]),
    row(DataView, "getInt64", "dataViewGetInt64", I64, &[U64, Bool]),
    row(DataView, "setInt64", "dataViewSetInt64", Bool, &[U64, I64, Bool]),
    row(DataView, "getUint64", "dataViewGetUint64", U64, &[U64, Bool]),
    row(DataView, "setUint64", "dataViewSetUint64", Bool, &[U64, U64, Bool]),
    row(DataView, "byteLength", "dataViewByteLength", U64, &[]),
    // `byteOffset`/`buffer`: the view's position in its backing buffer and a
    // reference to that `ArrayBuffer`.
    row(DataView, "byteOffset", "dataViewByteOffset", U64, &[]),
    row(DataView, "buffer", "dataViewBuffer", Buffer, &[]),
    // `ArrayBuffer`, which is what the buffer receiver denotes. These are
    // methods rather than the properties JS has, matching `String.length()`
    // and `Map.size()`; the divergence is recorded in NUCLEUS rather than
    // special-cased.
    row(Buffer, "byteLength", "bufferLength", U64, &[]),
    row(Buffer, "slice", "bufferSlice", Buffer, &[U64, U64]),
    // ES2024 resizable ArrayBuffer. As with `byteLength`, these JS properties
    // are methods here (the divergence is in NUCLEUS). `resize` returns a
    // boolean rather than JS's `undefined`; a rejected resize is a runtime
    // error.
    row(Buffer, "maxByteLength", "bufferMaxByteLength", U64, &[]),
    row(Buffer, "resizable", "bufferResizable", Bool, &[]),
    row(Buffer, "detached", "bufferDetached", Bool, &[]),
    row(Buffer, "resize", "bufferResize", Bool, &[U64]),
];

/// Look up the authoritative table by receiver tag and member name. Returns the
/// first matching row (preserving the match order of the sema code it replaces),
/// or `None` for a `(receiver, member)` the table does not name.
pub fn managed_method(receiver: ManagedTy, member: &str) -> Option<&'static ManagedMethod> {
    MANAGED_METHODS
        .iter()
        .find(|row| row.receiver == receiver && row.member == member)
}
