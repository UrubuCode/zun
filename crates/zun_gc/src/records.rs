#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Generation {
    /// Newly allocated objects are candidates for a future nursery collection.
    Young,
    /// Objects that survived a full collection are retained in the old set.
    Old,
}

#[derive(Debug)]
pub(super) struct Record {
    pub(super) addr: *mut u64,
    pub(super) field_count: usize,
    pub(super) alive: bool,
    pub(super) marked: bool,
    pub(super) generation: Generation,
}
