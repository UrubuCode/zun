/// Stable identity of a callable Zun function value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub struct FunctionHandle {
    method_rid: u32,
    reserved: u32,
    environment: usize,
}

/// `reserved` discriminant marking a `bind`-produced bound function. Its
/// `environment` points to the GC-managed bound record (target + prepended
/// arguments) rather than a closure environment, and `method_rid` is unused.
const BOUND_TAG: u32 = 1;

impl FunctionHandle {
    pub const fn new(method_rid: u32, environment: Option<usize>) -> Self {
        Self { method_rid, reserved: 0, environment: match environment { Some(address) => address, None => 0 } }
    }

    /// A bound function value (`Function.prototype.bind`). `record` is the
    /// address of the managed bound record holding the target function and the
    /// prepended arguments; it is traced through the handle's `environment`
    /// slot exactly like a closure environment.
    pub const fn new_bound(record: usize) -> Self {
        Self { method_rid: 0, reserved: BOUND_TAG, environment: record }
    }

    pub const fn method_rid(self) -> u32 { self.method_rid }
    pub const fn environment(self) -> Option<usize> {
        if self.environment == 0 { None } else { Some(self.environment) }
    }

    /// Whether this handle is a `bind`-produced bound function whose
    /// `environment` is a bound record rather than a closure environment.
    pub const fn is_bound(self) -> bool {
        self.reserved == BOUND_TAG
    }

    /// Return this handle with its managed closure environment relocated,
    /// preserving the bound discriminant so a bound function stays bound after
    /// a GC compaction.
    pub const fn relocate_environment(self, address: Option<usize>) -> Self {
        Self {
            method_rid: self.method_rid,
            reserved: self.reserved,
            environment: match address { Some(address) => address, None => 0 },
        }
    }

    /// Pack this handle into the two machine words of the JIT function carrier:
    /// the identity word is `method_rid` in the low 32 bits and `reserved`
    /// (the bound discriminant) in the high 32 bits; the second word is the
    /// `environment` address (a managed GC pointer, or 0). The carrier mirrors
    /// this layout so a function value can cross the JIT boundary without
    /// losing its closure environment to the one-word rid representation.
    pub const fn to_carrier_words(self) -> (u64, u64) {
        let identity = (self.method_rid as u64) | ((self.reserved as u64) << 32);
        (identity, self.environment as u64)
    }

    /// Reconstruct a handle from the two carrier words produced by
    /// [`Self::to_carrier_words`]. Inverse of that packing; the bound
    /// discriminant in the high half of `identity` is preserved.
    pub const fn from_carrier_words(identity: u64, environment: u64) -> Self {
        Self {
            method_rid: identity as u32,
            reserved: (identity >> 32) as u32,
            environment: environment as usize,
        }
    }
}

/// `reserved` base marking a generator method value (`gen.next`, `gen.return`,
/// `gen.throw`): the low bits select the method and `environment` is the
/// generator object (traced like a closure environment).
const GENERATOR_METHOD_BASE: u32 = 16;

impl FunctionHandle {
    /// A generator method value; `kind` is 0 = next, 1 = return, 2 = throw.
    pub const fn new_generator_method(kind: u32, generator: usize) -> Self {
        Self { method_rid: 0, reserved: GENERATOR_METHOD_BASE + kind, environment: generator }
    }

    /// The generator method selector (0 next, 1 return, 2 throw) when this
    /// handle is a generator method value.
    pub const fn generator_method(self) -> Option<u32> {
        if self.reserved >= GENERATOR_METHOD_BASE && self.reserved < GENERATOR_METHOD_BASE + 3 {
            Some(self.reserved - GENERATOR_METHOD_BASE)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FunctionHandle;

    #[test]
    fn plain_handle_is_not_bound() {
        let handle = FunctionHandle::new(7, Some(0x1000));
        assert!(!handle.is_bound());
        assert_eq!(handle.method_rid(), 7);
        assert_eq!(handle.environment(), Some(0x1000));
    }

    #[test]
    fn bound_handle_carries_its_record_and_relocates_preserving_the_tag() {
        let handle = FunctionHandle::new_bound(0x2000);
        assert!(handle.is_bound());
        assert_eq!(handle.environment(), Some(0x2000));
        let moved = handle.relocate_environment(Some(0x3000));
        assert!(moved.is_bound(), "relocation must keep the bound discriminant");
        assert_eq!(moved.environment(), Some(0x3000));
    }

    #[test]
    fn carrier_words_round_trip_a_plain_closure_handle() {
        let handle = FunctionHandle::new(7, Some(0x4000));
        let (identity, environment) = handle.to_carrier_words();
        assert_eq!(identity, 7, "no bound tag, so identity is just the rid");
        assert_eq!(environment, 0x4000);
        assert_eq!(FunctionHandle::from_carrier_words(identity, environment), handle);
    }

    #[test]
    fn carrier_words_round_trip_a_no_environment_handle() {
        let handle = FunctionHandle::new(42, None);
        let (identity, environment) = handle.to_carrier_words();
        assert_eq!(environment, 0, "a no-closure function has a zero environment word");
        let back = FunctionHandle::from_carrier_words(identity, environment);
        assert_eq!(back, handle);
        assert_eq!(back.environment(), None);
    }

    #[test]
    fn carrier_words_round_trip_a_bound_handle_preserving_the_tag() {
        let handle = FunctionHandle::new_bound(0x5000);
        let (identity, environment) = handle.to_carrier_words();
        let back = FunctionHandle::from_carrier_words(identity, environment);
        assert_eq!(back, handle);
        assert!(back.is_bound(), "the bound discriminant survives the carrier round-trip");
        assert_eq!(back.environment(), Some(0x5000));
    }
}
