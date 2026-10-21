//! Native/JIT stack-map contracts at the GC boundary.

use std::cell::RefCell;
use super::RelocationMap;

thread_local! {
    static ACTIVE_REGISTRIES: RefCell<Vec<*const StackMapRegistry>> = const { RefCell::new(Vec::new()) };
    static ACTIVE_FRAMES: RefCell<Vec<ActiveNativeFrame>> = const { RefCell::new(Vec::new()) };
}

/// A native safepoint description published by a code generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStackMap {
    pub pc: usize,
    pub span: u32,
    /// Offsets from the native stack pointer to managed references.
    pub reference_slots: Vec<u32>,
}

/// A native frame currently stopped at a safepoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveNativeFrame {
    pub registry: *const StackMapRegistry,
    pub pc: usize,
    pub sp: *mut u8,
}

/// RAII registration for one stopped native frame.
#[derive(Debug)]
pub struct ActiveNativeFrameScope {
    frame: ActiveNativeFrame,
}

impl ActiveNativeFrameScope {
    /// Register a frame whose registry and stack pointer remain valid until
    /// the returned scope is dropped.
    pub unsafe fn enter(registry: &StackMapRegistry, pc: usize, sp: *mut u8) -> Self {
        let frame = ActiveNativeFrame { registry: registry as *const _, pc, sp };
        ACTIVE_FRAMES.with(|frames| frames.borrow_mut().push(frame));
        Self { frame }
    }
}

impl Drop for ActiveNativeFrameScope {
    fn drop(&mut self) {
        ACTIVE_FRAMES.with(|frames| {
            let mut frames = frames.borrow_mut();
            if let Some(index) = frames.iter().rposition(|frame| *frame == self.frame) {
                frames.remove(index);
            }
        });
    }
}

/// Enumerate frames registered on the current thread.
pub fn with_active_native_frames<R>(f: impl FnOnce(&[ActiveNativeFrame]) -> R) -> R {
    ACTIVE_FRAMES.with(|frames| f(&frames.borrow()))
}

/// Registry of native safepoint maps known to the GC boundary.
#[derive(Debug, Default)]
pub struct StackMapRegistry {
    maps: Vec<NativeStackMap>,
}

#[cfg(test)]
mod tests {
    use super::{with_active_native_frames, ActiveNativeFrameScope, NativeStackMap, StackMapRegistry};
    use crate::RelocationMap;

    #[test]
    fn rewrites_reference_slots_at_a_stopped_safepoint() {
        let mut registry = StackMapRegistry::new();
        registry.replace([NativeStackMap { pc: 10, span: 4, reference_slots: vec![8] }]);
        let mut frame = [0u8; 16];
        let old = 0x1000usize;
        let new = 0x2000usize;
        unsafe { (frame.as_mut_ptr().add(8) as *mut u64).write_unaligned(old as u64); }
        let mut relocation = RelocationMap::new();
        relocation.insert_addresses(old, new);

        assert_eq!(unsafe { registry.rewrite_slots_at(10, frame.as_mut_ptr(), &relocation) }, 1);
        assert_eq!(unsafe { (frame.as_ptr().add(8) as *const u64).read_unaligned() }, new as u64);
    }

    #[test]
    fn tracks_nested_stopped_native_frames_per_thread() {
        let registry = StackMapRegistry::new();
        let mut frame = [0u8; 8];
        let outer = unsafe { ActiveNativeFrameScope::enter(&registry, 10, frame.as_mut_ptr()) };
        assert_eq!(with_active_native_frames(|frames| frames.len()), 1);
        let inner = unsafe { ActiveNativeFrameScope::enter(&registry, 20, frame.as_mut_ptr()) };
        assert_eq!(with_active_native_frames(|frames| frames.iter().map(|f| f.pc).collect::<Vec<_>>()), vec![10, 20]);
        drop(inner);
        assert_eq!(with_active_native_frames(|frames| frames[0].pc), 10);
        drop(outer);
        assert_eq!(with_active_native_frames(|frames| frames.len()), 0);
    }
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

    /// Rewrite managed references in the stopped native frame at `pc`.
    ///
    /// # Safety
    /// `sp` must identify a stopped frame covered by this registry, and every
    /// stack-map slot must be a valid writable reference slot.
    pub unsafe fn rewrite_slots_at(
        &self,
        pc: usize,
        sp: *mut u8,
        relocation: &RelocationMap,
    ) -> usize {
        let Some(map) = self.lookup(pc) else { return 0; };
        map.reference_slots
            .iter()
            .filter(|&&offset| unsafe {
                relocation.rewrite_slot(sp.add(offset as usize) as *mut u64)
            })
            .count()
    }
}
