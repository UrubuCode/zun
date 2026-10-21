//! C ABI allocation entry points used by generated native code.

use super::{with_active_stack_map_registry, with_current_heap_mut, ActiveNativeFrameScope, Heap};

#[inline(never)]
fn return_address() -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        let address: usize;
        // The helper is entered at the allocation safepoint. On x86-64 the
        // return address is at the top of the callee's stack before any
        // compiler-generated frame is established.
        unsafe { core::arch::asm!("mov {}, [rsp]", out(reg) address, options(nostack, preserves_flags)); }
        address
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

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
    with_active_stack_map_registry(|registry| {
        let Some(registry) = registry else {
            return with_current_heap_mut(|heap| heap.alloc(type_rid, field_count).addr());
        };
        let pc = return_address();
        // The JIT has already stopped at this call site. Keep its exact
        // safepoint coordinates published while the allocation slow path is
        // active so a collection can consume the registered stack map.
        let _frame = unsafe {
            ActiveNativeFrameScope::enter(registry, pc, caller_sp as *mut u8)
        };
        with_current_heap_mut(|heap| heap.alloc(type_rid, field_count).addr())
    })
}
