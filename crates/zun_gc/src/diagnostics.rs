use super::Heap;

impl Heap {
    /// Number of currently live objects (for diagnostics and tests).
    pub fn live_count(&self) -> usize {
        self.records.iter().filter(|record| record.alive).count()
    }

    /// Number of objects' worth of chunks currently held (for diagnostics).
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}
