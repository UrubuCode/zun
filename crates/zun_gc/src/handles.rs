#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeakRefId(pub(crate) u32);

impl WeakRefId {
    pub const fn get(self) -> u32 { self.0 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeakMapId(pub(crate) u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FinalizationId(pub(crate) u32);

impl FinalizationId {
    pub const fn from_raw(value: u32) -> Self { Self(value) }
    pub const fn get(self) -> u32 { self.0 }
}
