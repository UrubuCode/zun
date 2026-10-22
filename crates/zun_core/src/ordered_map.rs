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
    tombstones: Vec<Option<(Any, u64)>>,
    live_len: usize,
    dead_len: usize,
}

impl Default for OrderedMap {
    fn default() -> Self { Self::new() }
}

impl OrderedMap {
    pub fn new() -> Self {
        Self { entries: Vec::new(), buckets: vec![None; 8], tombstones: vec![None; 8], live_len: 0, dead_len: 0 }
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
        let (existing, insertion_slot) = self.find_insert_slot(hash, &equal);
        if let Some(index) = existing {
            return Some(self.entries[index].as_mut()
                .map(|entry| std::mem::replace(&mut entry.1, value)).unwrap());
        }
        let index = self.entries.len();
        self.entries.push(Some((key, value, hash)));
        self.buckets[insertion_slot] = Some(index);
        self.tombstones[insertion_slot] = None;
        self.live_len += 1;
        if self.live_len * 4 > self.buckets.len() * 3 { self.rehash(); }
        None
    }

    pub fn remove(&mut self, hash: u64, equal: impl Fn(Any) -> bool) -> Option<(Any, Any)> {
        let index = self.find_index(hash, equal)?;
        let removed = self.entries[index].take()?;
        let mask = self.buckets.len() - 1;
        let mut slot = removed.2 as usize & mask;
        for _ in 0..self.buckets.len() {
            if self.buckets[slot] == Some(index) {
                self.tombstones[slot] = Some((removed.0, removed.2));
                break;
            }
            slot = (slot + 1) & mask;
        }
        self.live_len -= 1;
        self.dead_len += 1;
        if self.live_len == 0 || (self.dead_len >= 8 && self.dead_len > self.live_len) {
            self.compact();
        }
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
        for tombstone in self.tombstones.iter_mut().flatten() {
            tombstone.0 = tombstone.0.map_object_address(map);
        }
        rewritten
    }

    pub fn entry_at(&self, position: usize) -> Option<(Any, Any)> {
        self.entries.iter().filter_map(|entry| entry.map(|(key, value, _)| (key, value))).nth(position)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.buckets = vec![None; 8];
        self.tombstones = vec![None; 8];
        self.live_len = 0;
        self.dead_len = 0;
    }

    fn rehash(&mut self) {
        self.rebuild_buckets(self.buckets.len() * 2);
    }

    fn rebuild_buckets(&mut self, capacity: usize) {
        let mut buckets = vec![None; capacity.max(8).next_power_of_two()];
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
        self.tombstones = vec![None; self.buckets.len()];
    }

    fn compact(&mut self) {
        self.entries = self.entries.iter().filter_map(|entry| *entry).map(Some).collect();
        self.dead_len = 0;
        let mut capacity = 8;
        while self.live_len * 4 > capacity * 3 { capacity *= 2; }
        self.rebuild_buckets(capacity);
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

    fn find_insert_slot(&self, hash: u64, equal: impl Fn(Any) -> bool) -> (Option<usize>, usize) {
        let mask = self.buckets.len() - 1;
        let mut slot = hash as usize & mask;
        let mut first_tombstone = None;
        loop {
            match self.buckets[slot] {
                None => {
                    return (None, first_tombstone.unwrap_or(slot));
                }
                Some(existing) if self.entries[existing].is_none() => {
                    if let Some((key, tombstone_hash)) = self.tombstones[slot] {
                        if tombstone_hash == hash && equal(key) {
                            return (None, slot);
                        }
                    }
                    first_tombstone.get_or_insert(slot);
                }
                Some(existing) if self.entries[existing].is_some_and(|(key, _, entry_hash)| {
                    entry_hash == hash && equal(key)
                }) => return (Some(existing), slot),
                Some(_) => {}
            }
            slot = (slot + 1) & mask;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OrderedMap;
    use crate::Any;

    #[test]
    fn compaction_shrinks_empty_tables_back_to_the_floor() {
        let mut map = OrderedMap::new();
        for key in 0..128i64 {
            map.insert(Any::int(key), Any::int(key), key as u64, |candidate| {
                candidate.as_int() == Some(key)
            });
        }
        for key in 0..128i64 {
            assert!(map.remove(key as u64, |candidate| candidate.as_int() == Some(key)).is_some());
        }
        assert!(map.is_empty());
        assert_eq!(map.buckets.len(), 8);
    }

    #[test]
    fn reinserting_removed_keys_reuses_tombstones() {
        let mut map = OrderedMap::new();
        for key in 0..64i64 {
            map.insert(Any::int(key), Any::int(key), key as u64, |candidate| {
                candidate.as_int() == Some(key)
            });
        }
        for key in (0..64i64).step_by(2) {
            assert!(map.remove(key as u64, |candidate| candidate.as_int() == Some(key)).is_some());
        }
        for key in (0..64i64).step_by(2) {
            assert!(map.insert(Any::int(key), Any::int(key + 1000), key as u64, |candidate| {
                candidate.as_int() == Some(key)
            }).is_none());
        }
        for key in 0..64i64 {
            assert_eq!(map.get(key as u64, |candidate| candidate.as_int() == Some(key)), Some(Any::int(if key % 2 == 0 { key + 1000 } else { key })));
        }
    }
}
