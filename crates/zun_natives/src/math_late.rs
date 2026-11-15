//! `zun:core` `mathLate` namespace — `Math` intrinsics added after the `math`
//! group was frozen.
//!
//! Native dispatch ids are global and assigned in [`crate::MODULES`] order, so
//! appending to the `math` group would shift every id after it. These belong to
//! `Math` by meaning and live here only to keep existing ids stable; their
//! implementations are the same ones the `math` group uses
//! (`crate::math::f16_round` and `crate::math::js_random`), so there is no
//! second implementation of either.

use crate::{NativeFn, NativeTy::*};

use crate::math::{f16round_late, random_late};

/// Appended last in [`crate::MODULES`] so every existing dispatch id stays
/// stable.
pub const MODULE: crate::Module = crate::Module {
    name: "mathLate",
    functions: &[
        // `Math.f16round(x)`: round to the nearest IEEE half-precision value.
        NativeFn {
            name: "f16roundNumber",
            params: &[F64],
            ret: F64,
            imp: f16round_late,
        },
        // `Math.random()`: a float in [0, 1).
        NativeFn {
            name: "randomNumber",
            params: &[],
            ret: F64,
            imp: random_late,
        },
    ],
};
