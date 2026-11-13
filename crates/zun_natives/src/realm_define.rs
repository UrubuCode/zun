//! `zun:core` `realm` namespace — definition of the built-in properties of the
//! realm global object.
//!
//! `globalSet` registers a binding the program may later refuse to overwrite;
//! an ECMAScript built-in global is a different shape (a writable, configurable,
//! non-enumerable data property, or a fully frozen constant such as
//! `undefined`). The module initializer seeds the realm through this native so
//! `Object.getOwnPropertyDescriptor(globalThis, name)` reports the specified
//! attributes. Serviced on the interpreter floor.

use crate::{NativeFn, NativeTy::*};

unavailable_stub!(unavailable, "realm intrinsic requires an interpreter heap");

/// Appended last in [`crate::MODULES`] so every existing dispatch id stays
/// stable.
pub const MODULE: crate::Module = crate::Module {
    name: "realm",
    functions: &[
        // `globalDefineBuiltin(name, value, constant)`: define `name` on the realm
        // global as a non-enumerable data property. `constant` selects the value
        // properties (`undefined`, `NaN`, `Infinity`), which are non-writable and
        // non-configurable; otherwise the property is writable and configurable.
        NativeFn {
            name: "globalDefineBuiltin",
            params: &[Str, Opaque, Bool],
            ret: Void,
            imp: unavailable,
        },
    ],
};
