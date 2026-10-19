//! The Zun runtime heap.
//!
//! A non-moving object allocator shared, by design, between the interpreter and
//! the JIT (PLAN.md §4.3, step 2). An object is a contiguous run of 8-byte
//! slots: slot 0 holds the type RID, slots `1..=field_count` hold the fields.
//! Field slots are **raw** — a number's bits, an integer, a boolean, a nested
//! object pointer, or a string-table handle — interpreted by the field's static
//! type. This is what lets native code load and store fields by offset.
//!
//! Non-moving: once allocated, an object's address never changes, so a pointer
//! handed to native code stays valid. There is no collection yet — memory is
//! reclaimed only when the [`Heap`] is dropped ("allocate first, collect
//! later"). A later phase adds mark-sweep over this same layout, using the
//! type RID to find each object's reference slots.
//!
//! # Safety
//! Object pointers are valid only while the owning [`Heap`] is alive. The
//! interpreter and JIT must not outlive it. Within that lifetime the heap never
//! moves or frees a block, so raw access through [`Obj`] is sound.

use std::rc::Rc;

/// Slots per allocation chunk (objects are bump-allocated within a chunk).
const CHUNK_SLOTS: usize = 64 * 1024;

/// The size of one object slot in bytes.
pub const SLOT_SIZE: usize = 8;

/// A pointer to an allocated object: slot 0 is the type RID, the rest fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Obj(*mut u64);

impl Obj {
    /// The raw address, for handing to native (JIT) code.
    pub fn addr(self) -> *mut u64 {
        self.0
    }

    /// Rebuild an `Obj` from a raw address (e.g. one returned to native code).
    ///
    /// # Safety
    /// `addr` must be an object pointer the live [`Heap`] produced.
    pub unsafe fn from_addr(addr: *mut u64) -> Obj {
        Obj(addr)
    }

    /// The object's type RID (slot 0).
    pub fn type_rid(self) -> u32 {
        // SAFETY: a valid Obj points at an allocated block whose slot 0 exists.
        unsafe { *self.0 as u32 }
    }

    /// Read raw field `index` (0-based; slot `index + 1`).
    pub fn field(self, index: usize) -> u64 {
        // SAFETY: callers index within the object's field count (the verifier
        // and layout guarantee this for generated code).
        unsafe { *self.0.add(index + 1) }
    }

    /// Write raw field `index`.
    pub fn set_field(self, index: usize, value: u64) {
        // SAFETY: see `field`.
        unsafe { *self.0.add(index + 1) = value }
    }
}

/// The runtime heap: bump-allocated chunks plus a string handle table.
#[derive(Debug, Default)]
pub struct Heap {
    /// Boxed chunks; their contents have stable addresses for the heap's life.
    chunks: Vec<Box<[u64]>>,
    /// Bump cursor within the last chunk.
    cursor: usize,
    /// Interned strings, referenced from field slots by index.
    strings: Vec<Rc<str>>,
}

impl Heap {
    /// Create an empty heap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate an object with `field_count` fields of type `type_rid`,
    /// returning a pointer to it. Fields start zeroed.
    pub fn alloc(&mut self, type_rid: u32, field_count: usize) -> Obj {
        let slots = field_count + 1; // + the type-RID header slot
        assert!(slots <= CHUNK_SLOTS, "object larger than a heap chunk");

        if self.chunks.is_empty() || self.cursor + slots > CHUNK_SLOTS {
            self.chunks.push(vec![0u64; CHUNK_SLOTS].into_boxed_slice());
            self.cursor = 0;
        }
        let chunk = self.chunks.last_mut().expect("just pushed a chunk");
        let base = &mut chunk[self.cursor] as *mut u64;
        self.cursor += slots;

        // SAFETY: `base` points at `slots` freshly zeroed slots inside a boxed
        // chunk that will not move or free while `self` lives.
        unsafe {
            *base = type_rid as u64;
        }
        Obj(base)
    }

    /// Intern a string, returning its handle for storing in a field slot.
    pub fn intern_string(&mut self, s: Rc<str>) -> u32 {
        let handle = self.strings.len() as u32;
        self.strings.push(s);
        handle
    }

    /// Resolve a string handle back to its string.
    pub fn string(&self, handle: u32) -> &Rc<str> {
        &self.strings[handle as usize]
    }

