//! The Zun prelude.
//!
//! A set of numeric/math helpers written in Zun that are auto-included in every
//! program (the analogue of Rust's `std::prelude`). Until Zun-source module
//! imports exist, [`with_prelude`] concatenates it *after* the user's source so
//! user spans (which start at offset 0) stay correct for diagnostics, while the
//! prelude's own functions sit at the end of the combined text. The prelude is
//! pure numeric code, so its functions are JIT-eligible like any other. When
//! code-module imports land, this becomes the importable `zun:math` module.
//!
//! This is distinct from the **intrinsics** (the `zun:core` native surface:
//! the `io`, `time`, `host`, ... groups), which are runtime-implemented in Rust
//! (see `zun_natives`).

/// The numeric helper portion of the prelude.
pub const PRELUDE: &str = include_str!("prelude.zun");

/// The managed error hierarchy (`builtin/error.zun`), also compiled with every
/// program.
///
/// `Error` and its subclasses are ordinary Zun classes with no dedicated `Ty`,
/// unlike `Array` or `Map`, whose names the checker resolves to a built-in
/// representation. A plain class only exists if some source declares it, so
/// until the precompiled `zun:std` module of PLAN §3.2 can carry classes, the
/// prelude is how they reach a program at all. Before this, `builtin/error.zun`
/// was compiled only inside `compile_standard_library`, which no user program
/// can reach, so `new Error("x")` was an unknown-class error and programs that
/// needed one declared their own.
///
/// Kept separate from [`PRELUDE`] so the numeric fingerprint above keeps its
/// meaning, and so the two can be split again once the std module can supply
/// classes.
pub const ERRORS: &str = include_str!("../builtin/error.zun");

/// The `Date` calendar class (`builtin/date.zun`), also compiled with every
/// program, and for the same reason as [`ERRORS`]: it is a plain class, so it
/// exists only where its source does.
///
/// Unlike the error hierarchy this one has imports of its own
/// (`zun:core`'s `date` and `time` namespaces). Those resolve from the middle
/// of the combined text, since import declarations are collected before the
/// body is checked rather than having to lead the file.
pub const DATE: &str = include_str!("../builtin/date.zun");

/// The `new Boolean(x)` wrapper class (`builtin/boolean.zun`), compiled with
/// every program for the same reason as [`ERRORS`] and [`DATE`].
pub const BOOLEAN: &str = include_str!("../builtin/boolean.zun");

/// The typed decorator context (`builtin/decorators.zun`), compiled with every
/// program for the same reason as [`ERRORS`].
pub const DECORATORS: &str = include_str!("../builtin/decorators.zun");

/// The ECMA-402 `Intl` classes (`builtin/intl.zun`), compiled with every program
/// for the same reason as [`ERRORS`]. `new Intl.X(..)` maps onto `IntlX`.
pub const INTL: &str = include_str!("../builtin/intl.zun");

/// The `zun:std:*` standard-library class/method sources. Each is a plain Zun
/// source string that the compiler compiles into the precompiled `zun:std`
/// module (see `zun_compiler`). They live here so `zun_prelude` owns every
/// `builtin/*.zun` file in-crate (no `include_str!` reaching outside a crate
/// directory, which `cargo publish` cannot package).
pub const MATH: &str = include_str!("../builtin/math.zun");
/// `String` prototype surface. See [`MATH`].
pub const STRING: &str = include_str!("../builtin/string.zun");
/// `Number` prototype surface. See [`MATH`].
pub const NUMBER: &str = include_str!("../builtin/number.zun");
/// `BigInt` surface. See [`MATH`].
pub const BIGINT: &str = include_str!("../builtin/bigint.zun");
/// `Array` prototype surface. See [`MATH`].
pub const ARRAY: &str = include_str!("../builtin/array.zun");
/// `Map` surface. See [`MATH`].
pub const MAP: &str = include_str!("../builtin/map.zun");
/// `Set` surface. See [`MATH`].
pub const SET: &str = include_str!("../builtin/set.zun");
/// `WeakMap` surface. See [`MATH`].
pub const WEAK_MAP: &str = include_str!("../builtin/weak_map.zun");
/// `WeakSet` surface. See [`MATH`].
pub const WEAK_SET: &str = include_str!("../builtin/weak_set.zun");
/// `WeakRef` surface. See [`MATH`].
pub const WEAK_REF: &str = include_str!("../builtin/weak_ref.zun");
/// `FinalizationRegistry` surface. See [`MATH`].
pub const FINALIZATION_REGISTRY: &str = include_str!("../builtin/finalization_registry.zun");
/// `RegExp` surface. See [`MATH`].
pub const REGEXP: &str = include_str!("../builtin/regexp.zun");

/// The base-type global registry (`builtin/realm.zun`), consumed by
/// `zun_sema::realm`. See [`MATH`]; kept here so the file lives in-crate.
pub const REALM: &str = include_str!("../builtin/realm.zun");

/// Stable content fingerprint for cache invalidation of compiled prelude data.
///
/// This is deliberately a small, deterministic identifier rather than a
/// cryptographic signature; distribution hosts must still authenticate their
/// artifacts separately.
pub const fn fingerprint() -> u64 {
    let bytes = PRELUDE.as_bytes();
    let mut hash = 0xcbf29ce484222325u64;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        index += 1;
    }
    hash
}

/// Combine user source with the base prelude, returning the full source to
/// compile. The user's text comes first so its byte offsets are unchanged.
pub fn with_prelude(user_source: &str) -> String {
    let mut combined = String::with_capacity(
        user_source.len() + PRELUDE.len() + ERRORS.len() + DATE.len() + BOOLEAN.len() + DECORATORS.len() + INTL.len() + 6,
    );
    combined.push_str(user_source);
    combined.push('\n');
    combined.push_str(PRELUDE);
    combined.push('\n');
    combined.push_str(ERRORS);
    combined.push('\n');
    combined.push_str(DATE);
    combined.push('\n');
    combined.push_str(BOOLEAN);
    combined.push('\n');
    combined.push_str(DECORATORS);
    combined.push('\n');
    combined.push_str(INTL);
    combined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_source_keeps_leading_offsets() {
        let combined = with_prelude("function main() {}");
        assert!(combined.starts_with("function main() {}"));
        assert!(combined.contains("function clamp"));
    }

    #[test]
    fn fingerprint_is_deterministic_and_nonzero() {
        assert_eq!(fingerprint(), fingerprint());
        assert_ne!(fingerprint(), 0);
    }
}
