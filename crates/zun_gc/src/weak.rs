use super::{FinalizationRecord, Heap, Obj, WeakMapId, WeakRefId};

impl Heap {
    pub fn weak_ref_get(&self, id: WeakRefId) -> Option<Obj> {
        let address = self.weak_refs.get(id.0 as usize)?.target?;
        self.by_addr
            .get(&address)
            .map(|&index| Obj(self.records[index].addr))
    }

    /// Dereference a managed WeakRef object, returning its target while the
    /// target is still live, or `None` once it has been collected.
    pub fn weak_ref_get_object(&self, object: Obj) -> Option<Obj> {
        let id = self
            .weak_ref_objects
            .get(&(object.addr() as usize))
            .copied()?;
        // A function referent is read through `weak_ref_get_function`; its slot
        // target is the closure environment, never the referent itself.
        if self.weak_refs.get(id.0 as usize)?.callee.is_some() {
            return None;
        }
        self.weak_ref_get(id)
    }

    /// Dereference a managed WeakRef whose referent is a function value:
    /// `Some(function)` while it is live, `None` once its environment has been
    /// collected, and `None` when the referent is not a function at all.
    pub fn weak_ref_get_function(&self, object: Obj) -> Option<zun_core::FunctionHandle> {
        let id = self
            .weak_ref_objects
            .get(&(object.addr() as usize))
            .copied()?;
        let slot = self.weak_refs.get(id.0 as usize)?;
        let callee = slot.callee?;
        if !slot.env_bearing {
            return Some(callee);
        }
        let address = slot.target?;
        let index = *self.by_addr.get(&address)?;
        Some(callee.relocate_environment(Some(self.records[index].addr as usize)))
    }

    /// The unregistered symbol a managed WeakRef refers to, if its referent is one.
    pub fn weak_ref_get_symbol(&self, object: Obj) -> Option<zun_core::SymbolId> {
        let id = self
            .weak_ref_objects
            .get(&(object.addr() as usize))
            .copied()?;
        self.weak_refs.get(id.0 as usize)?.symbol
    }

    /// Whether `object` is a managed WeakRef whose referent is a function value.
    pub fn is_weak_ref_function(&self, object: Obj) -> bool {
        self.weak_ref_objects
            .get(&(object.addr() as usize))
            .and_then(|id| self.weak_refs.get(id.0 as usize))
            .is_some_and(|slot| slot.callee.is_some())
    }

    /// Whether `object` is a managed WeakRef.
    pub fn is_weak_ref(&self, object: Obj) -> bool {
        self.weak_ref_objects
            .contains_key(&(object.addr() as usize))
    }

    /// Whether `object` is a `WeakSet` rather than a `WeakMap`. The two share
    /// one ephemeron floor, so this is the only thing that tells them apart.
    pub fn is_weak_set(&self, object: Obj) -> bool {
        self.weak_set_objects.contains(&(object.addr() as usize))
    }

    /// Whether `object` is a WeakMap / WeakSet (they share the ephemeron floor).
    pub fn is_weak_collection(&self, object: Obj) -> bool {
        self.weak_map_objects
            .contains_key(&(object.addr() as usize))
    }

    pub fn weak_map_set_object(&mut self, object: Obj, key: Obj, value: Obj) -> bool {
        self.weak_map_objects
            .get(&(object.addr() as usize))
            .copied()
            .is_some_and(|id| self.weak_map_set(id, key, value))
    }

    pub fn weak_map_get_object(&self, object: Obj) -> Option<(Obj, Obj)> {
        let id = self
            .weak_map_objects
            .get(&(object.addr() as usize))
            .copied()?;
        self.weak_map_get(id)
    }

    pub fn weak_map_has_object(&self, object: Obj, key: Obj) -> bool {
        self.weak_map_get_value_object(object, key).is_some()
    }

    pub fn weak_map_get_value_object(&self, object: Obj, key: Obj) -> Option<Obj> {
        let id = self
            .weak_map_objects
            .get(&(object.addr() as usize))
            .copied()?;
        let entry = self.weak_maps.get(id.0 as usize)?;
        let key_address = key.addr() as usize;
        let (_, value_address) = entry
            .entries
            .iter()
            .find(|(candidate, _)| *candidate == key_address)?;
        let value = self.by_addr.get(value_address)?;
        Some(Obj(self.records[*value].addr))
    }

    pub fn weak_map_delete_object(&mut self, object: Obj, key: Obj) -> bool {
        let Some(id) = self
            .weak_map_objects
            .get(&(object.addr() as usize))
            .copied()
        else {
            return false;
        };
        let Some(entry) = self.weak_maps.get_mut(id.0 as usize) else {
            return false;
        };
        let key_address = key.addr() as usize;
        let before = entry.entries.len();
        entry
            .entries
            .retain(|(candidate, _)| *candidate != key_address);
        entry.entries.len() != before
    }

    pub fn weak_map_set(&mut self, id: WeakMapId, key: Obj, value: Obj) -> bool {
        let Some(entry) = self.weak_maps.get_mut(id.0 as usize) else {
            return false;
        };
        let key_addr = key.addr() as usize;
        let value_addr = value.addr() as usize;
        if let Some((_, existing_value)) = entry
            .entries
            .iter_mut()
            .find(|(existing_key, _)| *existing_key == key_addr)
        {
            *existing_value = value_addr;
        } else {
            entry.entries.push((key_addr, value_addr));
        }
        if let Some((&address, _)) = self
            .weak_map_objects
            .iter()
            .find(|(_, candidate)| *candidate == &id)
        {
            let owner = unsafe { Obj::from_addr(address as *mut u64) };
            self.write_barrier(owner, key_addr as u64);
            self.write_barrier(owner, value_addr as u64);
        }
        true
    }

    pub fn weak_map_get(&self, id: WeakMapId) -> Option<(Obj, Obj)> {
        let entry = self.weak_maps.get(id.0 as usize)?;
        let (key_address, value_address) = *entry.entries.first()?;
        let key = self.by_addr.get(&key_address)?;
        let value = self.by_addr.get(&value_address)?;
        Some((Obj(self.records[*key].addr), Obj(self.records[*value].addr)))
    }

    pub fn weak_map_entries(&self, id: WeakMapId) -> Vec<(Obj, Obj)> {
        let Some(entry) = self.weak_maps.get(id.0 as usize) else {
            return Vec::new();
        };
        entry
            .entries
            .iter()
            .filter_map(|&(key_address, value_address)| {
                let key = self.by_addr.get(&key_address)?;
                let value = self.by_addr.get(&value_address)?;
                Some((Obj(self.records[*key].addr), Obj(self.records[*value].addr)))
            })
            .collect()
    }

    /// Drain the queue, projecting each held value back to the integer token
    /// it was registered with (the convenience `u64` register path). Held
    /// values that are not integers project to `0`.
    pub fn finalization_drain(&mut self) -> Vec<u64> {
        self.finalization_drain_records()
            .into_iter()
            .map(|record| record.held.as_int().map(|value| value as u64).unwrap_or(0))
            .collect()
    }

    pub fn finalization_drain_records(&mut self) -> Vec<FinalizationRecord> {
        std::mem::take(&mut self.finalization_queue)
    }
}
