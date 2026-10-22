//! C ABI allocation entry points used by generated native code.

use super::{with_active_stack_map_registry, with_current_heap_mut, ActiveNativeFrameScope, Heap};

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
            // Allocation count is a coarse fallback cadence; the nursery
            // budget is the actual generational trigger. Large objects can
            // exhaust that budget in a single call, so do not wait for the
            // count-based cadence in that case.
            if heap.native_allocation_due() || heap.nursery_collection_due() {
                let layouts = heap.reference_slots.clone();
                let ref_slots = |rid: u32| layouts.get(&rid).cloned().unwrap_or_default();
                // SAFETY: the active frame and all registered roots remain
                // stopped and valid for this allocation slow path.
                unsafe {
                    if heap.native_compaction_due() {
                        heap.collect_and_compact_with_registered_native_frames(
                            &mut Vec::new(),
                            &ref_slots,
                        );
                    } else {
                        heap.collect_young_evacuating_with_active_native_frame(
                            &mut Vec::new(),
                            pc,
                            caller_sp as *mut u8,
                            &ref_slots,
                        );
                    }
                }
            }
            heap.alloc(type_rid, field_count).addr()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::super::{ActiveStackMapScope, Heap, HeapScope, NativeStackMap, StackMapRegistry};

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn safepoint_pc_reads_return_address_from_pre_call_stack_pointer() {
        let mut call_frame = [0usize; 1];
        call_frame[0] = 0x1234_5678;
        let pre_call_sp = unsafe { call_frame.as_mut_ptr().add(1) as usize };

        let pc = unsafe { super::safepoint_pc(pre_call_sp) };

        assert_eq!(pc, call_frame[0]);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn jit_allocation_helper_collects_using_the_registered_stack_map() {
        let mut heap = Heap::new();
        let live = heap.alloc(1, 0);
        let return_pc = 0x4321usize;
        let mut native_frame = [return_pc, live.addr() as usize];
        let caller_sp = unsafe { native_frame.as_mut_ptr().add(1) as usize };

        let mut registry = StackMapRegistry::new();
        registry.replace([NativeStackMap {
            pc: return_pc,
            span: 1,
            reference_slots: vec![0],
        }]);
        let _registry_scope = unsafe { ActiveStackMapScope::enter(&registry) };
        let _heap_scope = unsafe { HeapScope::enter(&mut heap) };

        for _ in 0..2_048 {
            unsafe { super::zun_rt_alloc(2, 0, caller_sp) };
        }

        // The eighth young collection compacts the heap. The stack-map slot,
        // rather than the stale pre-compaction Obj, is the authoritative root.
        assert!(heap.by_addr.contains_key(&native_frame[1]));
    }
}
