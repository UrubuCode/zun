use super::{Heap, Obj, Record, CHUNK_SLOTS};

impl Heap {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn allocate_identity_hash(&mut self) -> u64 {
        if self.next_identity_hash == 0 {
            self.next_identity_hash = 1;
        }
        let hash = self.next_identity_hash;
        self.next_identity_hash = self
            .next_identity_hash
            .checked_add(1)
            .expect("identity hash exhausted");
        hash
    }

    pub fn alloc(&mut self, type_rid: u32, field_count: usize) -> Obj {
        let identity_hash = self.allocate_identity_hash();
        if let Some(idx) = self.free.get_mut(&field_count).and_then(Vec::pop) {
            let addr = self.records[idx].addr;
            unsafe {
                std::ptr::write_bytes(addr, 0, zun_core::OBJECT_HEADER_SLOTS + field_count);
                *addr = type_rid as u64;
                *addr.add(1) = identity_hash;
            }
            self.records[idx].alive = true;
            self.records[idx].marked = false;
            self.by_addr.insert(addr as usize, idx);
            return Obj(addr);
        }

        let slots = zun_core::OBJECT_HEADER_SLOTS + field_count;
        assert!(slots <= CHUNK_SLOTS, "object larger than a heap chunk");
        if self.chunks.is_empty() || self.cursor + slots > CHUNK_SLOTS {
            self.chunks.push(vec![0u64; CHUNK_SLOTS].into_boxed_slice());
            self.cursor = 0;
        }
        let chunk = self.chunks.last_mut().expect("just pushed a chunk");
        let base = &mut chunk[self.cursor] as *mut u64;
        self.cursor += slots;
        unsafe {
            *base = type_rid as u64;
            *base.add(1) = identity_hash;
        }
        let idx = self.records.len();
        self.records.push(Record {
            addr: base,
            field_count,
            alive: true,
            marked: false,
        });
        self.by_addr.insert(base as usize, idx);
        Obj(base)
    }
}
