use super::{any_same_value_zero, Heap, Obj};

impl Heap {
    pub fn alloc_map(&mut self) -> Obj {
        let object = self.alloc(0, 0);
        self.maps
            .insert(object.addr() as usize, zun_core::OrderedMap::new());
        object
    }

    pub fn map_insert(
        &mut self,
        object: Obj,
        key: zun_core::Any,
        value: zun_core::Any,
    ) -> Option<zun_core::Any> {
        // Map keys normalise -0 to +0 (ECMAScript Map.prototype.set).
        let key = match key.as_float() {
            Some(f) if f == 0.0 => zun_core::Any::float(0.0),
            _ => key,
        };
        let hash = self.any_hash(key);
        let strings = &self.strings;
        let result =
            self.maps
                .get_mut(&(object.addr() as usize))?
                .insert(key, value, hash, |candidate| {
                    any_same_value_zero(candidate, key, strings)
                });
        if let Some(child) = key.as_object() {
            self.write_barrier(object, child as u64);
        }
        if let Some(child) = value.as_object() {
            self.write_barrier(object, child as u64);
        }
        result
    }

    pub fn map_get(&self, object: Obj, key: zun_core::Any) -> Option<zun_core::Any> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        self.maps
            .get(&(object.addr() as usize))?
            .get(hash, |candidate| {
                any_same_value_zero(candidate, key, strings)
            })
    }

    pub fn map_has(&self, object: Obj, key: zun_core::Any) -> Option<bool> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        Some(
            self.maps
                .get(&(object.addr() as usize))?
                .contains(hash, |candidate| {
                    any_same_value_zero(candidate, key, strings)
                }),
        )
    }

    pub fn map_clear(&mut self, object: Obj) -> Option<()> {
        self.maps.get_mut(&(object.addr() as usize))?.clear();
        Some(())
    }

    pub fn map_remove(
        &mut self,
        object: Obj,
        key: zun_core::Any,
    ) -> Option<(zun_core::Any, zun_core::Any)> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        self.maps
            .get_mut(&(object.addr() as usize))?
            .remove(hash, |candidate| {
                any_same_value_zero(candidate, key, strings)
            })
    }

    pub fn map_len(&self, object: Obj) -> Option<usize> {
        self.maps
            .get(&(object.addr() as usize))
            .map(zun_core::OrderedMap::len)
    }

    pub fn map_entries(&self, object: Obj) -> Option<Vec<(zun_core::Any, zun_core::Any)>> {
        Some(self.maps.get(&(object.addr() as usize))?.iter().collect())
    }

    /// Pin or unpin the physical layout of a Map / Set for a live `forEach` cursor.
    pub fn collection_pin(&mut self, object: Obj, pin: bool) {
        let key = object.addr() as usize;
        let table = match self.maps.get_mut(&key) {
            Some(table) => Some(table),
            None => self.sets.get_mut(&key),
        };
        if let Some(table) = table {
            if pin {
                table.pin()
            } else {
                table.unpin()
            }
        }
    }

    /// Physical slot `index` of a Map / Set: `None` once past the end, `Some(None)`
    /// for a deleted slot, otherwise the (key, value) entry (a Set's value is `true`).
    pub fn collection_cursor(
        &self,
        object: Obj,
        index: usize,
    ) -> Option<Option<(zun_core::Any, zun_core::Any)>> {
        let key = object.addr() as usize;
        let table = self.maps.get(&key).or_else(|| self.sets.get(&key))?;
        if index >= table.physical_len() {
            return None;
        }
        Some(table.physical_entry(index))
    }
}
