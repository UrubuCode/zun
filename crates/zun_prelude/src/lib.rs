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

/// The prelude source, compiled with every program.
pub const PRELUDE: &str = include_str!("prelude.zun");

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
    let mut combined = String::with_capacity(user_source.len() + PRELUDE.len() + 1);
    combined.push_str(user_source);
    combined.push('\n');
    combined.push_str(PRELUDE);
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
