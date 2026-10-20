use super::{any_same_value_zero, Heap, Obj};

impl Heap {
    pub fn alloc_set(&mut self) -> Obj {
        let object = self.alloc(0, 0);
        self.sets.insert(object.addr() as usize, zun_core::OrderedMap::new());
        object
    }

    pub fn set_add(&mut self, object: Obj, value: zun_core::Any) -> Option<bool> {
        let hash = self.any_hash(value);
        let strings = &self.strings;
        let set = self.sets.get_mut(&(object.addr() as usize))?;
        let existed = set.get(hash, |candidate| any_same_value_zero(candidate, value, strings)).is_some();
        if !existed {
            set.insert(value, zun_core::Any::bool(true), hash, |candidate| any_same_value_zero(candidate, value, strings));
        }
        Some(!existed)
    }

    pub fn set_has(&self, object: Obj, value: zun_core::Any) -> Option<bool> {
        let hash = self.any_hash(value);
        let strings = &self.strings;
        Some(self.sets.get(&(object.addr() as usize))?.get(hash, |candidate| any_same_value_zero(candidate, value, strings)).is_some())
    }

    pub fn set_remove(&mut self, object: Obj, value: zun_core::Any) -> Option<bool> {
        let hash = self.any_hash(value);
        let strings = &self.strings;
        Some(self.sets.get_mut(&(object.addr() as usize))?.remove(hash, |candidate| any_same_value_zero(candidate, value, strings)).is_some())
    }

    pub fn set_len(&self, object: Obj) -> Option<usize> {
        self.sets.get(&(object.addr() as usize)).map(zun_core::OrderedMap::len)
    }

    pub fn set_clear(&mut self, object: Obj) -> Option<()> {
        self.sets.get_mut(&(object.addr() as usize))?.clear();
        Some(())
    }

    pub fn set_values(&self, object: Obj) -> Option<Vec<zun_core::Any>> {
        Some(self.sets.get(&(object.addr() as usize))?.iter().map(|(key, _)| key).collect())
    }
}
