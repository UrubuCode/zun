//! Native/JIT frame root registration for precise GC safepoints.

use super::Obj;
use std::cell::{Cell, RefCell};

thread_local! {
    static NEXT_ROOT_ID: Cell<usize> = const { Cell::new(0) };
    static NATIVE_ROOTS: RefCell<Vec<(usize, *const Obj, usize)>> = const { RefCell::new(Vec::new()) };
}

/// Guard that exposes object references held by a native/JIT frame to GC.
#[derive(Debug)]
pub struct NativeRootScope {
    id: usize,
}

impl NativeRootScope {
    /// Register `roots` for the duration of a native call.
    ///
    /// # Safety
    /// `roots` must remain allocated and unchanged until the returned guard is
    /// dropped, and every entry must belong to the current heap.
    pub unsafe fn enter(roots: &[Obj]) -> Self {
        let id = NEXT_ROOT_ID.with(|next| {
            let id = next.get();
            next.set(id.wrapping_add(1));
            id
        });
        NATIVE_ROOTS.with(|registered| {
            registered.borrow_mut().push((id, roots.as_ptr(), roots.len()));
        });
        Self { id }
    }
}

impl Drop for NativeRootScope {
    fn drop(&mut self) {
        NATIVE_ROOTS.with(|registered| {
            let mut registered = registered.borrow_mut();
            if let Some(index) = registered.iter().position(|&(id, _, _)| id == self.id)
            {
                registered.remove(index);
            }
        });
    }
}

/// Append roots registered by active native frames to a GC root set.
pub fn append_native_roots(roots: &mut Vec<Obj>) {
    NATIVE_ROOTS.with(|registered| {
        for &(_, ptr, len) in registered.borrow().iter() {
            // SAFETY: each scope promises that its borrowed slice remains valid.
            roots.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr, len) });
        }
    });
}
