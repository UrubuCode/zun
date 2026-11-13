//! Native/JIT frame root registration for precise GC safepoints.
//!
//! Two shapes, and the difference is whether the native reads the reference
//! back after a safepoint:
//!
//! - [`Rooted`] (via [`root!`](crate::root)) holds one managed reference in a
//!   pinned guard. This is what a native wants almost always.
//! - [`RootArray`] holds a run of them in storage the **registry owns**, for a
//!   native that needs more slots than it has guards — a recursive walk with a
//!   slot per nesting level (`JSON.stringify`), or a packed argument list a JIT
//!   entry point only needs kept alive.
//!
//! Both are read through the same storage the collector writes, which is the
//! whole point. The predecessor, `NativeRootScope`, registered a pointer into
//! the caller's non-`mut` local array and the collector wrote through it; that
//! is undefined behaviour, and the compiler acted on it — a re-read of
//! `roots[i]` could legitimately answer the pre-collection address out of a
//! register. There is no API here that hands out that mistake any more.

use super::Obj;
use super::RelocationMap;
use std::cell::{Cell, RefCell, UnsafeCell};
use std::marker::{PhantomData, PhantomPinned};
use std::pin::Pin;

thread_local! {
    static NEXT_ROOT_ID: Cell<usize> = const { Cell::new(0) };
    /// Managed addresses owned by live [`RootArray`]s (0 = an empty slot).
    static ROOT_ARRAYS: RefCell<Vec<(usize, Vec<usize>)>> = const { RefCell::new(Vec::new()) };
    /// Address words of live [`Rooted`] guards. Each is a managed address (0 =
    /// none) that the collector reports and rewrites in place. Kept apart from
    /// `ROOT_ARRAYS` because these slots are owned by their guard and written
    /// through a pointer derived from an `UnsafeCell`.
    static ROOTED_SLOTS: RefCell<Vec<*mut usize>> = const { RefCell::new(Vec::new()) };
}

fn next_root_id() -> usize {
    NEXT_ROOT_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    })
}

/// A run of managed references the collector's registry owns on a native's
/// behalf, addressed by index.
///
/// Reads and writes go through the registry, so they see the same storage the
/// collector rewrites: there is no copy in the caller's frame to go stale, and
/// no pointer for the optimiser to assume nothing writes through. Slots start
/// empty, and [`RootArray::get`] answers `None` for one that was never set.
///
/// Drops in any order; the registry is keyed by id, not a stack.
#[derive(Debug)]
pub struct RootArray {
    id: usize,
    /// The registry is thread-local; dropping on another thread would
    /// unregister from the wrong one and leave an entry behind.
    _not_send: PhantomData<*const ()>,
}

impl RootArray {
    /// Register `len` empty slots.
    pub fn new(len: usize) -> Self {
        Self::register(vec![0; len])
    }

    /// Register the given objects, in order.
    ///
    /// This is the form a native uses when it only needs the objects **kept
    /// alive** across a call and never reads them back — the JIT entry points
    /// pack their arguments into raw words before the call, and what a
    /// collection would have to rewrite afterwards is those words and the JIT
    /// frame's own slots, which Cranelift stack maps describe, not this array.
    pub fn from_objects(objects: impl IntoIterator<Item = Obj>) -> Self {
        Self::register(
            objects
                .into_iter()
                .map(|object| object.addr() as usize)
                .collect(),
        )
    }

    fn register(addresses: Vec<usize>) -> Self {
        let id = next_root_id();
        ROOT_ARRAYS.with(|arrays| arrays.borrow_mut().push((id, addresses)));
        Self {
            id,
            _not_send: PhantomData,
        }
    }

    fn with_slots<R>(&self, apply: impl FnOnce(&mut Vec<usize>) -> R) -> R {
        ROOT_ARRAYS.with(|arrays| {
            let mut arrays = arrays.borrow_mut();
            let entry = arrays
                .iter_mut()
                .find(|(id, _)| *id == self.id)
                .expect("a live RootArray is registered");
            apply(&mut entry.1)
        })
    }

