//! `zun:core` `brand` namespace: the private-name brand check (`#x in obj`).
//!
//! Serviced on the interpreter floor: the answer reads the operand's run-time
//! type from the managed heap.

use crate::{NativeFn, NativeTy::*};

unavailable_stub!(unavailable, "brand intrinsic requires an interpreter heap");

/// Appended last in [`crate::MODULES`] so every existing dispatch id stays
/// stable.
pub const MODULE: crate::Module = crate::Module {
    name: "brand",
    functions: &[
        // `hasPrivateBrand(value, classRid, name)`: `#x in value` for an instance
        // private declared by the class whose metadata TypeDef rid is
        // `classRid`. True only for a typed instance whose run-time type is
        // that class or derives from it. Unlike `instanceofClass` it never
        // consults the prototype chain: the brand survives
        // `Object.setPrototypeOf`, and `Object.create(C.prototype)` has none.
        // A primitive operand is a TypeError naming the private (`name`).
        NativeFn {
            name: "hasPrivateBrand",
            params: &[Opaque, U32, Str],
            ret: Bool,
            imp: unavailable,
        },
    ],
};
