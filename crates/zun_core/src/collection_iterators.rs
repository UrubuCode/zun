use super::{Any, OrderedMap};

/// State for the default `Map[Symbol.iterator]()` protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapIterator {
    next_index: usize,
}

impl MapIterator {
    pub fn new() -> Self {
        Self { next_index: 0 }
    }

    pub fn next(&mut self, map: &OrderedMap) -> Option<(Any, Any)> {
        let (slot, key, value) = map.physical_next(self.next_index)?;
        self.next_index = slot + 1;
        Some((key, value))
    }

    /// Whether all currently live entries have been consumed.
    pub fn is_done(&self, map: &OrderedMap) -> bool {
        map.physical_next(self.next_index).is_none()
    }
}

impl Default for MapIterator {
    fn default() -> Self {
        Self::new()
    }
}

/// State for the default `Set[Symbol.iterator]()` protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetIterator {
    next_index: usize,
}

impl SetIterator {
    pub fn new() -> Self {
        Self { next_index: 0 }
    }

    pub fn next(&mut self, set: &OrderedMap) -> Option<Any> {
        let (slot, value, _) = set.physical_next(self.next_index)?;
        self.next_index = slot + 1;
        Some(value)
    }

    /// Whether all currently live values have been consumed.
    pub fn is_done(&self, set: &OrderedMap) -> bool {
        set.physical_next(self.next_index).is_none()
    }
}

impl Default for SetIterator {
    fn default() -> Self {
        Self::new()
    }
}
