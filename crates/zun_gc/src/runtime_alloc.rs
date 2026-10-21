//! C ABI allocation entry points used by generated native code.

use super::{with_active_stack_map_registry, with_current_heap_mut, ActiveNativeFrameScope, Heap};

/// Allocate through a raw `Heap` pointer for the `newobj` runtime call.
///
/// # Safety
/// `heap` must be a valid pointer to a live `Heap`.
pub unsafe extern "C" fn zun_gc_alloc(
    heap: *mut Heap,
    type_rid: u32,
    field_count: usize,
) -> *mut u64 {
    // SAFETY: the caller guarantees `heap` is a live Heap.
    let heap = unsafe { &mut *heap };
    heap.alloc(type_rid, field_count).addr()
}

/// Allocate through the thread's current heap for JIT `newobj` calls.
///
/// # Safety
/// A current heap must be registered via `HeapScope`.
pub unsafe extern "C" fn zun_rt_alloc(
    type_rid: u32,
    field_count: usize,
    caller_sp: usize,
) -> *mut u64 {
    #[cfg(target_arch = "x86_64")]
    let pc = {
        let address: usize;
        // Read the return address at the allocation helper's ABI entry. This
        // must be done here rather than in a nested Rust function, otherwise
        // the captured address would identify that nested call site.
        unsafe { core::arch::asm!("mov {}, [rsp]", out(reg) address, options(nostack, preserves_flags)); }
        address
    };
    #[cfg(not(target_arch = "x86_64"))]
    let pc = 0;

    with_active_stack_map_registry(|registry| {
        let Some(registry) = registry else {
            return with_current_heap_mut(|heap| heap.alloc(type_rid, field_count).addr());
        };
        // The JIT has already stopped at this call site. Keep its exact
        // safepoint coordinates published while the allocation slow path is
        // active so a collection can consume the registered stack map.
        let _frame = unsafe {
            ActiveNativeFrameScope::enter(registry, pc, caller_sp as *mut u8)
        };
        with_current_heap_mut(|heap| heap.alloc(type_rid, field_count).addr())
    })
}
