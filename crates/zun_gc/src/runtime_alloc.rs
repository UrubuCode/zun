//! C ABI allocation entry points used by generated native code.

use super::Heap;

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
