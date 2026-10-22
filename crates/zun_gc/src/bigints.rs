use super::{Heap, Obj};

impl Heap {
    /// Allocate a managed BigInt object whose arbitrary-precision payload is
    /// owned by the heap side table. The payload contains no managed
    /// references, so tracing only needs to keep the owning object alive.
    pub fn alloc_bigint(&mut self, value: zun_core::BigInt) -> Obj {
        let object = self.alloc(zun_core::CoreTypeId::BigInt.get() as u32, 0);
        self.bigints.insert(object.addr() as usize, value);
        object
    }

    pub fn bigint_value(&self, object: Obj) -> Option<&zun_core::BigInt> {
        self.bigints.get(&(object.addr() as usize))
    }
}
