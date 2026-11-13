//! Native/JIT frame root registration for precise GC safepoints.

use super::Obj;
use super::RelocationMap;
use std::cell::{Cell, RefCell, UnsafeCell};
use std::marker::{PhantomData, PhantomPinned};
use std::pin::Pin;

thread_local! {
    static NEXT_ROOT_ID: Cell<usize> = const { Cell::new(0) };
    static NATIVE_ROOTS: RefCell<Vec<(usize, *const Obj, usize)>> = const { RefCell::new(Vec::new()) };
    /// Address words of live [`Rooted`] guards. Each is a managed address (0 =
    /// none) that the collector reports and rewrites in place. Kept apart from
    /// `NATIVE_ROOTS` because these slots are owned by their guard and written
    /// through a pointer derived from an `UnsafeCell`, not from a `&[Obj]`.
    static ROOTED_SLOTS: RefCell<Vec<*mut usize>> = const { RefCell::new(Vec::new()) };
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
            registered
                .borrow_mut()
                .push((id, roots.as_ptr(), roots.len()));
        });
        Self { id }
    }
}

impl Drop for NativeRootScope {
    fn drop(&mut self) {
        NATIVE_ROOTS.with(|registered| {
            let mut registered = registered.borrow_mut();
            if let Some(index) = registered.iter().position(|&(id, _, _)| id == self.id) {
                registered.remove(index);
            }
        });
    }
}

mod sealed {
    pub trait Sealed {}
}

/// A value whose managed reference is a single address word, so a [`Rooted`]
/// guard can hand that word to the collector and splice the relocated word back.
///
/// Sealed: a carrier is only correct if rewriting the word preserves every
/// other bit of the value, and that is a per-type decision (see the
/// `FunctionHandle` impl). Adding a carrier is an impl here, not a new guard.
pub trait RootCarrier: Copy + sealed::Sealed {
    /// The managed address held by the value; 0 means "references nothing".
    fn address(&self) -> usize;
    /// The value with only its managed address replaced.
    fn with_address(self, address: usize) -> Self;
}

impl sealed::Sealed for Obj {}
impl RootCarrier for Obj {
    fn address(&self) -> usize {
        self.addr() as usize
    }
    fn with_address(self, address: usize) -> Self {
        // SAFETY: the collector only hands back addresses of live objects.
        unsafe { Obj::from_addr(address as *mut u64) }
    }
}

impl<T: RootCarrier> sealed::Sealed for Option<T> {}
impl<T: RootCarrier> RootCarrier for Option<T> {
    fn address(&self) -> usize {
        self.map(|value| value.address()).unwrap_or(0)
    }
    fn with_address(self, address: usize) -> Self {
        // `None` carries no address, so the collector is told 0 for it and
        // `with_address` is only ever called back with that same 0.
        self.map(|value| value.with_address(address))
    }
}

impl sealed::Sealed for zun_core::FunctionHandle {}
impl RootCarrier for zun_core::FunctionHandle {
    fn address(&self) -> usize {
        self.environment().unwrap_or(0)
    }
    fn with_address(self, address: usize) -> Self {
        // `relocate_environment`, not `FunctionHandle::new`: `new` drops the
        // bound discriminant, turning a relocated bound function into a plain
        // closure over its bound record.
        self.relocate_environment((address != 0).then_some(address))
    }
}

/// A managed reference held by a native across a call that can collect and move
/// objects. The guard owns the value; the only way to read it is [`Rooted::get`],
/// which answers the current address. There is no un-rooted copy to use by
/// mistake, unlike [`NativeRootScope`] where the registered slice and the
/// variable the author keeps using are two different places.
///
/// Create one with [`root!`](crate::root).
#[derive(Debug)]
pub struct Rooted<T: RootCarrier> {
    /// Everything about the value except its managed address (e.g. a bound
    /// handle's discriminant). Its own address part is stale and never read.
    shell: Cell<T>,
    /// The managed address, rewritten by the collector. `UnsafeCell` so that
    /// the registered raw pointer is legitimately derived from interior-mutable
    /// storage rather than from a shared reference to plain data.
    slot: UnsafeCell<usize>,
    registered: Cell<bool>,
    /// Registration stores the slot's address, so the guard must not move
    /// afterwards; `!Unpin` makes that a `Pin` obligation.
    _pinned: PhantomPinned,
    /// The registry is thread-local; dropping on another thread would unregister
    /// from the wrong one and leave a dangling entry behind.
    _not_send: PhantomData<*const ()>,
}

