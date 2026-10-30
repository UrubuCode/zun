use super::{Heap, Obj};

impl Heap {
    pub fn array_len(&self, object: Obj) -> Option<usize> {
        self.arrays.get(&(object.addr() as usize)).map(zun_core::ArrayStorage::len)
    }

    pub fn array_get(&self, object: Obj, index: usize) -> Option<zun_core::Any> {
        self.arrays.get(&(object.addr() as usize))?.get(index)
    }

    pub fn array_set(&mut self, object: Obj, index: usize, value: zun_core::Any) -> Option<zun_core::Any> {
        let result = self.arrays.get_mut(&(object.addr() as usize))?.set(index, value)?;
        if let Some(child) = value.as_object() {
            self.write_barrier(object, child as u64);
        }
        Some(result)
    }

    pub fn array_insert(&mut self, object: Obj, index: usize, value: zun_core::Any) -> bool {
        let inserted = self.arrays.get_mut(&(object.addr() as usize)).is_some_and(|array| array.insert(index, value));
        if inserted {
            if let Some(child) = value.as_object() {
                self.write_barrier(object, child as u64);
            }
        }
        inserted
    }

    pub fn array_remove(&mut self, object: Obj, index: usize) -> Option<zun_core::Any> {
        self.arrays.get_mut(&(object.addr() as usize))?.remove(index)
    }

    pub fn array_pop(&mut self, object: Obj) -> Option<zun_core::Any> {
        self.arrays.get_mut(&(object.addr() as usize))?.pop()
    }

    pub fn array_push(&mut self, object: Obj, value: zun_core::Any) -> Option<usize> {
        let length = {
            let array = self.arrays.get_mut(&(object.addr() as usize))?;
            array.push(value);
            array.len()
        };
        if let Some(child) = value.as_object() {
            self.write_barrier(object, child as u64);
        }
        Some(length)
    }
}

impl Heap {
    /// Integrity level of an array: 0 open, 1 non-extensible, 2 sealed, 3 frozen.
    /// Unknown objects report 0.
    pub fn array_integrity(&self, object: Obj) -> u8 {
        self.arrays.get(&(object.addr() as usize)).map_or(0, zun_core::ArrayStorage::integrity)
    }

    /// Raise (never lower) the integrity level of an array. Returns `false` for
    /// a non-array.
    pub fn array_raise_integrity(&mut self, object: Obj, level: u8) -> bool {
        match self.arrays.get_mut(&(object.addr() as usize)) {
            Some(array) => { array.raise_integrity(level); true }
            None => false,
        }
    }

    /// Shrink or grow an array to `new_length` (new slots are `0`-filled).
    pub fn array_resize(&mut self, object: Obj, new_length: usize) -> bool {
        match self.arrays.get_mut(&(object.addr() as usize)) {
            Some(array) => { array.resize(new_length); true }
            None => false,
        }
    }
}
