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
}
