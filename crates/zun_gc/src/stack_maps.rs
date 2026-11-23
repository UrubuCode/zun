//! Native/JIT stack-map contracts at the GC boundary.

use super::RelocationMap;
use std::cell::RefCell;

thread_local! {
    static ACTIVATIONS: RefCell<Vec<Activation>> = const { RefCell::new(Vec::new()) };
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
        let frame = ActiveNativeFrame {
            registry: registry as *const _,
            pc,
            sp,
        };
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
    /// Sorted `[start, end)` ranges of compiled function bodies.
    compiled_code: Vec<(usize, usize)>,
    /// Sorted `[start, end)` ranges of host-entry stubs.
    entry_code: Vec<(usize, usize)>,
}

fn range_contains(ranges: &[(usize, usize)], pc: usize) -> bool {
    let index = ranges.partition_point(|&(start, _)| start <= pc);
    index > 0 && pc < ranges[index - 1].1
}

#[cfg(test)]
mod tests {
    use super::{
        with_active_native_frames, ActiveNativeFrameScope, NativeStackMap, StackMapRegistry,
    };
    use crate::RelocationMap;

    #[test]
    fn rewrites_reference_slots_at_a_stopped_safepoint() {
        let mut registry = StackMapRegistry::new();
        registry.replace([NativeStackMap {
            pc: 10,
            span: 4,
            reference_slots: vec![8],
        }]);
        let mut frame = [0u8; 16];
        let old = 0x1000usize;
        let new = 0x2000usize;
        unsafe {
            (frame.as_mut_ptr().add(8) as *mut u64).write_unaligned(old as u64);
        }
        let mut relocation = RelocationMap::new();
        relocation.insert_addresses(old, new);

        assert_eq!(
            unsafe { registry.rewrite_slots_at(10, frame.as_mut_ptr(), &relocation) },
            1
        );
        assert_eq!(
            unsafe { (frame.as_ptr().add(8) as *const u64).read_unaligned() },
            new as u64
        );
    }

    #[test]
    fn tracks_nested_stopped_native_frames_per_thread() {
        let registry = StackMapRegistry::new();
        let mut frame = [0u8; 8];
        let outer = unsafe { ActiveNativeFrameScope::enter(&registry, 10, frame.as_mut_ptr()) };
        assert_eq!(with_active_native_frames(|frames| frames.len()), 1);
        let inner = unsafe { ActiveNativeFrameScope::enter(&registry, 20, frame.as_mut_ptr()) };
        assert_eq!(
            with_active_native_frames(|frames| frames.iter().map(|f| f.pc).collect::<Vec<_>>()),
            vec![10, 20]
        );
        drop(inner);
        assert_eq!(with_active_native_frames(|frames| frames[0].pc), 10);
        drop(outer);
        assert_eq!(with_active_native_frames(|frames| frames.len()), 0);
    }
}

/// The innermost compiled frame of an activation, stopped inside a call that
/// can reach a collection: `pc` is the call's return address (the key of its
/// safepoint), `sp` the stack pointer at the call instruction, and `fp` the
/// frame pointer of the compiled function that made the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuspendedFrame {
    pub pc: usize,
    pub sp: *mut u8,
    pub fp: usize,
}

/// One host entry into compiled code: the registry that describes it and, while
/// the activation is suspended in a call out of compiled code (an allocation
/// slow path, a bridge into the interpreter), its innermost compiled frame.
///
/// A chain of compiled frames ends at the host entry that called into compiled
/// code, so one activation owns exactly one frame-pointer chain. A nested
/// activation (compiled, bridge, interpreter, compiled again) is a further
/// record on the stack; the chains never run through the Rust frames between.
#[derive(Debug, Clone, Copy)]
struct Activation {
    registry: *const StackMapRegistry,
    suspended: Option<SuspendedFrame>,
}

