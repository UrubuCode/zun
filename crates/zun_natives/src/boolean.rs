//! `zun:boolean` — the `Boolean` wrapper surface: JS truthiness coercion and
//! the primitive `boolean.toString()` rendering.

use crate::{NativeArg, NativeError, NativeTy::*, NativeFn, NativeValue, Sink};

/// ECMAScript truthiness of a single argument, mirroring the interpreter's
/// `is_truthy` (used for `if`/`&&` on dynamic callback results): `false`,
/// `0`/`NaN`, `""`, `null` and `undefined` are falsy; every object, function
/// and non-empty string is truthy.
///
/// The argument crosses the boundary already marshalled by its runtime value,
/// so a concrete `string`/`number` arrives as `Str`/`Int`/`Float` and is
/// classified exactly. The one gap is a *boxed* string carried inside an `Any`
/// (e.g. a value whose static type is `any`): the native sees only its string
/// handle and cannot resolve the backing text, so it is treated as truthy. No
/// consumer boxes a string purely to coerce it, and the concrete-`string` path
/// (the common case) is exact.
fn truthy(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let flag = match args.first() {
        Some(NativeArg::Int(value)) => *value != 0,
        Some(NativeArg::Float(value)) => *value != 0.0 && !value.is_nan(),
        Some(NativeArg::Bool(value)) => *value,
        Some(NativeArg::Str(value)) => !value.is_empty(),
        Some(NativeArg::Function(_)) => true,
        Some(NativeArg::Opaque) => true,
        Some(NativeArg::Any(value)) => {
            use zun_core::AnyTag;
            match value.tag() {
                AnyTag::Int => value.as_int().is_some_and(|number| number != 0),
                AnyTag::Float => value
                    .as_float()
                    .is_some_and(|number| number != 0.0 && !number.is_nan()),
                AnyTag::Bool => value.as_bool().unwrap_or(false),
                AnyTag::Null | AnyTag::Void => false,
                // Objects, symbols, class descriptors, functions and boxed
                // (unresolvable) strings are all truthy.
                AnyTag::Object | AnyTag::Symbol | AnyTag::ClassDescriptor | AnyTag::String => true,
            }
        }
        // Absent argument (`Boolean()`) and `undefined`/`null` are falsy.
        Some(NativeArg::Void) | None => false,
    };
    Ok(NativeValue::Bool(flag))
}

/// `true.toString()` / `false.toString()` → `"true"` / `"false"`.
fn to_string(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let flag = matches!(args.first(), Some(NativeArg::Bool(true)));
    Ok(NativeValue::Str(if flag { "true" } else { "false" }.to_string()))
}

/// The `Boolean` wrapper surface.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:boolean",
    functions: &[
        NativeFn {
            // Accepts any value (an `Opaque` parameter is never coerced at the
            // boundary — the real runtime value is marshalled through). Names
            // are globally unique across native modules, so this carries its
            // `boolean` identity explicitly.
            name: "booleanCoerce",
            params: &[Opaque],
            ret: Bool,
            imp: truthy,
        },
        NativeFn {
            name: "booleanToString",
            params: &[Bool],
            ret: Str,
            imp: to_string,
        },
    ],
};
