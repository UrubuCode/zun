//! C ABI allocation entry points used by generated native code.

use super::{append_native_roots, with_active_stack_map_registry, with_current_heap_mut, ActiveNativeFrameScope, Heap};

#[inline]
unsafe fn safepoint_pc(caller_sp: usize) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // Cranelift materializes the caller's SP immediately before the ABI
        // call. The call pushes its return address at SP - 8 on x86-64.
        unsafe { (caller_sp as *const usize).sub(1).read_unaligned() }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = caller_sp;
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
    let pc = unsafe { safepoint_pc(caller_sp) };

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
        with_current_heap_mut(|heap| {
            if heap.native_allocation_due() {
                let mut roots = Vec::new();
                append_native_roots(&mut roots);
                let layouts = heap.reference_slots.clone();
                let ref_slots = |rid: u32| layouts.get(&rid).cloned().unwrap_or_default();
                // SAFETY: the active frame and all registered roots remain
                // stopped and valid for this allocation slow path.
                unsafe {
                    heap.collect_with_active_native_frame(
                        &roots,
                        pc,
                        caller_sp as *const u8,
                        &ref_slots,
                    );
                }
            }
            heap.alloc(type_rid, field_count).addr()
        })
    })
}
