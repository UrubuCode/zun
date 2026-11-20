use super::{Any, OrderedMap};

/// State for the default `Map[Symbol.iterator]()` protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapIterator {
    next_index: usize,
    /// Latched once the cursor reported completion: an exhausted iterator
    /// stays exhausted even if the collection grows afterwards.
    finished: bool,
    /// What each step yields: 0 the `[key, value]` entry, 1 the key, 2 the value.
    mode: u8,
}

impl MapIterator {
    pub fn new() -> Self {
        Self { next_index: 0, finished: false, mode: 0 }
    }

    /// A cursor yielding entries (0), keys (1) or values (2).
    pub fn with_mode(mode: u8) -> Self {
        Self { next_index: 0, finished: false, mode }
    }

    pub fn mode(&self) -> u8 {
        self.mode
    }

    pub fn next(&mut self, map: &OrderedMap) -> Option<(Any, Any)> {
        if self.finished {
            return None;
        }
        let Some((slot, key, value)) = map.physical_next(self.next_index) else {
            self.finished = true;
            return None;
        };
        self.next_index = slot + 1;
        Some((key, value))
    }

    /// Whether all currently live entries have been consumed.
    pub fn is_done(&self, map: &OrderedMap) -> bool {
        self.finished || map.physical_next(self.next_index).is_none()
    }

    /// Record that completion was observed.
    pub fn latch_done(&mut self) {
        self.finished = true;
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
    /// Latched once the cursor reported completion.
    finished: bool,
}

impl SetIterator {
    pub fn new() -> Self {
        Self { next_index: 0, finished: false }
    }

    pub fn next(&mut self, set: &OrderedMap) -> Option<Any> {
        if self.finished {
            return None;
        }
        let Some((slot, value, _)) = set.physical_next(self.next_index) else {
            self.finished = true;
            return None;
        };
        self.next_index = slot + 1;
        Some(value)
    }

    /// Whether all currently live values have been consumed.
    pub fn is_done(&self, set: &OrderedMap) -> bool {
        self.finished || set.physical_next(self.next_index).is_none()
    }

    /// Record that completion was observed.
    pub fn latch_done(&mut self) {
        self.finished = true;
    }
}

impl Default for SetIterator {
    fn default() -> Self {
        Self::new()
    }
}
