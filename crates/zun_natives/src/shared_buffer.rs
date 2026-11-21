//! `zun:core` `sharedBuffer` namespace: the `SharedArrayBuffer` brand.
//!
//! Serviced on the interpreter floor: the brand lives on the managed buffer.

use crate::{NativeFn, NativeTy::*};

unavailable_stub!(unavailable, "sharedBuffer intrinsic requires an interpreter heap");

/// Appended last in [`crate::MODULES`] so every existing dispatch id stays
/// stable.
pub const MODULE: crate::Module = crate::Module {
    name: "sharedBuffer",
    functions: &[
        // `bufferMarkShared(buffer)`: brand a freshly allocated buffer as a
        // `SharedArrayBuffer` and return it. `new SharedArrayBuffer(n)` is
        // `bufferMarkShared(bufferNew(n))`.
        NativeFn {
            name: "bufferMarkShared",
            params: &[Buffer],
            ret: Buffer,
            imp: unavailable,
        },
    ],
};
