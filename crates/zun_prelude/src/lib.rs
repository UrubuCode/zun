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
//! This is distinct from the **intrinsics** (`zun:fs`, `zun:io`, ...), which are
//! runtime-implemented in Rust (see `zun_intrinsics`).

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
pub const ERRORS: &str = include_str!("../../../builtin/error.zun");

/// The `Date` calendar class (`builtin/date.zun`), also compiled with every
/// program, and for the same reason as [`ERRORS`]: it is a plain class, so it
/// exists only where its source does.
///
/// Unlike the error hierarchy this one has imports of its own
/// (`zun:core`'s `date` and `time` namespaces). Those resolve from the middle
/// of the combined text, since import declarations are collected before the
/// body is checked rather than having to lead the file.
pub const DATE: &str = include_str!("../../../builtin/date.zun");

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
        user_source.len() + PRELUDE.len() + ERRORS.len() + DATE.len() + 3,
    );
    combined.push_str(user_source);
    combined.push('\n');
    combined.push_str(PRELUDE);
    combined.push('\n');
    combined.push_str(ERRORS);
    combined.push('\n');
    combined.push_str(DATE);
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
