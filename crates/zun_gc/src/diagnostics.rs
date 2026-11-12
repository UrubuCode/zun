use super::Heap;

impl Heap {
    /// Number of currently live objects (for diagnostics and tests).
    pub fn live_count(&self) -> usize {
        self.records.iter().filter(|record| record.alive).count()
    }

    /// Number of objects' worth of chunks currently held (for diagnostics).
    pub fn chunk_count(&self) -> usize {
        self.chunks.len() + self.nursery_chunks.len()
    }

    /// Number of nursery chunks currently held. A collection releases the
    /// nursery, so this stays small however many objects a program allocates;
    /// growth here means nursery storage is being abandoned.
    pub fn nursery_chunk_count(&self) -> usize {
        self.nursery_chunks.len()
    }

    /// Length of the object record table, live and dead. Bounded by the peak
    /// live count, because a moving collection releases dead records' storage
    /// and makes their indices reusable at fresh addresses.
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Number of entries in the interned-string table, including handles no
    /// longer referenced by anything: the table is append-only (see
    /// `intern_string`), so this only grows.
    pub fn string_table_len(&self) -> usize {
        self.strings.len()
    }

    /// Objects allocated since the last collection. A safepoint reads this to
    /// decide whether to collect: counting at the single allocation point
    /// means every managed allocation is seen, not only the typed `newobj`
    /// the interpreter used to count for itself.
    pub fn allocations_since_collection(&self) -> usize {
        self.allocations
    }

    /// Bytes held by heap chunks, tenured and nursery. This is the committed
    /// object storage, not the live-object total.
    pub fn chunk_bytes(&self) -> usize {
        self.chunks
            .iter()
            .chain(self.nursery_chunks.iter())
            .map(|chunk| std::mem::size_of_val(&**chunk))
            .sum()
    }
}
