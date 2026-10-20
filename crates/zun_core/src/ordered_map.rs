use super::Any;

/// An insertion-ordered hash table used by the runtime-backed Map/Set types.
///
/// Hashing and SameValueZero equality are supplied by the runtime because
/// object keys need the GC-owned identity hash. Entries remain in insertion
/// order independently of bucket layout.
#[derive(Debug, Clone)]
pub struct OrderedMap {
    entries: Vec<Option<(Any, Any, u64)>>,
    buckets: Vec<Vec<usize>>,
    live_len: usize,
    dead_len: usize,
}

impl Default for OrderedMap {
    fn default() -> Self { Self::new() }
}

impl OrderedMap {
    pub fn new() -> Self {
        Self { entries: Vec::new(), buckets: vec![Vec::new(); 8], live_len: 0, dead_len: 0 }
    }

    pub fn len(&self) -> usize { self.live_len }
    pub fn is_empty(&self) -> bool { self.live_len == 0 }

    pub fn get(&self, hash: u64, equal: impl Fn(Any) -> bool) -> Option<Any> {
        let bucket = &self.buckets[(hash as usize) % self.buckets.len()];
        bucket.iter().find_map(|&index| {
            self.entries[index].and_then(|(key, value, _)| equal(key).then_some(value))
        })
    }

    pub fn contains(&self, hash: u64, equal: impl Fn(Any) -> bool) -> bool {
        self.get(hash, equal).is_some()
    }

    pub fn insert(&mut self, key: Any, value: Any, hash: u64, equal: impl Fn(Any) -> bool) -> Option<Any> {
        if self.dead_len >= 8 && self.dead_len > self.live_len { self.compact(); }
        let bucket_index = (hash as usize) % self.buckets.len();
        if let Some(&index) = self.buckets[bucket_index]
            .iter().find(|&&index| self.entries[index].is_some_and(|(key, _, _)| equal(key)))
        {
            return Some(self.entries[index].as_mut()
                .map(|entry| std::mem::replace(&mut entry.1, value)).unwrap());
        }
        let index = self.entries.len();
        self.entries.push(Some((key, value, hash)));
        self.buckets[bucket_index].push(index);
        self.live_len += 1;
        if self.live_len * 4 > self.buckets.len() * 3 { self.rehash(); }
        None
    }

    pub fn remove(&mut self, hash: u64, equal: impl Fn(Any) -> bool) -> Option<(Any, Any)> {
        let bucket_index = (hash as usize) % self.buckets.len();
        let &index = self.buckets[bucket_index]
            .iter().find(|&&index| self.entries[index].is_some_and(|(key, _, _)| equal(key)))?;
        let removed = self.entries[index].take()?;
        self.live_len -= 1;
        self.dead_len += 1;
        Some((removed.0, removed.1))
    }

    pub fn iter(&self) -> impl Iterator<Item = (Any, Any)> + '_ {
        self.entries.iter().filter_map(|entry| entry.map(|(key, value, _)| (key, value)))
    }

    pub fn entry_at(&self, position: usize) -> Option<(Any, Any)> {
        self.entries.iter().filter_map(|entry| entry.map(|(key, value, _)| (key, value))).nth(position)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.buckets = vec![Vec::new(); 8];
        self.live_len = 0;
        self.dead_len = 0;
    }

    fn rehash(&mut self) {
        let mut buckets = vec![Vec::new(); self.buckets.len() * 2];
        let bucket_count = buckets.len();
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some((_, _, hash)) = entry { buckets[(*hash as usize) % bucket_count].push(index); }
        }
        self.buckets = buckets;
    }

    fn compact(&mut self) {
        self.entries = self.entries.iter().filter_map(|entry| *entry).map(Some).collect();
        self.dead_len = 0;
        self.rehash();
    }
}
