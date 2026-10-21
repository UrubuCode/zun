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
