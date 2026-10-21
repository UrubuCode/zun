use super::Any;

/// Contiguous storage contract for a managed array.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArrayStorage {
    elements: Vec<Any>,
}

impl ArrayStorage {
    /// Create an array with `length` initialized to `undefined`-like void values.
    pub fn with_length(length: usize) -> Self { Self { elements: vec![Any::int(0); length] } }
    pub fn len(&self) -> usize { self.elements.len() }
    pub fn is_empty(&self) -> bool { self.elements.is_empty() }
    pub fn get(&self, index: usize) -> Option<Any> { self.elements.get(index).copied() }

    pub fn set(&mut self, index: usize, value: Any) -> Option<Any> {
        let slot = self.elements.get_mut(index)?;
        Some(std::mem::replace(slot, value))
    }

    pub fn push(&mut self, value: Any) -> usize {
        let index = self.elements.len();
        self.elements.push(value);
        index
    }
    pub fn pop(&mut self) -> Option<Any> { self.elements.pop() }
    pub fn resize(&mut self, new_length: usize) { self.elements.resize(new_length, Any::int(0)); }

    pub fn insert(&mut self, index: usize, value: Any) -> bool {
        if index > self.elements.len() { return false; }
        self.elements.insert(index, value);
        true
    }

    pub fn remove(&mut self, index: usize) -> Option<Any> {
        (index < self.elements.len()).then(|| self.elements.remove(index))
    }
    pub fn iter(&self) -> impl Iterator<Item = Any> + '_ { self.elements.iter().copied() }
    pub fn object_references(&self) -> impl Iterator<Item = usize> + '_ {
        self.elements.iter().filter_map(|value| value.as_object())
    }

    pub fn rewrite_object_references(&mut self, map: impl Fn(usize) -> usize + Copy) -> usize {
        let mut rewritten = 0;
        for value in &mut self.elements {
            let mapped = value.map_object_address(map);
            if mapped != *value { rewritten += 1; *value = mapped; }
        }
        rewritten
    }
}

/// State for the default `Array[Symbol.iterator]()` protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayIterator {
    next_index: usize,
}

impl ArrayIterator {
    pub fn new() -> Self { Self { next_index: 0 } }
    pub fn next(&mut self, array: &ArrayStorage) -> Option<Any> {
        let value = array.get(self.next_index)?;
        self.next_index += 1;
        Some(value)
    }
    pub fn is_done(&self, array: &ArrayStorage) -> bool { self.next_index >= array.len() }
}

impl Default for ArrayIterator {
    fn default() -> Self { Self::new() }
}
