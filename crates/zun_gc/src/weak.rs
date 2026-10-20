use super::{FinalizationRecord, Heap, Obj, WeakMapId, WeakRefId};

impl Heap {
    pub fn weak_ref_get(&self, id: WeakRefId) -> Option<Obj> {
        let address = self.weak_refs.get(id.0 as usize)?.target?;
        self.by_addr
            .get(&address)
            .map(|&index| Obj(self.records[index].addr))
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
        let key = key.addr() as usize;
        let value = value.addr() as usize;
        if let Some((_, existing_value)) = entry
            .entries
            .iter_mut()
            .find(|(existing_key, _)| *existing_key == key)
        {
            *existing_value = value;
        } else {
            entry.entries.push((key, value));
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

    pub fn finalization_drain(&mut self) -> Vec<u64> {
        self.finalization_drain_records()
            .into_iter()
            .map(|record| record.token)
            .collect()
    }

    pub fn finalization_drain_records(&mut self) -> Vec<FinalizationRecord> {
        std::mem::take(&mut self.finalization_queue)
    }
}
