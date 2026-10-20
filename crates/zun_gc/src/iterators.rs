use super::{Heap, Obj};

impl Heap {
    pub fn array_iterator_next(&mut self, iterator: Obj) -> Option<zun_core::Any> {
        let (array, state) = self.array_iterators.get_mut(&(iterator.addr() as usize))?;
        state.next(self.arrays.get(&(array.addr() as usize))?)
    }
    pub fn array_iterator_done(&self, iterator: Obj) -> Option<bool> {
        let (array, state) = self.array_iterators.get(&(iterator.addr() as usize))?;
        Some(state.is_done(self.arrays.get(&(array.addr() as usize))?))
    }
    pub fn set_iterator_next(&mut self, iterator: Obj) -> Option<zun_core::Any> {
        let (set, state) = self.set_iterators.get_mut(&(iterator.addr() as usize))?;
        state.next(self.sets.get(&(set.addr() as usize))?)
    }
    pub fn set_iterator_done(&self, iterator: Obj) -> Option<bool> {
        let (set, state) = self.set_iterators.get(&(iterator.addr() as usize))?;
        Some(state.is_done(self.sets.get(&(set.addr() as usize))?))
    }
    pub fn map_iterator_next(&mut self, iterator: Obj) -> Option<Obj> {
        let (map, state) = self.map_iterators.get_mut(&(iterator.addr() as usize))?;
        let (key, value) = state.next(self.maps.get(&(map.addr() as usize))?)?;
        let pair = self.alloc_array(2);
        self.array_set(pair, 0, key);
        self.array_set(pair, 1, value);
        Some(pair)
    }
    pub fn map_iterator_done(&self, iterator: Obj) -> Option<bool> {
        let (map, state) = self.map_iterators.get(&(iterator.addr() as usize))?;
        Some(state.is_done(self.maps.get(&(map.addr() as usize))?))
    }
}
