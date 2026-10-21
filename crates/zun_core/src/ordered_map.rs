use super::Any;

/// An insertion-ordered hash table used by the runtime-backed Map/Set types.
///
/// Hashing and SameValueZero equality are supplied by the runtime because
/// object keys need the GC-owned identity hash. Entries remain in insertion
/// order independently of bucket layout.
#[derive(Debug, Clone)]
pub struct OrderedMap {
    entries: Vec<Option<(Any, Any, u64)>>,
    buckets: Vec<Option<usize>>,
    live_len: usize,
    dead_len: usize,
}

impl Default for OrderedMap {
    fn default() -> Self { Self::new() }
}

impl OrderedMap {
    pub fn new() -> Self {
        Self { entries: Vec::new(), buckets: vec![None; 8], live_len: 0, dead_len: 0 }
    }

    pub fn len(&self) -> usize { self.live_len }
    pub fn is_empty(&self) -> bool { self.live_len == 0 }

    pub fn get(&self, hash: u64, equal: impl Fn(Any) -> bool) -> Option<Any> {
        self.find_index(hash, equal)
            .and_then(|index| self.entries[index].map(|(_, value, _)| value))
    }

    pub fn contains(&self, hash: u64, equal: impl Fn(Any) -> bool) -> bool {
        self.get(hash, equal).is_some()
    }

    pub fn insert(&mut self, key: Any, value: Any, hash: u64, equal: impl Fn(Any) -> bool) -> Option<Any> {
        if self.dead_len >= 8 && self.dead_len > self.live_len { self.compact(); }
        if let Some(index) = self.find_index(hash, &equal) {
            return Some(self.entries[index].as_mut()
                .map(|entry| std::mem::replace(&mut entry.1, value)).unwrap());
        }
        let index = self.entries.len();
        self.entries.push(Some((key, value, hash)));
        self.insert_bucket(index, hash);
        self.live_len += 1;
        if self.live_len * 4 > self.buckets.len() * 3 { self.rehash(); }
        None
    }

    pub fn remove(&mut self, hash: u64, equal: impl Fn(Any) -> bool) -> Option<(Any, Any)> {
        let index = self.find_index(hash, equal)?;
        let removed = self.entries[index].take()?;
        self.live_len -= 1;
        self.dead_len += 1;
        Some((removed.0, removed.1))
    }

    pub fn iter(&self) -> impl Iterator<Item = (Any, Any)> + '_ {
        self.entries.iter().filter_map(|entry| entry.map(|(key, value, _)| (key, value)))
    }

    pub fn rewrite_object_references(&mut self, map: impl Fn(usize) -> usize + Copy) -> usize {
        let mut rewritten = 0;
        for entry in self.entries.iter_mut().flatten() {
            let key = entry.0.map_object_address(map);
            let value = entry.1.map_object_address(map);
            if key != entry.0 { rewritten += 1; entry.0 = key; }
            if value != entry.1 { rewritten += 1; entry.1 = value; }
        }
        rewritten
    }

    pub fn entry_at(&self, position: usize) -> Option<(Any, Any)> {
        self.entries.iter().filter_map(|entry| entry.map(|(key, value, _)| (key, value))).nth(position)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.buckets = vec![None; 8];
        self.live_len = 0;
        self.dead_len = 0;
    }

    fn rehash(&mut self) {
        let mut buckets = vec![None; self.buckets.len() * 2];
        let mask = buckets.len() - 1;
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some((_, _, hash)) = entry {
                let mut slot = *hash as usize & mask;
                while buckets[slot].is_some() {
                    slot = (slot + 1) & mask;
                }
                buckets[slot] = Some(index);
            }
        }
        self.buckets = buckets;
    }

    fn compact(&mut self) {
        self.entries = self.entries.iter().filter_map(|entry| *entry).map(Some).collect();
        self.dead_len = 0;
        self.rehash();
    }

    fn find_index(&self, hash: u64, equal: impl Fn(Any) -> bool) -> Option<usize> {
        let mask = self.buckets.len() - 1;
        let mut slot = hash as usize & mask;
        for _ in 0..self.buckets.len() {
            match self.buckets[slot] {
                None => return None,
                Some(index) => {
                    if self.entries[index].is_some_and(|(key, _, entry_hash)| {
                        entry_hash == hash && equal(key)
                    }) {
                        return Some(index);
                    }
                }
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    fn insert_bucket(&mut self, index: usize, hash: u64) {
        let mask = self.buckets.len() - 1;
        let mut slot = hash as usize & mask;
        loop {
            match self.buckets[slot] {
                None => {
                    self.buckets[slot] = Some(index);
                    return;
                }
                Some(existing) if self.entries[existing].is_none() => {
                    self.buckets[slot] = Some(index);
                    return;
                }
                Some(_) => slot = (slot + 1) & mask,
            }
        }
    }
}
