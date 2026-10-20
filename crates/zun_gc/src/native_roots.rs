//! Native/JIT frame root registration for precise GC safepoints.

use super::Obj;
use std::cell::RefCell;

thread_local! {
    static NATIVE_ROOTS: RefCell<Vec<(*const Obj, usize)>> = const { RefCell::new(Vec::new()) };
}

/// Guard that exposes object references held by a native/JIT frame to GC.
#[derive(Debug)]
pub struct NativeRootScope {
    roots: *const Obj,
    len: usize,
}

impl NativeRootScope {
    /// Register `roots` for the duration of a native call.
    ///
    /// # Safety
    /// `roots` must remain allocated and unchanged until the returned guard is
    /// dropped, and every entry must belong to the current heap.
    pub unsafe fn enter(roots: &[Obj]) -> Self {
        NATIVE_ROOTS.with(|registered| {
            registered.borrow_mut().push((roots.as_ptr(), roots.len()));
        });
        Self { roots: roots.as_ptr(), len: roots.len() }
    }
}

impl Drop for NativeRootScope {
    fn drop(&mut self) {
        NATIVE_ROOTS.with(|registered| {
            let mut registered = registered.borrow_mut();
            if let Some(index) = registered
                .iter()
                .rposition(|&(ptr, len)| ptr == self.roots && len == self.len)
            {
                registered.remove(index);
            }
        });
    }
}

/// Append roots registered by active native frames to a GC root set.
pub fn append_native_roots(roots: &mut Vec<Obj>) {
    NATIVE_ROOTS.with(|registered| {
        for &(ptr, len) in registered.borrow().iter() {
            // SAFETY: each scope promises that its borrowed slice remains valid.
            roots.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr, len) });
        }
    });
}