    /// Number of objects' worth of chunks currently held (for diagnostics).
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

/// Allocate through a raw `Heap` pointer — the entry point native (JIT) code
/// calls for `newobj`.
///
/// # Safety
/// `heap` must be a valid pointer to a live [`Heap`]; the returned pointer is
/// valid for that heap's lifetime.
pub unsafe extern "C" fn zun_gc_alloc(heap: *mut Heap, type_rid: u32, field_count: usize) -> *mut u64 {
    // SAFETY: the caller guarantees `heap` is a live Heap.
    let heap = unsafe { &mut *heap };
    heap.alloc(type_rid, field_count).addr()
}

// --- current-heap registration (the runtime context shared with the JIT) ---

thread_local! {
    /// The heap native code allocates into for the duration of a run. The VM
    /// sets it before executing and clears it after, so interpreter and JIT
    /// share one heap. Raw pointer because the heap is owned on the stack.
    static CURRENT_HEAP: std::cell::Cell<*mut Heap> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// A guard that registers `heap` as the current heap and clears it on drop.
#[derive(Debug)]
pub struct HeapScope {
    prev: *mut Heap,
}

impl HeapScope {
    /// Register `heap` as the current heap for this thread.
    ///
    /// # Safety
    /// `heap` must stay valid and not move until this guard is dropped.
    pub unsafe fn enter(heap: *mut Heap) -> HeapScope {
        let prev = CURRENT_HEAP.with(|c| c.replace(heap));
        HeapScope { prev }
    }
}

impl Drop for HeapScope {
    fn drop(&mut self) {
        CURRENT_HEAP.with(|c| c.set(self.prev));
    }
}

/// Run `f` with a mutable borrow of the current heap. Panics if none is set.
pub fn with_current_heap_mut<R>(f: impl FnOnce(&mut Heap) -> R) -> R {
    let ptr = CURRENT_HEAP.with(|c| c.get());
    assert!(!ptr.is_null(), "no current heap registered");
    // SAFETY: the HeapScope guard keeps `ptr` valid while set, and the runtime
    // is single-threaded, so no aliasing borrow is live across this call.
    f(unsafe { &mut *ptr })
}

/// Run `f` with a shared borrow of the current heap. Panics if none is set.
pub fn with_current_heap<R>(f: impl FnOnce(&Heap) -> R) -> R {
    with_current_heap_mut(|h| f(h))
}

/// The allocation entry point native (JIT) code calls for `newobj`, using the
/// thread's current heap.
///
/// # Safety
/// A current heap must be registered (via [`HeapScope`]) for the call.
pub unsafe extern "C" fn zun_rt_alloc(type_rid: u32, field_count: usize) -> *mut u64 {
    with_current_heap_mut(|h| h.alloc(type_rid, field_count).addr())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_sets_type_and_zeroes_fields() {
        let mut heap = Heap::new();
        let o = heap.alloc(7, 3);
        assert_eq!(o.type_rid(), 7);
        assert_eq!(o.field(0), 0);
        assert_eq!(o.field(1), 0);
        assert_eq!(o.field(2), 0);
    }

    #[test]
    fn fields_read_back_what_was_written() {
        let mut heap = Heap::new();
        let o = heap.alloc(1, 2);
        o.set_field(0, 42);
        o.set_field(1, f64::to_bits(3.5));
        assert_eq!(o.field(0), 42);
        assert_eq!(f64::from_bits(o.field(1)), 3.5);
    }

    #[test]
    fn addresses_stay_stable_across_many_allocations() {
        let mut heap = Heap::new();
        let first = heap.alloc(1, 4);
        first.set_field(0, 0xDEAD_BEEF);
        // Allocating many more objects (spanning several chunks) must not move
        // or corrupt the first object.
        let mut last = first;
        for i in 0..200_000u64 {
            last = heap.alloc(2, 4);
            last.set_field(0, i);
        }
        assert_eq!(first.field(0), 0xDEAD_BEEF);
        assert_eq!(first.type_rid(), 1);
        assert!(heap.chunk_count() > 1, "should have spanned multiple chunks");
        assert_eq!(last.field(0), 199_999);
    }

    #[test]
    fn nested_object_pointer_in_a_field() {
        let mut heap = Heap::new();
        let inner = heap.alloc(5, 1);
        inner.set_field(0, 99);
        let outer = heap.alloc(6, 1);
        outer.set_field(0, inner.addr() as u64);
        // Recover the inner object from the raw pointer stored in outer's field.
        let recovered = unsafe { Obj::from_addr(outer.field(0) as *mut u64) };
        assert_eq!(recovered.field(0), 99);
        assert_eq!(recovered.type_rid(), 5);
    }

    #[test]
    fn string_table_roundtrip() {
        let mut heap = Heap::new();
        let h = heap.intern_string("hello".into());
        assert_eq!(&**heap.string(h), "hello");
    }

    #[test]
    fn extern_alloc_matches_method_alloc() {
        let mut heap = Heap::new();
        let ptr = unsafe { zun_gc_alloc(&mut heap as *mut Heap, 3, 2) };
        let o = unsafe { Obj::from_addr(ptr) };
        assert_eq!(o.type_rid(), 3);
        o.set_field(1, 7);
        assert_eq!(o.field(1), 7);
    }
}