impl<T: RootCarrier> Rooted<T> {
    /// An inert guard: nothing is rooted until [`Rooted::enter`]. Use
    /// [`root!`](crate::root), which pins and enters in one step.
    pub fn new(value: T) -> Self {
        Self {
            shell: Cell::new(value),
            slot: UnsafeCell::new(value.address()),
            registered: Cell::new(false),
            _pinned: PhantomPinned,
            _not_send: PhantomData,
        }
    }

    /// Register the pinned guard with the collector. Idempotent.
    pub fn enter(self: Pin<&Self>) -> Pin<&Self> {
        if !self.registered.replace(true) {
            let slot = self.slot.get();
            ROOTED_SLOTS.with(|slots| slots.borrow_mut().push(slot));
        }
        self
    }

    /// The value, at its current (possibly relocated) address.
    ///
    /// # Panics
    /// If the guard was never entered: its address could be stale already, and
    /// a loud failure beats a silent read of a recycled object.
    pub fn get(self: Pin<&Self>) -> T {
        assert!(
            self.registered.get(),
            "Rooted::get on a guard that was never entered; use root!"
        );
        // SAFETY: the slot is only touched on this thread (`!Send`), and the
        // collector writes it synchronously, never during this read.
        self.shell.get().with_address(unsafe { *self.slot.get() })
    }
}

impl<T: RootCarrier> Drop for Rooted<T> {
    fn drop(&mut self) {
        if self.registered.get() {
            let slot = self.slot.get();
            ROOTED_SLOTS.with(|slots| {
                let mut slots = slots.borrow_mut();
                if let Some(index) = slots.iter().position(|&s| s == slot) {
                    slots.remove(index);
                }
            });
        }
    }
}

/// Root a managed reference for the rest of the enclosing scope:
/// `root!(callback = callback);` then `callback.get()` after anything that can
/// collect.
///
/// The guard lives in a hidden, pinned local the caller cannot name, so it can
/// be neither moved nor dropped early; `callback` is rebound to a shared
/// `Pin<&Rooted<_>>`.
#[macro_export]
macro_rules! root {
    ($name:ident = $value:expr) => {
        let $name = ::core::pin::pin!($crate::Rooted::new($value));
        let $name = $crate::Rooted::enter($name.into_ref());
    };
}

/// Append roots registered by active native frames to a GC root set.
///
/// Reports both registries ([`NativeRootScope`] slices and [`Rooted`] guards),
/// so every collection path that honoured the former honours the latter.
pub fn append_native_roots(roots: &mut Vec<Obj>) {
    NATIVE_ROOTS.with(|registered| {
        for &(_, ptr, len) in registered.borrow().iter() {
            // SAFETY: each scope promises that its borrowed slice remains valid.
            roots.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr, len) });
        }
    });
    ROOTED_SLOTS.with(|slots| {
        for &slot in slots.borrow().iter() {
            // SAFETY: a guard unregisters its slot before the slot is freed.
            let address = unsafe { *slot };
            if address != 0 {
                roots.push(unsafe { Obj::from_addr(address as *mut u64) });
            }
        }
    });
}

/// Rewrite object pointers held by active native root scopes and [`Rooted`]
/// guards after relocation.
///
/// # Safety
/// The active scopes must still satisfy the lifetime guarantees documented by
/// [`NativeRootScope::enter`].
pub unsafe fn rewrite_native_roots(relocation: &RelocationMap) -> usize {
    let mut rewritten = 0;
    ROOTED_SLOTS.with(|slots| {
        for &slot in slots.borrow().iter() {
            // SAFETY: a guard unregisters its slot before the slot is freed, and
            // the pointer comes from an `UnsafeCell`, so writing is permitted.
            let address = unsafe { *slot };
            if address == 0 {
                continue;
            }
            let mapped = relocation.relocate_address(address);
            if mapped != address {
                unsafe { *slot = mapped };
                rewritten += 1;
            }
        }
    });
    NATIVE_ROOTS.with(|registered| {
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
    });
    rewritten
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
