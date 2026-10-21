use std::collections::HashMap;

use crate::Obj;

/// Address translations produced by a future compacting collection.
///
/// The map is intentionally separate from collection: building it and using
/// it to rewrite known reference slots can be tested before relocation is
/// enabled in the collector.
#[derive(Debug, Default)]
pub struct RelocationMap {
    addresses: HashMap<usize, usize>,
}

impl RelocationMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, old: Obj, new: Obj) {
        self.insert_addresses(old.addr() as usize, new.addr() as usize);
    }

    pub fn insert_addresses(&mut self, old: usize, new: usize) {
        self.addresses.insert(old, new);
    }

    pub fn relocate(&self, object: Obj) -> Obj {
        let address = self.relocate_address(object.addr() as usize);
        unsafe { Obj::from_addr(address as *mut u64) }
    }

    pub fn relocate_address(&self, address: usize) -> usize {
        self.addresses.get(&address).copied().unwrap_or(address)
    }

    /// Rewrite a slot known by the collector to contain a managed object
    /// reference. Non-reference slots must never be passed here.
    ///
    /// # Safety
    /// `slot` must be a valid, writable pointer to an eight-byte reference
    /// slot for the duration of this call.
    pub unsafe fn rewrite_slot(&self, slot: *mut u64) -> bool {
        let old = unsafe { slot.read_unaligned() as usize };
        let new = self.relocate_address(old);
        if old == new {
            false
        } else {
            unsafe { slot.write_unaligned(new as u64) };
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RelocationMap;

    #[test]
    fn translates_objects_and_reference_slots() {
        let old = 0x1000usize;
        let new = 0x2000usize;
        let mut map = RelocationMap::new();
        map.insert_addresses(old, new);

        assert_eq!(map.relocate_address(old), new);
        assert_eq!(map.relocate_address(0), 0);

        let mut slot = old as u64;
        assert!(unsafe { map.rewrite_slot(&mut slot) });
        assert_eq!(slot, new as u64);
        assert!(!unsafe { map.rewrite_slot(&mut slot) });
    }
}
