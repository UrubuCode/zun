//! The hash collections the heap keys by object address.
//!
//! Every side table of the heap is keyed by an address or a small integer,
//! and `std`'s default hasher is SipHash: DoS-resistant, and several times
//! slower than these keys need. No key here is attacker-chosen -- they are
//! addresses the allocator handed out -- so the resistance buys nothing, and
//! the cost is paid on every object access the runtime makes and, in a sweep,
//! once per table per dead object.
//!
//! This is the multiply-rotate scheme `rustc` uses for the same reason. The
//! final rotation matters for exactly these keys: addresses are multiples of
//! eight, a multiplication keeps their low bits zero, and the table picks its
//! bucket from the low bits.

use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0xf135_7aea_2e62_a9c5;

#[derive(Default, Clone, Copy)]
pub struct FastHasher {
    hash: u64,
}

impl FastHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.wrapping_add(word)).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.add(u64::from_le_bytes(chunk.try_into().unwrap_or([0; 8])));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(last));
        }
        // The length keeps a prefix from colliding with its extension.
        self.add(bytes.len() as u64);
    }

    #[inline]
    fn write_u8(&mut self, value: u8) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u32(&mut self, value: u32) {
        self.add(u64::from(value));
    }

    #[inline]
    fn write_u64(&mut self, value: u64) {
        self.add(value);
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.add(value as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash.rotate_left(26)
    }
}

pub type BuildFastHasher = BuildHasherDefault<FastHasher>;
pub type HashMap<K, V> = std::collections::HashMap<K, V, BuildFastHasher>;
pub type HashSet<K> = std::collections::HashSet<K, BuildFastHasher>;
