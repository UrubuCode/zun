//! Two-word carrier used when a managed function value crosses the JIT
//! boundary. A function value may hold a closure environment (or a bound
//! record), which the one-word method-rid representation cannot carry; this
//! carrier mirrors [`zun_core::FunctionHandle`] so the environment survives.
//!
//! Staged for the JIT closure carrier (see `docs/design/jit-closure-carrier.md`,
//! which specifies the single atomic change that wires this through
//! `translate.rs`, `shim.rs`, the bridge, and GC rooting). Kept standalone and
//! unit-tested until that change consumes it.
#![allow(dead_code)]

use zun_core::FunctionHandle;

/// Native layout of the two-word managed function carrier.
pub const IDENTITY_OFFSET: i32 = 0;
pub const ENVIRONMENT_OFFSET: i32 = 8;
pub const CARRIER_SIZE: u32 = IDENTITY_OFFSET as u32 + 16;
pub const CARRIER_ALIGN_SHIFT: u8 = 3;

/// C-ABI representation of a [`zun_core::FunctionHandle`] at the JIT boundary.
///
/// The identity and environment are intentionally separate machine words, as in
/// the `Any` carrier: keeping the carrier explicit stops the typed JIT from
/// treating the 16-byte function value as a single `i64` and discarding the
/// closure environment (a managed object reference) that precise GC must trace.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitFunctionAbi {
    /// Packed identity: `method_rid` in the low 32 bits, the bound discriminant
    /// in the high 32 bits (see [`FunctionHandle::to_carrier_words`]).
    pub identity: u64,
    /// Environment word: a managed closure-environment / bound-record address,
    /// or 0 for a no-closure function value.
    pub environment: u64,
}

impl JitFunctionAbi {
    /// Return the carrier as the two machine words used at the JIT boundary.
    pub const fn raw_words(self) -> (u64, u64) {
        (self.identity, self.environment)
    }

    /// Reconstruct a carrier from its two machine words.
    pub const fn from_raw_words(identity: u64, environment: u64) -> Self {
        Self { identity, environment }
    }

    /// Encode a runtime function handle for a native call boundary.
    pub const fn from_handle(handle: FunctionHandle) -> Self {
        let (identity, environment) = handle.to_carrier_words();
        Self { identity, environment }
    }

    /// Decode a carrier back into a runtime function handle.
    pub const fn into_handle(self) -> FunctionHandle {
        FunctionHandle::from_carrier_words(self.identity, self.environment)
    }

    /// Append the environment as a GC root when it is a managed address. Unlike
    /// the `Any` carrier, no tag inspection is needed: the environment word is a
    /// managed reference iff it is non-zero (a closure environment and a bound
    /// record are traced the same way, exactly as `FunctionHandle::environment`
    /// and `relocate_environment` treat them on the interpreter side).
    pub fn append_managed_root(self, roots: &mut Vec<zun_gc::Obj>) {
        if self.environment != 0 {
            roots.push(unsafe { zun_gc::Obj::from_addr(self.environment as *mut u64) });
        }
    }

    /// Rewrite the environment address after a moving collection. Returns
    /// whether the word changed.
    pub fn rewrite_managed_environment(&mut self, relocation: &zun_gc::RelocationMap) -> bool {
        if self.environment == 0 {
            return false;
        }
        let mapped = relocation.relocate_address(self.environment as usize);
        if mapped == self.environment as usize {
            return false;
        }
        self.environment = mapped as u64;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        JitFunctionAbi, CARRIER_ALIGN_SHIFT, CARRIER_SIZE, ENVIRONMENT_OFFSET, IDENTITY_OFFSET,
    };
    use zun_core::FunctionHandle;

    #[test]
    fn carrier_layout_matches_the_any_carrier_shape() {
        assert_eq!(IDENTITY_OFFSET, 0);
        assert_eq!(ENVIRONMENT_OFFSET, 8);
        assert_eq!(CARRIER_SIZE, 16);
        assert_eq!(CARRIER_ALIGN_SHIFT, 3);
        assert_eq!(core::mem::size_of::<JitFunctionAbi>(), 16);
        assert_eq!(core::mem::align_of::<JitFunctionAbi>(), 8);
    }

    #[test]
    fn carrier_roundtrips_a_closure_handle_without_collapsing_two_words() {
        let handle = FunctionHandle::new(7, Some(0x1234));
        let carrier = JitFunctionAbi::from_handle(handle);
        assert_eq!(carrier.into_handle(), handle);
        let words = carrier.raw_words();
        assert_eq!(JitFunctionAbi::from_raw_words(words.0, words.1), carrier);
    }

    #[test]
    fn a_no_closure_handle_has_a_zero_environment_and_is_not_rooted() {
        let carrier = JitFunctionAbi::from_handle(FunctionHandle::new(3, None));
        assert_eq!(carrier.environment, 0);
        let mut roots = Vec::new();
        carrier.append_managed_root(&mut roots);
        assert!(roots.is_empty(), "a no-closure function value roots nothing");
    }

    #[test]
    fn carrier_exposes_and_rewrites_a_managed_environment() {
        let old = 0x1000usize;
        let new = 0x2000usize;
        let mut carrier = JitFunctionAbi::from_handle(FunctionHandle::new(5, Some(old)));

        let mut roots = Vec::new();
        carrier.append_managed_root(&mut roots);
        assert_eq!(roots[0].addr() as usize, old);

        let mut relocation = zun_gc::RelocationMap::new();
        relocation.insert_addresses(old, new);
        assert!(carrier.rewrite_managed_environment(&relocation));
        assert_eq!(carrier.environment, new as u64);
        assert_eq!(carrier.into_handle().environment(), Some(new));
    }
}
