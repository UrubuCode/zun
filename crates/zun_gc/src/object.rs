/// A pointer to an allocated object: slot 0 is the type RID, slot 1 is the
/// identity hash, and the remaining slots are fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Obj(pub(crate) *mut u64);

impl Obj {
    pub fn addr(self) -> *mut u64 {
        self.0
    }

    pub unsafe fn from_addr(addr: *mut u64) -> Obj {
        Obj(addr)
    }

    pub fn type_rid(self) -> u32 {
        unsafe { *self.0 as u32 }
    }

    pub fn identity_hash(self) -> u64 {
        unsafe { *self.0.add(1) }
    }

    pub fn field(self, index: usize) -> u64 {
        unsafe { *self.0.add(zun_core::OBJECT_HEADER_SLOTS + index) }
    }

    pub fn set_field(self, index: usize, value: u64) {
        unsafe { *self.0.add(zun_core::OBJECT_HEADER_SLOTS + index) = value }
    }
}
