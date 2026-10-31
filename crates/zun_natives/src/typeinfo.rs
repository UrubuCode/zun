//! `zun:typeinfo` — runtime type reflection for dynamically-typed (`any`)
//! values: the `typeof` operator and `instanceof` test when the operand's
//! static type does not already decide the answer.
//!
//! Both functions need the runtime heap to classify a managed reference (a
//! BigInt payload versus an ordinary object, a class's hierarchy), so the
//! interpreter intercepts them on its managed floor (see
//! `zun_interp::collections`) exactly like the collection intrinsics. The
//! implementations here are the heap-free fallback used only if the functions
//! are ever dispatched outside that floor; they classify everything they can
//! from the neutral value alone and treat every managed reference as `object`.

use crate::{NativeArg, NativeError, NativeTy::*, NativeFn, NativeValue, Sink};

/// Heap-free `typeof` classification (see the module note). A managed object
/// reference cannot be refined to `bigint` here, so it reports `object`.
fn typeof_any(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    use zun_core::AnyTag;
    let name = match args.first() {
        Some(NativeArg::Int(_)) | Some(NativeArg::Float(_)) => "number",
        Some(NativeArg::Bool(_)) => "boolean",
        Some(NativeArg::Str(_)) => "string",
        Some(NativeArg::Function(_)) => "function",
        Some(NativeArg::Void) | None => "undefined",
        Some(NativeArg::Opaque) => "object",
        Some(NativeArg::Any(value)) => match value.tag() {
            AnyTag::Int | AnyTag::Float => "number",
            AnyTag::Bool => "boolean",
            AnyTag::String => "string",
            AnyTag::Symbol => "symbol",
            AnyTag::ClassDescriptor => "function",
            AnyTag::Void => "undefined",
            // `typeof null === "object"`.
            AnyTag::Null => "object",
            AnyTag::Object => "object",
        },
    };
    Ok(NativeValue::Str(name.to_string()))
}

/// Heap-free `Object.prototype.toString.call` fallback (managed floor refines it).
fn object_to_string_tag(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Str("[object Object]".to_string()))
}

/// Heap-free `instanceof` fallback: without the class hierarchy it cannot
/// decide membership, so it answers `false` (never a false positive).
fn instanceof_class(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(false))
}

/// Runtime type reflection for `any`/dynamic operands.
pub const MODULE: crate::Module = crate::Module {
    name: "typeinfo",
    functions: &[
        NativeFn {
            // `typeof value` when `value`'s static type is `any`. Returns the
            // JS type string: number/string/boolean/bigint/symbol/function/
            // undefined/object (with `typeof null === "object"`).
            name: "typeofAny",
            params: &[Opaque],
            ret: Str,
            imp: typeof_any,
        },
        NativeFn {
            // `Object.prototype.toString.call(value)`.
            name: "objectToStringTag",
            params: &[Opaque],
            ret: Str,
            imp: object_to_string_tag,
        },
        NativeFn {
            // `value instanceof C` when the static type cannot fold the answer.
            // The second argument is C's metadata TypeDef rid; the managed
            // floor walks `value`'s runtime class hierarchy against it.
            name: "instanceofClass",
            params: &[Opaque, U32],
            ret: Bool,
            imp: instanceof_class,
        },
        NativeFn {
            // `value instanceof C` where `C` is a dynamic object carrying a
            // `[Symbol.hasInstance]` method. The managed floor looks the method
            // up in `C`'s symbol-function slot and returns `ToBoolean` of its
            // result; absent the method it answers `false` (a dynamic object
            // has no class identity). The second argument is the right-hand
            // object itself (not a metadata rid), hence `Opaque`.
            name: "instanceofDynamic",
            params: &[Opaque, Opaque],
            ret: Bool,
            imp: instanceof_class,
        },
    ],
};
