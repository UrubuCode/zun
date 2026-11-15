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
        // `globalDefineBuiltin(name, value, constant, enumerable)`: define `name` on
        // the realm global as a data property. `constant` selects the value
        // properties (`undefined`, `NaN`, `Infinity`), which are non-writable and
        // non-configurable; otherwise the property is writable and configurable.
        // `enumerable` is false for every ECMAScript and web-platform class and
        // namespace, and true for the handful of Node platform functions
        // (`structuredClone`, `atob`, ...) whose own property is enumerable.
        NativeFn {
            name: "globalDefineBuiltin",
            params: &[Str, Opaque, Bool, Bool],
            ret: Void,
            imp: unavailable,
        },
    ],
};

/// The ECMAScript namespace objects the prelude declares as classes (a class
/// holds their static members), so the bare identifier evaluates to a class
/// descriptor. `typeof` must still answer `"object"` for these: they are plain
/// objects, not constructors. `Math`, `Reflect` and `Iterator` are seeded as
/// real objects and need no entry.
pub const NAMESPACE_CLASSES: &[&str] = &["JSON", "Atomics"];
