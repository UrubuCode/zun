//! Current-heap registration shared by interpreter, JIT, and native calls.

use super::Heap;
use std::cell::Cell;

thread_local! {
    static CURRENT_HEAP: Cell<*mut Heap> = const { Cell::new(std::ptr::null_mut()) };
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
        Self { prev }
    }
}

impl Drop for HeapScope {
    fn drop(&mut self) {
        CURRENT_HEAP.with(|cell| cell.set(self.prev));
    }
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
