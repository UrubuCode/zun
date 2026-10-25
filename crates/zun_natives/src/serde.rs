//! `serialize`/`deserialize` — the binary ser/de PRIMITIVE (Erlang
//! `term_to_binary`/`binary_to_term`-style), imported from `"zun:core"`:
//!
//! ```ts
//! import { serialize, deserialize } from "zun:core";
//! const bytes = serialize(value);   // -> Uint8Array
//! const value = deserialize(bytes); // Uint8Array -> reconstructed value
//! ```
//!
//! Both walk and allocate on the managed heap (the memo table holds live
//! object references during the walk), so they are serviced on the
//! interpreter's Tier-0 floor (`zun_interp::collections`), not here; the
//! implementations below are the heap-free fallback used only if ever
//! dispatched off that floor.
//!
//! The wire format is defined once in `zun_core::serde`.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError(
        "serialize/deserialize require an interpreter heap".into(),
    ))
}

/// The ser/de primitive. Appended LAST in the module registry so every
/// existing dispatch id stays stable.
pub const MODULE: crate::Module = crate::Module {
    name: "serde",
    functions: &[
        NativeFn {
            // `serialize(value)` -> `Uint8Array`. The operand arrives as a raw
            // managed value (`Opaque`): the interpreter inspects it directly.
            // A function value is not data and is rejected with a clean error.
            name: "serialize",
            params: &[Opaque],
            ret: U8Array,
            imp: unavailable,
        },
        NativeFn {
            // `deserialize(bytes)` -> reconstructed value (`any`). The input is
            // a `Uint8Array` byte view; an unknown tag, a truncated stream, or
            // an unresolved type token is a clean error.
            name: "deserialize",
            params: &[U8Array],
            ret: Opaque,
            imp: unavailable,
        },
    ],
};