/// Guard that publishes one JIT registry while its native entry point runs.
///
/// This is the host-entry boundary: every Rust-to-compiled call opens one, so
/// the activation stack mirrors the nesting of compiled code on this thread.
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
        ACTIVATIONS.with(|active| {
            active.borrow_mut().push(Activation {
                registry,
                suspended: None,
            })
        });
        Self { registry }
    }
}

impl Drop for ActiveStackMapScope {
    fn drop(&mut self) {
        ACTIVATIONS.with(|active| {
            let mut active = active.borrow_mut();
            if let Some(index) = active
                .iter()
                .rposition(|activation| activation.registry == self.registry)
            {
                active.remove(index);
            }
        });
    }
}

/// Execute `f` with the innermost active JIT stack-map registry, if any.
pub fn with_active_stack_map_registry<R>(f: impl FnOnce(Option<&StackMapRegistry>) -> R) -> R {
    ACTIVATIONS.with(|active| {
        let registry = active.borrow().last().map(|activation| activation.registry);
        // SAFETY: every pointer is protected by an active scope whose caller
        // guarantees the registry outlives this callback.
        f(registry.map(|registry| unsafe { &*registry }))
    })
}

/// Publishes the innermost compiled frame of the current activation for as long
/// as the compiled code is suspended in a call that can collect.
///
/// A collection then scans that frame and every compiled caller up its frame
/// pointer chain, from the shared root entry points (`append_native_roots` and
/// `rewrite_native_roots`), so the allocation slow path and the interpreter run
/// by a bridge see the same frames.
#[derive(Debug)]
pub struct SuspendedFrameScope {
    /// `Some(previous)` when a frame was published; `None` when no activation
    /// was open (nothing to restore).
    previous: Option<Option<SuspendedFrame>>,
}

impl SuspendedFrameScope {
    /// Publish `frame` as the current activation's innermost compiled frame.
    ///
    /// # Safety
    /// `frame` must describe a compiled frame that stays stopped, with its stack
    /// memory valid, until the returned guard is dropped.
    pub unsafe fn enter(frame: SuspendedFrame) -> Self {
        let previous = ACTIVATIONS.with(|active| {
            active
                .borrow_mut()
                .last_mut()
                .map(|activation| activation.suspended.replace(frame))
        });
        Self { previous }
    }

    /// Publish the frame of a compiled function that is calling out, given the
    /// stack pointer just before the call instruction and the function's frame
    /// pointer. The return address the call pushes sits at `caller_sp - 8`.
    ///
    /// # Safety
    /// Same contract as [`Self::enter`]; `caller_sp` must be the stack pointer
    /// read immediately before the call that is now in progress.
    pub unsafe fn enter_at_call(caller_sp: usize, caller_fp: usize) -> Self {
        let pc = unsafe { return_pc_before_call(caller_sp) };
        unsafe {
            Self::enter(SuspendedFrame {
                pc,
                sp: caller_sp as *mut u8,
                fp: caller_fp,
            })
        }
    }
}

impl Drop for SuspendedFrameScope {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            ACTIVATIONS.with(|active| {
                if let Some(activation) = active.borrow_mut().last_mut() {
                    activation.suspended = previous;
                }
            });
        }
    }
}

/// The return address a call pushes: at `caller_sp - 8` on x86-64, where
/// `caller_sp` is the stack pointer immediately before the call instruction.
///
/// # Safety
/// `caller_sp` must be that stack pointer for a call in progress.
#[inline]
pub unsafe fn return_pc_before_call(caller_sp: usize) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { (caller_sp as *const usize).sub(1).read_unaligned() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = caller_sp;
        0
    }
}

