use super::{Heap, Obj};

impl Heap {
    pub fn array_iterator_next(&mut self, iterator: Obj) -> Option<zun_core::Any> {
        let (array, state) = self.array_iterators.get_mut(&(iterator.addr() as usize))?;
        let mode = state.mode();
        let (index, value) = state.advance(self.arrays.get(&(array.addr() as usize))?)?;
        match mode {
            1 => Some(zun_core::Any::float(index as f64)),
            2 => {
                let pair = self.alloc_array(2);
                self.array_set(pair, 0, zun_core::Any::float(index as f64));
                self.array_set(pair, 1, value);
                Some(zun_core::Any::object(pair.addr() as usize))
            }
            _ => Some(value),
        }
    }
    /// The iteration mode of an array iterator (0 values, 1 keys, 2 entries,
    /// 3 string iterator).
    pub fn array_iterator_mode(&self, iterator: Obj) -> Option<u8> {
        let (_, state) = self.array_iterators.get(&(iterator.addr() as usize))?;
        Some(state.mode())
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
    /// One step of a `Map` cursor in its own mode: the `[key, value]` pair, the
    /// key, or the value.
    pub fn map_iterator_step(&mut self, iterator: Obj) -> Option<zun_core::Any> {
        let (map, state) = self.map_iterators.get_mut(&(iterator.addr() as usize))?;
        let mode = state.mode();
        let (key, value) = state.next(self.maps.get(&(map.addr() as usize))?)?;
        Some(match mode {
            1 => key,
            2 => value,
            _ => {
                let pair = self.alloc_array(2);
                self.array_set(pair, 0, key);
                self.array_set(pair, 1, value);
                zun_core::Any::object(pair.addr() as usize)
            }
        })
    }
    pub fn map_iterator_done(&self, iterator: Obj) -> Option<bool> {
        let (map, state) = self.map_iterators.get(&(iterator.addr() as usize))?;
        Some(state.is_done(self.maps.get(&(map.addr() as usize))?))
    }
}

impl Heap {
    /// Whether a `Map` cursor is exhausted, latching the answer: once it has
    /// reported completion it stays complete even if the map grows.
    pub fn map_iterator_done_latching(&mut self, iterator: Obj) -> Option<bool> {
        let (map, state) = self.map_iterators.get_mut(&(iterator.addr() as usize))?;
        let done = state.is_done(self.maps.get(&(map.addr() as usize))?);
        if done {
            state.latch_done();
        }
        Some(done)
    }

    /// The `Set` twin of [`Self::map_iterator_done_latching`].
    pub fn set_iterator_done_latching(&mut self, iterator: Obj) -> Option<bool> {
        let (set, state) = self.set_iterators.get_mut(&(iterator.addr() as usize))?;
        let done = state.is_done(self.sets.get(&(set.addr() as usize))?);
        if done {
            state.latch_done();
        }
        Some(done)
    }
}
