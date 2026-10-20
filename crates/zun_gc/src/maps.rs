use super::{any_same_value_zero, Heap, Obj};

impl Heap {
    pub fn alloc_map(&mut self) -> Obj {
        let object = self.alloc(0, 0);
        self.maps.insert(object.addr() as usize, zun_core::OrderedMap::new());
        object
    }

    pub fn map_insert(&mut self, object: Obj, key: zun_core::Any, value: zun_core::Any) -> Option<zun_core::Any> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        self.maps.get_mut(&(object.addr() as usize))?.insert(key, value, hash, |candidate| any_same_value_zero(candidate, key, strings))
    }

    pub fn map_get(&self, object: Obj, key: zun_core::Any) -> Option<zun_core::Any> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        self.maps.get(&(object.addr() as usize))?.get(hash, |candidate| any_same_value_zero(candidate, key, strings))
    }

    pub fn map_has(&self, object: Obj, key: zun_core::Any) -> Option<bool> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        Some(self.maps.get(&(object.addr() as usize))?.contains(hash, |candidate| any_same_value_zero(candidate, key, strings)))
    }

    pub fn map_clear(&mut self, object: Obj) -> Option<()> {
        self.maps.get_mut(&(object.addr() as usize))?.clear();
        Some(())
    }

    pub fn map_remove(&mut self, object: Obj, key: zun_core::Any) -> Option<(zun_core::Any, zun_core::Any)> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        self.maps.get_mut(&(object.addr() as usize))?.remove(hash, |candidate| any_same_value_zero(candidate, key, strings))
    }

    pub fn map_len(&self, object: Obj) -> Option<usize> {
        self.maps.get(&(object.addr() as usize)).map(zun_core::OrderedMap::len)
    }

    pub fn map_entries(&self, object: Obj) -> Option<Vec<(zun_core::Any, zun_core::Any)>> {
        Some(self.maps.get(&(object.addr() as usize))?.iter().collect())
    }
}
