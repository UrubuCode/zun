//! Current-heap registration shared by interpreter, JIT, and native calls.

use super::Heap;
use std::cell::Cell;

thread_local! {
    static CURRENT_HEAP: Cell<*mut Heap> = const { Cell::new(std::ptr::null_mut()) };
    /// Chunk-pointer table of the current heap's static store (null with no
    /// heap). JIT code embeds this cell's address and loads the table through it
    /// on every inline static access, so compiled code is never bound to one
    /// heap -- only to the thread that built it, the same contract as the
    /// pending-exception slot.
    static CURRENT_STATICS: Cell<*const *mut ()> = const { Cell::new(std::ptr::null()) };
}

#[derive(Debug)]
pub struct HeapScope {
    prev: *mut Heap,
}

impl HeapScope {
    /// # Safety
    /// `heap` must remain valid and unmoved until this guard is dropped.
    pub unsafe fn enter(heap: *mut Heap) -> Self {
        let prev = CURRENT_HEAP.with(|cell| cell.replace(heap));
        set_current_statics(heap);
        Self { prev }
    }
}

impl Drop for HeapScope {
    fn drop(&mut self) {
        CURRENT_HEAP.with(|cell| cell.set(self.prev));
        set_current_statics(self.prev);
    }
}

fn set_current_statics(heap: *mut Heap) {
    let table = if heap.is_null() {
        std::ptr::null()
    } else {
        // SAFETY: a scope's heap pointer is valid while the scope is live.
        unsafe { (*heap).static_store.table_ptr().cast::<*mut ()>() }
    };
    CURRENT_STATICS.with(|cell| cell.set(table));
}

/// Address of this thread's current-statics cell (see `CURRENT_STATICS`).
/// Stable for the thread's life.
pub fn static_table_cell_addr() -> u64 {
    CURRENT_STATICS.with(|cell| cell.as_ptr() as u64)
}

pub fn with_current_heap_mut<R>(f: impl FnOnce(&mut Heap) -> R) -> R {
    let ptr = CURRENT_HEAP.with(|cell| cell.get());
    assert!(!ptr.is_null(), "no current heap registered");
    // SAFETY: HeapScope keeps the pointer valid for this thread.
    f(unsafe { &mut *ptr })
}

pub fn with_current_heap<R>(f: impl FnOnce(&Heap) -> R) -> R {
    with_current_heap_mut(|heap| f(heap))
}

/// Return the thread-local heap pointer for native/JIT boundaries.
///
/// The pointer is valid only while the corresponding [`HeapScope`] remains
/// alive. Callers must not retain it beyond that scope.
pub fn current_heap_ptr() -> Option<*mut Heap> {
    let ptr = CURRENT_HEAP.with(|cell| cell.get());
    (!ptr.is_null()).then_some(ptr)
}
