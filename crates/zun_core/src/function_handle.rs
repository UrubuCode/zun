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
}