/// Visit every compiled frame of one activation, innermost first, as
/// `(registry, pc, sp)`.
///
/// With the managed calling convention and `preserve_frame_pointers`, a compiled
/// function's frame starts `push rbp; mov rbp, rsp`: `[fp]` is its caller's
/// frame pointer and `[fp + 8]` the return address into the caller. That return
/// address keys the caller's safepoint, and the caller's stack pointer at the
/// call instruction is `fp + 16` (the call pushed the return address, the
/// prologue pushed the frame pointer). The walk stops at the first return
/// address outside compiled code: that is the host entry, and no Rust frame is
/// ever walked.
unsafe fn walk_activation(
    activation: &Activation,
    mut visit: impl FnMut(&StackMapRegistry, usize, *mut u8),
) {
    let Some(top) = activation.suspended else {
        return;
    };
    let registry = unsafe { &*activation.registry };
    visit(registry, top.pc, top.sp);
    #[cfg(target_arch = "x86_64")]
    {
        let mut fp = top.fp;
        // A chain is at most as long as the number of compiled frames, which the
        // stack bounds; the cap only turns a corrupt chain into a loud failure.
        let mut remaining = 1usize << 24;
        while fp != 0 && remaining > 0 {
            remaining -= 1;
            let frame = fp as *const usize;
            let return_pc = unsafe { frame.add(1).read_unaligned() };
            if !registry.is_compiled_code(return_pc) {
                // The host entry. In a debug build, anything that is neither
                // compiled code nor a known entry stub means the chain was
                // followed off the managed frames: surface it.
                debug_assert!(
                    registry.is_entry_code(return_pc) || !registry.has_code_ranges(),
                    "frame walk left compiled code at {return_pc:#x}, which is not a host entry"
                );
                break;
            }
            let caller_fp = unsafe { frame.read_unaligned() };
            visit(registry, return_pc, (fp + 16) as *mut u8);
            fp = caller_fp;
        }
        debug_assert!(remaining > 0, "compiled frame chain did not terminate");
    }
}

/// Append the managed references held by every suspended compiled frame of
/// every activation on this thread.
pub(crate) fn append_compiled_frame_roots(roots: &mut Vec<super::Obj>) {
    // Snapshot so the walk does not hold the borrow while reading stack memory.
    let activations: Vec<Activation> = ACTIVATIONS.with(|active| active.borrow().clone());
    for activation in &activations {
        unsafe {
            walk_activation(activation, |registry, pc, sp| {
                roots.extend(registry.roots_at(pc, sp));
            });
        }
    }
}

/// Rewrite the managed references held by every suspended compiled frame of
/// every activation on this thread.
///
/// # Safety
/// Every published frame must still be stopped, as
/// [`SuspendedFrameScope::enter`] requires.
pub(crate) unsafe fn rewrite_compiled_frame_roots(relocation: &RelocationMap) -> usize {
    let activations: Vec<Activation> = ACTIVATIONS.with(|active| active.borrow().clone());
    let mut rewritten = 0;
    for activation in &activations {
        unsafe {
            walk_activation(activation, |registry, pc, sp| {
                rewritten += registry.rewrite_slots_at(pc, sp, relocation);
            });
        }
    }
    rewritten
}

/// The `(pc, sp)` of every compiled frame the current thread's activations
/// have published, each activation innermost frame first. For tests and
/// diagnostics.
pub fn suspended_frames() -> Vec<(usize, usize)> {
    let activations: Vec<Activation> = ACTIVATIONS.with(|active| active.borrow().clone());
    let mut frames = Vec::new();
    for activation in &activations {
        unsafe {
            walk_activation(activation, |_, pc, sp| frames.push((pc, sp as usize)));
        }
    }
    frames
}

