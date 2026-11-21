//! `zun:core` `classShape` namespace: shape of a class that the metadata
//! layout cannot carry on its own.
//!
//! Serviced on the interpreter floor: the answer edits the managed prototype
//! object a class owns.

use crate::{NativeFn, NativeTy::*};

unavailable_stub!(unavailable, "classShape intrinsic requires an interpreter heap");

/// Appended last in [`crate::MODULES`] so every existing dispatch id stays
/// stable.
pub const MODULE: crate::Module = crate::Module {
    name: "classShape",
    functions: &[
        // `nullPrototype(classRid)`: give the prototype object of the class
        // whose metadata TypeDef rid is `classRid` a null [[Prototype]], for
        // `class C extends null {}`. Such a class is base-less *and* its
        // instances inherit nothing -- not even `Object.prototype` -- which no
        // TypeDef row records, so the module initializer applies it.
        NativeFn {
            name: "nullPrototype",
            params: &[U32],
            ret: Void,
            imp: unavailable,
        },
        // `dynamicBase(classRid, name)`: record that the class whose metadata
        // TypeDef rid is `classRid` extends the run-time value named `name`
        // (`class C extends Symbol {}`). Such a class is compiled base-less,
        // so nothing in the metadata says whose statics it inherits.
        NativeFn {
            name: "dynamicBase",
            params: &[U32, Str],
            ret: Void,
            imp: unavailable,
        },
    ],
};
