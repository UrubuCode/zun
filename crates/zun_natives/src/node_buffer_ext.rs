//! `zun:core` `nodeBufferExt` namespace: Node `Buffer` intrinsics that postdate
//! the `nodeBuffer` group, which is no longer last in [`crate::MODULES`].
//!
//! Serviced on the interpreter floor beside the rest of the Buffer surface:
//! every Buffer is a `U8` view over a managed `ArrayBuffer`.

use crate::{NativeFn, NativeTy::*};

unavailable_stub!(unavailable, "Buffer intrinsic requires an interpreter heap");

/// Appended last in [`crate::MODULES`] so every existing dispatch id stays
/// stable.
pub const MODULE: crate::Module = crate::Module {
    name: "nodeBufferExt",
    functions: &[
        // `Buffer.concat(list, totalLength?)` with both operands validated at
        // run time: `list` must be an array of byte views, and `totalLength`,
        // when given, truncates or zero-pads the result.
        NativeFn {
            name: "nodeBufferConcatAny",
            params: &[Opaque, Opaque],
            ret: U8Array,
            imp: unavailable,
        },
    ],
};