    /// The object in `index`, at its current address, or `None` when the slot
    /// is empty or out of range.
    pub fn get(&self, index: usize) -> Option<Obj> {
        let address = self.with_slots(|slots| slots.get(index).copied())?;
        // SAFETY: the collector only ever writes a live object's address here.
        (address != 0).then(|| unsafe { Obj::from_addr(address as *mut u64) })
    }

    /// Put `object` in `index`, growing the array if it is past the end.
    pub fn set(&self, index: usize, object: Obj) {
        self.with_slots(|slots| {
            if index >= slots.len() {
                slots.resize(index + 1, 0);
            }
            slots[index] = object.addr() as usize;
        });
    }

    /// Empty the slot at `index`.
    pub fn clear(&self, index: usize) {
        self.with_slots(|slots| {
            if let Some(slot) = slots.get_mut(index) {
                *slot = 0;
            }
        });
    }
}

impl Drop for RootArray {
    fn drop(&mut self) {
        ROOT_ARRAYS.with(|arrays| {
            let mut arrays = arrays.borrow_mut();
            if let Some(index) = arrays.iter().position(|(id, _)| *id == self.id) {
                arrays.remove(index);
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
/// mistake, which is what the predecessor invited: the registered slice and
/// the variable the author kept using were two different places.
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
/// Reports both registries ([`Rooted`] guards and [`RootArray`] slots), so
/// every collection path honours both.
pub fn append_native_roots(roots: &mut Vec<Obj>) {
    ROOT_ARRAYS.with(|arrays| {
        for (_, slots) in arrays.borrow().iter() {
            for &address in slots {
                if address != 0 {
                    // SAFETY: a slot only ever holds a live object's address.
                    roots.push(unsafe { Obj::from_addr(address as *mut u64) });
                }
            }
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

/// Rewrite the managed references held by active [`Rooted`] guards and
/// [`RootArray`]s after relocation.
///
/// # Safety
/// Every live guard must still own its slot, which is what `Drop` guarantees.
/// Nothing else is borrowed from a caller, so there is no lifetime promise
/// left for a caller to break.
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
    ROOT_ARRAYS.with(|arrays| {
        for (_, slots) in arrays.borrow_mut().iter_mut() {
            for slot in slots.iter_mut() {
                if *slot == 0 {
                    continue;
                }
                let mapped = relocation.relocate_address(*slot);
                if mapped != *slot {
                    *slot = mapped;
                    rewritten += 1;
                }
            }
        }
    });
    rewritten
}

#[cfg(test)]
mod tests {
    use super::{append_native_roots, rewrite_native_roots, RootArray};
    use crate::{Obj, RelocationMap};

    fn obj(address: usize) -> Obj {
        unsafe { Obj::from_addr(address as *mut u64) }
    }

    #[test]
    fn a_root_array_slot_is_read_back_at_its_relocated_address() {
        let array = RootArray::new(2);
        array.set(1, obj(0x1000));
        let mut relocation = RelocationMap::new();
        relocation.insert_addresses(0x1000, 0x2000);

        assert_eq!(unsafe { rewrite_native_roots(&relocation) }, 1);

        assert_eq!(array.get(1).map(|o| o.addr() as usize), Some(0x2000));
        assert_eq!(array.get(0), None, "an unset slot stays empty");
        assert_eq!(array.get(9), None, "an index past the end is empty");
    }

    #[test]
    fn a_root_array_reports_only_its_filled_slots() {
        let array = RootArray::from_objects([obj(0x1000), obj(0x1040)]);
        let empty = RootArray::new(4);
        let mut roots = Vec::new();
        append_native_roots(&mut roots);
        assert_eq!(roots, vec![obj(0x1000), obj(0x1040)]);
        drop(empty);

        array.clear(0);
        roots.clear();
        append_native_roots(&mut roots);
        assert_eq!(roots, vec![obj(0x1040)]);
    }

    #[test]
    fn a_dropped_root_array_is_no_longer_reported() {
        {
            let _array = RootArray::from_objects([obj(0x1000)]);
        }
        let mut roots = Vec::new();
        append_native_roots(&mut roots);
        assert!(roots.is_empty());
    }
}
