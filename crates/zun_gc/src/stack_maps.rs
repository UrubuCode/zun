//! Native/JIT stack-map contracts at the GC boundary.

use std::cell::RefCell;

thread_local! {
    static ACTIVE_REGISTRIES: RefCell<Vec<*const StackMapRegistry>> = const { RefCell::new(Vec::new()) };
}

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

/// Guard that publishes one JIT registry while its native entry point runs.
#[derive(Debug)]
pub struct ActiveStackMapScope {
    registry: *const StackMapRegistry,
}

impl ActiveStackMapScope {
    /// Register `registry` for the duration of a native call.
    ///
    /// # Safety
    /// `registry` must remain alive and unchanged until the returned guard is
    /// dropped.
    pub unsafe fn enter(registry: &StackMapRegistry) -> Self {
        let registry = registry as *const StackMapRegistry;
        ACTIVE_REGISTRIES.with(|active| active.borrow_mut().push(registry));
        Self { registry }
    }
}

impl Drop for ActiveStackMapScope {
    fn drop(&mut self) {
        ACTIVE_REGISTRIES.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(index) = active.iter().rposition(|&registry| registry == self.registry) {
                active.remove(index);
            }
        });
    }
}

/// Execute `f` with the innermost active JIT stack-map registry, if any.
pub fn with_active_stack_map_registry<R>(f: impl FnOnce(Option<&StackMapRegistry>) -> R) -> R {
    ACTIVE_REGISTRIES.with(|active| {
        let registry = active.borrow().last().copied();
        // SAFETY: every pointer is protected by an active scope whose caller
        // guarantees the registry outlives this callback.
        f(registry.map(|registry| unsafe { &*registry }))
    })
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
