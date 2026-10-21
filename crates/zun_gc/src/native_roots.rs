//! Native/JIT frame root registration for precise GC safepoints.

use super::Obj;
use super::RelocationMap;
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

/// Rewrite object pointers held by active native root scopes after relocation.
///
/// # Safety
/// The active scopes must still satisfy the lifetime guarantees documented by
/// [`NativeRootScope::enter`].
pub unsafe fn rewrite_native_roots(relocation: &RelocationMap) -> usize {
    NATIVE_ROOTS.with(|registered| {
        let mut rewritten = 0;
        for &(_, ptr, len) in registered.borrow().iter() {
            let roots = unsafe { std::slice::from_raw_parts_mut(ptr as *mut Obj, len) };
            for root in roots {
                let address = root.addr() as usize;
                let mapped = relocation.relocate_address(address);
                if mapped != address {
                    *root = unsafe { Obj::from_addr(mapped as *mut u64) };
                    rewritten += 1;
                }
            }
        }
        rewritten
    })
}

#[cfg(test)]
mod tests {
    use super::{rewrite_native_roots, NativeRootScope};
    use crate::{Obj, RelocationMap};

    #[test]
    fn rewrites_active_native_root_slots() {
        let old = 0x1000usize;
        let new = 0x2000usize;
        let roots = [unsafe { Obj::from_addr(old as *mut u64) }];
        let mut relocation = RelocationMap::new();
        relocation.insert_addresses(old, new);
        let _scope = unsafe { NativeRootScope::enter(&roots) };
        assert_eq!(unsafe { rewrite_native_roots(&relocation) }, 1);
        assert_eq!(roots[0].addr() as usize, new);
    }
}