impl StackMapRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare the address ranges of compiled managed code and of the host-entry
    /// stubs that call into it. A frame-pointer walk climbs through `compiled`
    /// return addresses and stops at an `entry` one.
    pub fn set_code_ranges(
        &mut self,
        compiled: impl IntoIterator<Item = (usize, usize)>,
        entry: impl IntoIterator<Item = (usize, usize)>,
    ) {
        self.compiled_code = compiled.into_iter().collect();
        self.compiled_code.sort_unstable();
        self.entry_code = entry.into_iter().collect();
        self.entry_code.sort_unstable();
    }

    /// Whether `pc` lies in compiled managed code (a function body).
    pub fn is_compiled_code(&self, pc: usize) -> bool {
        range_contains(&self.compiled_code, pc)
    }

    /// Whether `pc` lies in a host-entry stub, the end of every frame chain.
    pub fn is_entry_code(&self, pc: usize) -> bool {
        range_contains(&self.entry_code, pc)
    }

    /// Whether any code range was declared (a registry built by hand for a test
    /// declares none, and its frames are described by `(pc, sp)` alone).
    pub fn has_code_ranges(&self) -> bool {
        !self.compiled_code.is_empty() || !self.entry_code.is_empty()
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
                (value != 0).then(|| unsafe { super::Obj::from_addr(value as *mut u64) })
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
        let Some(map) = self.lookup(pc) else {
            return 0;
        };
        map.reference_slots
            .iter()
            .filter(|&&offset| unsafe {
                relocation.rewrite_slot(sp.add(offset as usize) as *mut u64)
            })
            .count()
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
mod frame_walk_tests {
    use super::{
        suspended_frames, ActiveStackMapScope, NativeStackMap, StackMapRegistry, SuspendedFrame,
        SuspendedFrameScope,
    };
    use crate::{append_native_roots, rewrite_native_roots, Obj, RelocationMap};

    const COMPILED_A: usize = 0x1000;
    const COMPILED_B: usize = 0x2000;
    const ENTRY: usize = 0x9000;

    /// Two compiled frames stacked on a host entry, laid out the way a managed
    /// call chain lays them out (`push rbp; mov rbp, rsp` prologue):
    ///
    /// ```text
    ///  word 0   B: saved rbp (= A's fp)        <- fp_B
    ///  word 1   B: return address into A
    ///  word 2   A: reference slot 0            <- A's sp at the call (= fp_B + 16)
    ///  word 3   A: reference slot 1
    ///  word 4   A: saved rbp (= 0)             <- fp_A
    ///  word 5   A: return address into the host entry
    /// ```
    /// B itself is the published top frame: its slot lives below `fp_B`.
    struct Chain {
        words: [usize; 8],
        registry: StackMapRegistry,
    }

    impl Chain {
        fn new(ref_a0: usize, ref_a1: usize, ref_b: usize) -> Box<Self> {
            let mut chain = Box::new(Chain {
                words: [0; 8],
                registry: StackMapRegistry::new(),
            });
            let base = chain.words.as_mut_ptr() as usize;
            // B's own slot sits in the word below fp_B (index 7 of the array is
            // reused as that slot: the "stack" here only needs consistent offsets).
            let fp_b = base;
            let fp_a = base + 4 * 8;
            chain.words[0] = fp_a;
            chain.words[1] = COMPILED_A + 0x10;
            chain.words[2] = ref_a0;
            chain.words[3] = ref_a1;
            chain.words[4] = 0;
            chain.words[5] = ENTRY + 0x4;
            chain.words[6] = ref_b;
            chain.registry.replace([
                // A's call site: slots at sp+0 and sp+8.
                NativeStackMap {
                    pc: COMPILED_A + 0x10,
                    span: 1,
                    reference_slots: vec![0, 8],
                },
                // B's own safepoint, with its slot at its sp (words[6]).
                NativeStackMap {
                    pc: COMPILED_B + 0x20,
                    span: 1,
                    reference_slots: vec![0],
                },
            ]);
            chain.registry.set_code_ranges(
                [(COMPILED_A, COMPILED_A + 0x100), (COMPILED_B, COMPILED_B + 0x100)],
                [(ENTRY, ENTRY + 0x100)],
            );
            let _ = fp_b;
            chain
        }

        fn top(&mut self) -> SuspendedFrame {
            let base = self.words.as_mut_ptr() as usize;
            SuspendedFrame {
                pc: COMPILED_B + 0x20,
                sp: (base + 6 * 8) as *mut u8,
                fp: base,
            }
        }
    }

    #[test]
    fn walk_visits_every_compiled_frame_up_to_the_host_entry() {
        let mut chain = Chain::new(0x100, 0x200, 0x300);
        let _activation = unsafe { ActiveStackMapScope::enter(&chain.registry) };
        let top = chain.top();
        let _frame = unsafe { SuspendedFrameScope::enter(top) };

        let base = chain.words.as_ptr() as usize;
        // B first (at the published sp), then A at fp_B + 16, then the chain ends
        // at the entry stub.
        assert_eq!(
            suspended_frames(),
            vec![(COMPILED_B + 0x20, base + 6 * 8), (COMPILED_A + 0x10, base + 16)]
        );
        let mut roots = Vec::new();
        append_native_roots(&mut roots);
        let mut addresses: Vec<usize> = roots.iter().map(|o: &Obj| o.addr() as usize).collect();
        addresses.sort_unstable();
        assert_eq!(addresses, vec![0x100, 0x200, 0x300]);
    }

    #[test]
    fn rewrite_updates_the_slots_of_outer_frames() {
        let mut chain = Chain::new(0x100, 0x200, 0x300);
        let _activation = unsafe { ActiveStackMapScope::enter(&chain.registry) };
        let top = chain.top();
        let _frame = unsafe { SuspendedFrameScope::enter(top) };

        let mut relocation = RelocationMap::new();
        relocation.insert_addresses(0x100, 0x1100);
        relocation.insert_addresses(0x200, 0x1200);
        relocation.insert_addresses(0x300, 0x1300);
        assert_eq!(unsafe { rewrite_native_roots(&relocation) }, 3);
        assert_eq!(chain.words[2..4], [0x1100, 0x1200]);
        assert_eq!(chain.words[6], 0x1300);
    }

    #[test]
    fn a_nested_activation_keeps_the_suspended_frames_of_the_outer_one() {
        let mut outer = Chain::new(0x100, 0x200, 0x300);
        let mut inner = Chain::new(0x400, 0x500, 0x600);
        let _outer_activation = unsafe { ActiveStackMapScope::enter(&outer.registry) };
        let outer_top = outer.top();
        let _outer_frame = unsafe { SuspendedFrameScope::enter(outer_top) };
        // The outer activation calls out (a bridge); the interpreter enters
        // compiled code again, which opens a second activation.
        let _inner_activation = unsafe { ActiveStackMapScope::enter(&inner.registry) };
        let inner_top = inner.top();
        let _inner_frame = unsafe { SuspendedFrameScope::enter(inner_top) };

        let mut roots = Vec::new();
        append_native_roots(&mut roots);
        let mut addresses: Vec<usize> = roots.iter().map(|o: &Obj| o.addr() as usize).collect();
        addresses.sort_unstable();
        assert_eq!(addresses, vec![0x100, 0x200, 0x300, 0x400, 0x500, 0x600]);
    }

    #[test]
    fn dropping_the_scope_unpublishes_the_frame() {
        let mut chain = Chain::new(0x100, 0x200, 0x300);
        let _activation = unsafe { ActiveStackMapScope::enter(&chain.registry) };
        let top = chain.top();
        {
            let _frame = unsafe { SuspendedFrameScope::enter(top) };
            assert!(!suspended_frames().is_empty());
        }
        assert!(suspended_frames().is_empty());
    }

    /// A return address that is neither compiled code nor a host entry means the
    /// chain left the managed frames: a debug build surfaces it.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "not a host entry")]
    fn a_return_address_outside_compiled_and_entry_code_is_a_bug_in_debug() {
        let mut chain = Chain::new(0x100, 0x200, 0x300);
        chain.words[5] = 0x7777_0000;
        let _activation = unsafe { ActiveStackMapScope::enter(&chain.registry) };
        let top = chain.top();
        let _frame = unsafe { SuspendedFrameScope::enter(top) };
        let _ = suspended_frames();
    }
}
