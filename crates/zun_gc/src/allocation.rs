use super::Heap;

impl Heap {
    pub fn new() -> Self {
        Self::default()
    }

    pub(super) fn allocate_identity_hash(&mut self) -> u64 {
        if self.next_identity_hash == 0 {
            self.next_identity_hash = 1;
        }
        let hash = self.next_identity_hash;
        self.next_identity_hash = self
            .next_identity_hash
            .checked_add(1)
            .expect("identity hash exhausted");
        hash
    }
}
