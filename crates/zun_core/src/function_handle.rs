/// Stable identity of a callable Zun function value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub struct FunctionHandle {
    method_rid: u32,
    reserved: u32,
    environment: usize,
}

impl FunctionHandle {
    pub const fn new(method_rid: u32, environment: Option<usize>) -> Self {
        Self { method_rid, reserved: 0, environment: match environment { Some(address) => address, None => 0 } }
    }
    pub const fn method_rid(self) -> u32 { self.method_rid }
    pub const fn environment(self) -> Option<usize> {
        if self.environment == 0 { None } else { Some(self.environment) }
    }

    /// Return this handle with its managed closure environment relocated.
    pub const fn relocate_environment(self, address: Option<usize>) -> Self {
        Self::new(self.method_rid(), address)
    }

}
