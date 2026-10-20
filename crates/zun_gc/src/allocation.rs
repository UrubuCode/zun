use super::{Ephemeron, FinalizationId, FinalizerEntry, Heap, Obj, Record, WeakMapId, WeakRefId, WeakSlot, CHUNK_SLOTS};

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

    pub fn alloc_dynamic(&mut self) -> Obj {
        let object = self.alloc(0, 0);
        self.dynamic_objects
            .insert(object.addr() as usize, zun_core::DynamicObject::new());
        object
    }

    /// Allocate a managed Promise object with pending settlement.
    pub fn alloc_promise(&mut self) -> Obj {
        let object = self.alloc(zun_core::CoreTypeId::Promise.get() as u32, 0);
        self.promises
            .insert(object.addr() as usize, zun_core::ManagedPromise::new());
        object
    }

    pub fn promise_state(&self, object: Obj) -> Option<zun_core::ManagedPromiseState> {
        self.promises.get(&(object.addr() as usize)).map(|promise| promise.state())
    }

    pub fn promise_result(&self, object: Obj) -> Option<zun_core::Any> {
        self.promises.get(&(object.addr() as usize)).and_then(|promise| promise.result())
    }

    pub fn promise_is_ready(&self, object: Obj) -> Option<bool> {
        self.promises.get(&(object.addr() as usize)).map(|promise| promise.is_ready())
    }

    pub fn promise_error(&self, object: Obj) -> Option<String> {
        self.promises
            .get(&(object.addr() as usize))
            .and_then(|promise| promise.error().map(str::to_owned))
    }

    pub fn promise_resolve(&mut self, object: Obj, value: zun_core::Any) -> Option<bool> {
        self.promises
            .get_mut(&(object.addr() as usize))
            .map(|promise| promise.resolve(value))
    }

    pub fn promise_reject(&mut self, object: Obj, error: impl Into<String>) -> Option<bool> {
        self.promises
            .get_mut(&(object.addr() as usize))
            .map(|promise| promise.reject(error))
    }

    pub fn alloc_array(&mut self, length: usize) -> Obj {
        let object = self.alloc(0, 0);
        self.arrays.insert(
            object.addr() as usize,
            zun_core::ArrayStorage::with_length(length),
        );
        object
    }

    pub fn alloc_buffer(&mut self, length: usize) -> Obj {
        let object = self.alloc(0, 0);
        self.buffers
            .insert(object.addr() as usize, zun_core::ByteBuffer::new(length));
        object
    }

    pub fn alloc_data_view(&mut self, buffer: Obj, offset: usize, length: usize) -> Option<Obj> {
        let view = zun_core::DataView::new(self.buffer_len(buffer)?, offset, length)?;
        let object = self.alloc(0, 1);
        object.set_field(0, buffer.addr() as u64);
        self.data_views.insert(object.addr() as usize, view);
        Some(object)
    }

    pub fn alloc_u32_array(&mut self, buffer: Obj, offset: usize, length: usize) -> Option<Obj> {
        let view = zun_core::TypedArrayView::new(
            self.buffer_len(buffer)?, offset, length, zun_core::TypedElement::U32,
        )?;
        let object = self.alloc(0, 1);
        object.set_field(0, buffer.addr() as u64);
        self.typed_views.insert(object.addr() as usize, (buffer, view));
        Some(object)
    }

    pub fn alloc_u8_array(&mut self, buffer: Obj, offset: usize, length: usize) -> Option<Obj> {
        let view = zun_core::TypedArrayView::new(
            self.buffer_len(buffer)?, offset, length, zun_core::TypedElement::U8,
        )?;
        let object = self.alloc(0, 1);
        object.set_field(0, buffer.addr() as u64);
        self.typed_views.insert(object.addr() as usize, (buffer, view));
        Some(object)
    }

    pub fn alloc_f64_array(&mut self, buffer: Obj, offset: usize, length: usize) -> Option<Obj> {
        let view = zun_core::TypedArrayView::new(
            self.buffer_len(buffer)?, offset, length, zun_core::TypedElement::F64,
        )?;
        let object = self.alloc(0, 1);
        object.set_field(0, buffer.addr() as u64);
        self.typed_views.insert(object.addr() as usize, (buffer, view));
        Some(object)
    }

    pub fn alloc_u16_array(&mut self, buffer: Obj, offset: usize, length: usize) -> Option<Obj> {
        let view = zun_core::TypedArrayView::new(
            self.buffer_len(buffer)?, offset, length, zun_core::TypedElement::U16,
        )?;
        let object = self.alloc(0, 1);
        object.set_field(0, buffer.addr() as u64);
        self.typed_views.insert(object.addr() as usize, (buffer, view));
        Some(object)
    }

    pub fn alloc_array_iterator(&mut self, array: Obj) -> Option<Obj> {
        self.array_len(array)?;
        let iterator = self.alloc(0, 1);
        iterator.set_field(0, array.addr() as u64);
        self.array_iterators.insert(
            iterator.addr() as usize,
            (array, zun_core::ArrayIterator::new()),
        );
        Some(iterator)
    }

    pub fn alloc_set_iterator(&mut self, set: Obj) -> Option<Obj> {
        self.set_len(set)?;
        let iterator = self.alloc(0, 1);
        iterator.set_field(0, set.addr() as u64);
        self.set_iterators.insert(
            iterator.addr() as usize,
            (set, zun_core::SetIterator::new()),
        );
        Some(iterator)
    }

    pub fn alloc_map_iterator(&mut self, map: Obj) -> Option<Obj> {
        self.map_len(map)?;
        let iterator = self.alloc(0, 1);
        iterator.set_field(0, map.addr() as u64);
        self.map_iterators.insert(
            iterator.addr() as usize,
            (map, zun_core::MapIterator::new()),
        );
        Some(iterator)
    }

    pub fn alloc_date(&mut self, millis: Option<i64>) -> Obj {
        let object = self.alloc(0, 0);
        self.dates.insert(
            object.addr() as usize,
            millis.map_or(
                zun_core::DateValue::invalid(),
                zun_core::DateValue::from_millis,
            ),
        );
        object
    }

    pub fn alloc_date_now(&mut self) -> Obj {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .ok();
        self.alloc_date(millis)
    }

    pub fn alloc_error(&mut self, name: impl Into<String>, message: impl Into<String>) -> Obj {
        let object = self.alloc(0, 0);
        self.errors.insert(
            object.addr() as usize,
            zun_core::ErrorValue::new(name, message),
        );
        object
    }

    pub fn weak_ref_new(&mut self, target: Obj) -> WeakRefId {
        let id = WeakRefId(self.weak_refs.len() as u32);
        self.weak_refs.push(WeakSlot { target: Some(target.addr() as usize) });
        id
    }

    pub fn weak_map_new(&mut self) -> WeakMapId {
        let id = WeakMapId(self.weak_maps.len() as u32);
        self.weak_maps.push(Ephemeron::default());
        id
    }

    pub fn alloc_weak_map(&mut self) -> Obj {
        let object = self.alloc(0, 0);
        let id = self.weak_map_new();
        self.weak_map_objects.insert(object.addr() as usize, id);
        object
    }

    pub fn finalization_register(&mut self, target: Obj, token: u64) -> FinalizationId {
        self.finalization_register_with_callback(target, token, 0)
    }

    pub fn finalization_register_with_callback(
        &mut self,
        target: Obj,
        token: u64,
        callback_id: u64,
    ) -> FinalizationId {
        let id = FinalizationId(self.finalizers.len() as u32);
        self.finalizers.push(FinalizerEntry {
            target: target.addr() as usize,
            token,
            callback_id,
            queued: false,
        });
        id
    }
}
