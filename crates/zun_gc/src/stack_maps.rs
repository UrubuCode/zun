//! Native/JIT stack-map contracts at the GC boundary.

/// A native safepoint description published by a code generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStackMap {
    pub pc: usize,
    pub span: u32,
    /// Offsets from the native stack pointer to managed references.
    pub reference_slots: Vec<u32>,
}

/// Registry of native safepoint maps known to the GC boundary.
#[derive(Debug, Default)]
pub struct StackMapRegistry {
    maps: Vec<NativeStackMap>,
}

impl StackMapRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replace(&mut self, maps: impl IntoIterator<Item = NativeStackMap>) {
        self.maps.clear();
        self.maps.extend(maps);
        self.maps.sort_unstable_by_key(|map| map.pc);
    }

    pub fn lookup(&self, pc: usize) -> Option<NativeStackMap> {
        let index = match self.maps.binary_search_by_key(&pc, |map| map.pc) {
            Ok(index) => index,
            Err(0) => return None,
            Err(index) => index - 1,
        };
        let map = self.maps[index].clone();
        let end = map.pc.saturating_add(map.span as usize);
        (pc < end).then_some(map)
    }

    pub fn len(&self) -> usize {
        self.maps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }

    /// Read managed references described by the safepoint covering `pc`.
    ///
    /// The caller must provide the native stack pointer for the same frame and
    /// guarantee that the frame is stopped at the safepoint. This is the
    /// runtime-facing primitive used by a future precise native-frame scan.
    pub unsafe fn roots_at(&self, pc: usize, sp: *const u8) -> Vec<super::Obj> {
        let Some(map) = self.lookup(pc) else {
            return Vec::new();
        };
        map.reference_slots
            .iter()
            .filter_map(|&offset| {
                let address = unsafe { sp.add(offset as usize) as *const u64 };
                let value = unsafe { address.read_unaligned() };
                (value != 0)
                    .then(|| unsafe { super::Obj::from_addr(value as *mut u64) })
            })
            .collect()
    }
}
