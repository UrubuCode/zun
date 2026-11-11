use super::Any;
use std::collections::BTreeMap;

/// Largest dense prefix an array grows to eagerly. Growth past this (for
/// example `a[4294967294] = x` or `a.length = 4294967295`) keeps the extra
/// length virtual and stores far elements in a sparse map, so a huge length
/// never costs a huge allocation.
const DENSE_LIMIT: usize = 1 << 22;

/// Contiguous storage contract for a managed array: a dense prefix plus a
/// sparse tail for far-away indices. Every slot not in either is a hole.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArrayStorage {
    elements: Vec<Any>,
    /// Elements at indices `>= elements.len()` (never holes).
    sparse: BTreeMap<usize, Any>,
    /// Logical length; always `>= elements.len()`.
    length: usize,
    /// Integrity level: 0 open, 1 non-extensible, 2 sealed, 3 frozen.
    integrity: u8,
    /// Set on the array that backs a function's `arguments` object.
    is_arguments: bool,
}

impl ArrayStorage {
    /// Create an array with `length` initialized to `undefined`-like void values.
    pub fn with_length(length: usize) -> Self {
        let mut array = Self::default();
        array.resize(length);
        array
    }
    pub fn is_arguments(&self) -> bool {
        self.is_arguments
    }
    pub fn mark_arguments(&mut self) {
        self.is_arguments = true;
    }
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn integrity(&self) -> u8 {
        self.integrity
    }
    pub fn raise_integrity(&mut self, level: u8) {
        if level > self.integrity {
            self.integrity = level;
        }
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    /// Whether any slot lives outside the dense prefix (the array is sparse).
    pub fn is_sparse(&self) -> bool {
        self.length > self.elements.len()
    }
    /// Indices of every non-hole slot, ascending, without walking holes.
    pub fn present_indices(&self) -> Vec<usize> {
        let mut indices: Vec<usize> = self
            .elements
            .iter()
            .enumerate()
            .filter(|(_, value)| **value != Any::hole())
            .map(|(index, _)| index)
            .collect();
        indices.extend(self.sparse.keys().copied());
        indices
    }
    pub fn get(&self, index: usize) -> Option<Any> {
        if let Some(value) = self.elements.get(index) {
            return Some(*value);
        }
        if index < self.length {
            return Some(self.sparse.get(&index).copied().unwrap_or_else(Any::hole));
        }
        None
    }

    pub fn set(&mut self, index: usize, value: Any) -> Option<Any> {
        if index >= self.length {
            return None;
        }
        if let Some(slot) = self.elements.get_mut(index) {
            return Some(std::mem::replace(slot, value));
        }
        if self.sparse.is_empty() && index < DENSE_LIMIT {
            self.elements.resize(index + 1, Any::hole());
            let slot = &mut self.elements[index];
            return Some(std::mem::replace(slot, value));
        }
        Some(self.sparse.insert(index, value).unwrap_or_else(Any::hole))
    }

    pub fn push(&mut self, value: Any) -> usize {
        let index = self.length;
        self.length += 1;
        if index == self.elements.len() {
            self.elements.push(value);
        } else {
            self.sparse.insert(index, value);
        }
        index
    }
    pub fn pop(&mut self) -> Option<Any> {
        if self.length == 0 {
            return None;
        }
        self.length -= 1;
        if self.length < self.elements.len() {
            self.elements.pop()
        } else {
            Some(self.sparse.remove(&self.length).unwrap_or_else(Any::hole))
        }
    }
    pub fn resize(&mut self, new_length: usize) {
        if new_length < self.length {
            self.elements.truncate(new_length);
            self.sparse.split_off(&new_length);
        } else if self.sparse.is_empty() && new_length <= DENSE_LIMIT {
            self.elements.resize(new_length, Any::hole());
        }
        self.length = new_length;
    }

    pub fn insert(&mut self, index: usize, value: Any) -> bool {
        if index > self.length {
            return false;
        }
        if !self.is_sparse() {
            self.elements.insert(index, value);
            self.length += 1;
            return true;
        }
        // Sparse: re-key every slot at or after `index` one step up.
        let dense_tail = if index < self.elements.len() {
            self.elements.split_off(index)
        } else {
            Vec::new()
        };
        let tail = self.sparse.split_off(&index);
        let moved: Vec<(usize, Any)> = dense_tail
            .into_iter()
            .enumerate()
            .filter(|(_, v)| *v != Any::hole())
            .map(|(offset, v)| (index + offset, v))
            .chain(tail)
            .collect();
        self.length += 1;
        self.sparse.insert(index, value);
        for (key, v) in moved {
            self.sparse.insert(key + 1, v);
        }
        true
    }

    pub fn remove(&mut self, index: usize) -> Option<Any> {
        if index >= self.length {
            return None;
        }
        if !self.is_sparse() {
            self.length -= 1;
            return Some(self.elements.remove(index));
        }
        let removed = self.get(index);
        let dense_tail = if index < self.elements.len() {
            self.elements.split_off(index)
        } else {
            Vec::new()
        };
        let mut tail = self.sparse.split_off(&index);
        tail.remove(&index);
        let moved: Vec<(usize, Any)> = dense_tail
            .into_iter()
            .enumerate()
            .skip(1)
            .filter(|(_, v)| *v != Any::hole())
            .map(|(offset, v)| (index + offset, v))
            .chain(tail)
            .collect();
        self.length -= 1;
        for (key, v) in moved {
            self.sparse.insert(key - 1, v);
        }
        removed
    }
    pub fn iter(&self) -> impl Iterator<Item = Any> + '_ {
        (0..self.length).map(|index| self.get(index).unwrap_or_else(Any::hole))
    }
    pub fn object_references(&self) -> impl Iterator<Item = usize> + '_ {
        self.elements
            .iter()
            .chain(self.sparse.values())
            .filter_map(|value| value.as_object())
    }

    pub fn rewrite_object_references(&mut self, map: impl Fn(usize) -> usize + Copy) -> usize {
        let mut rewritten = 0;
        for value in self.elements.iter_mut().chain(self.sparse.values_mut()) {
            let mapped = value.map_object_address(map);
            if mapped != *value {
                rewritten += 1;
                *value = mapped;
            }
        }
        rewritten
    }
}

/// State for the default `Array[Symbol.iterator]()` protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayIterator {
    next_index: usize,
    /// 0 = values, 1 = keys, 2 = entries.
    mode: u8,
}

impl ArrayIterator {
    pub fn new() -> Self {
        Self {
            next_index: 0,
            mode: 0,
        }
    }
    pub fn with_mode(mode: u8) -> Self {
        Self {
            next_index: 0,
            mode,
        }
    }
    pub fn mode(&self) -> u8 {
        self.mode
    }
    /// Read the element at the cursor and advance, returning its index too.
    pub fn advance(&mut self, array: &ArrayStorage) -> Option<(usize, Any)> {
        let index = self.next_index;
        let value = array.get(index)?;
        self.next_index += 1;
        Some((index, value))
    }
    pub fn next(&mut self, array: &ArrayStorage) -> Option<Any> {
        let value = array.get(self.next_index)?;
        self.next_index += 1;
        Some(value)
    }
    pub fn is_done(&self, array: &ArrayStorage) -> bool {
        self.next_index >= array.len()
    }
}

impl Default for ArrayIterator {
    fn default() -> Self {
        Self::new()
    }
}
