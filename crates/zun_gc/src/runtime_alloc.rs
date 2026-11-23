//! C ABI allocation entry points used by generated native code.

use super::{
    append_native_roots, with_active_stack_map_registry, with_current_heap_mut, Heap,
    SuspendedFrameScope,
};

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
/// `caller_sp` is the compiled caller's stack pointer immediately before this
/// call and `caller_fp` its frame pointer. Both are read by the caller's code
/// (a register move each), so the fast path pays nothing for them; the slow path
/// publishes them as the activation's innermost compiled frame, and the
/// collection walks the frame-pointer chain from there to scan every compiled
/// caller, not only this one.
///
/// # Safety
/// A current heap must be registered via `HeapScope`.
pub unsafe extern "C" fn zun_rt_alloc(
    type_rid: u32,
    field_count: usize,
    caller_sp: usize,
    caller_fp: usize,
) -> *mut u64 {
    // Fast path: one thread-local lookup. Allocation count is a coarse fallback
    // cadence; the nursery budget is the actual generational trigger. Large
    // objects can exhaust that budget in a single call, so do not wait for the
    // count-based cadence in that case. Publishing the safepoint (below) is only
    // needed when a collection can actually run.
    let fast = with_current_heap_mut(|heap| {
        if heap.native_allocation_due() || heap.nursery_collection_due() {
            None
        } else {
            Some(heap.alloc(type_rid, field_count).addr())
        }
    });
    if let Some(address) = fast {
        return address;
    }

    let has_activation = with_active_stack_map_registry(|registry| registry.is_some());
    if !has_activation {
        return with_current_heap_mut(|heap| heap.alloc(type_rid, field_count).addr());
    }
    // The JIT has already stopped at this call site. Publish its frame (the
    // exact safepoint plus the frame pointer that anchors the chain of compiled
    // callers) while the slow path runs, so a collection consumes the registered
    // stack maps of every compiled frame.
    let _frame = unsafe { SuspendedFrameScope::enter_at_call(caller_sp, caller_fp) };
    with_current_heap_mut(|heap| {
        // The layout table is moved out for the collection rather than
        // cloned, and its slot lists are shared, so a layout lookup during
        // marking or rewriting is a reference-count bump, not an allocation.
        let layouts = std::mem::take(&mut heap.reference_slots);
        let empty: std::rc::Rc<[usize]> = std::rc::Rc::from(Vec::new());
        let ref_slots = |rid: u32| layouts.get(&rid).cloned().unwrap_or_else(|| empty.clone());
        // SAFETY: the published frames and all registered roots remain stopped
        // and valid for this allocation slow path.
        unsafe {
            if heap.native_compaction_due() {
                heap.collect_and_compact_with_registered_native_frames(
                    &mut Vec::new(),
                    &ref_slots,
                );
            } else {
                // The compiled frames' references come with the native roots.
                let mut roots = Vec::new();
                append_native_roots(&mut roots);
                heap.collect_young_evacuating(&mut roots, &ref_slots, &mut |_| {});
            }
        }
        heap.reference_slots = layouts;
        heap.alloc(type_rid, field_count).addr()
    })
}

#[cfg(test)]
mod tests {
    use super::super::{ActiveStackMapScope, Heap, HeapScope, NativeStackMap, StackMapRegistry};

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn return_pc_before_call_reads_return_address_from_pre_call_stack_pointer() {
        let mut call_frame = [0usize; 1];
        call_frame[0] = 0x1234_5678;
        let pre_call_sp = unsafe { call_frame.as_mut_ptr().add(1) as usize };

        let pc = unsafe { crate::return_pc_before_call(pre_call_sp) };

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

        for _ in 0..140_000 {
            unsafe { super::zun_rt_alloc(2, 0, caller_sp, 0) };
        }

        // The 64th young collection (the backstop; this test promotes almost
        // nothing) compacts the heap, 16-byte objects spending the 32 KiB
        // nursery budget every 2048 allocations. The stack-map slot,
        // rather than the stale pre-compaction Obj, is the authoritative root.
        assert!(heap.by_addr.contains_key(&native_frame[1]));
    }
}
