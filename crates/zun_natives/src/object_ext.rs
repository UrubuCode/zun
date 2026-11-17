//! `zun:core` `objectExt` namespace: Object intrinsics that postdate the frozen
//! `collections` group.
//!
//! Serviced on the interpreter floor: they read the managed heap and can
//! re-enter the interpreter to run a script-installed method.

use crate::{NativeFn, NativeTy::*};

unavailable_stub!(unavailable, "object intrinsic requires an interpreter heap");

/// Appended last in [`crate::MODULES`] so every existing dispatch id stays
/// stable.
pub const MODULE: crate::Module = crate::Module {
    name: "objectExt",
    functions: &[
        // `objectProtoCall(receiver, method, argument)`: the call
        // `receiver.method(argument)` for one of the `Object.prototype`
        // reflectors (`hasOwnProperty`, `propertyIsEnumerable`,
        // `isPrototypeOf`) on a dynamic receiver. A function the script
        // installed under that name on the receiver, or on its prototype
        // chain, takes the call; otherwise the built-in answers. The result
        // is `any` because an override may return anything.
        NativeFn {
            name: "objectProtoCall",
            params: &[Opaque, Str, Opaque],
            ret: Opaque,
            imp: unavailable,
        },
    ],
};
