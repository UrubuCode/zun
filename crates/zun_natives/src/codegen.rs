//! `zun:core` `codegen` namespace: runtime code generation from strings that
//! postdates the frozen `collections` group (which holds `functionConstruct`).
//!
//! Serviced on the interpreter floor: it needs the injected `EvalHost` and
//! re-enters the interpreter to run the compiled source. With no host (AOT,
//! compiler-free runtime) it throws the same "code generation from strings
//! disallowed" error as `new Function`.

use crate::{NativeFn, NativeTy::*};

unavailable_stub!(unavailable, "codegen intrinsic requires an interpreter heap");

/// Appended last in [`crate::MODULES`] so every existing dispatch id stays
/// stable.
pub const MODULE: crate::Module = crate::Module {
    name: "codegen",
    functions: &[
        // `indirectEval(source)`: the body of the `eval` function *value*
        // (`(0, eval)(src)`, `const e = eval; e(src)`, `globalThis.eval(src)`).
        // A non-string argument is returned unchanged; a string is compiled as
        // global code (no access to the caller's scope, `this` is the global
        // object) and its completion value is returned.
        NativeFn {
            name: "indirectEval",
            params: &[Opaque],
            ret: Opaque,
            imp: unavailable,
        },
    ],
};
