//! The Zun runtime heap.
//!
//! A non-moving object allocator shared, by design, between the interpreter and
//! the JIT (PLAN.md §4.3, step 2). An object is a contiguous run of 8-byte
//! slots: slot 0 holds the type RID, slot 1 holds the identity hash, and slots
//! `2..=field_count + 1` hold the fields.
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

use std::collections::HashMap;
use std::rc::Rc;

fn hash_bytes(bytes: &[u8]) -> u64 {
    bytes.iter().fold(1469598103934665603, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(1099511628211)
    })
}

fn any_same_value_zero(a: zun_core::Any, b: zun_core::Any, strings: &[Rc<str>]) -> bool {
    if a.tag() != b.tag() {
        return false;
    }
    match a.tag() {
        zun_core::AnyTag::Float => {
            let left = a.as_float().unwrap();
            let right = b.as_float().unwrap();
            left == right || (left.is_nan() && right.is_nan())
        }
        zun_core::AnyTag::String => {
            strings.get(a.as_string().unwrap() as usize)
                == strings.get(b.as_string().unwrap() as usize)
        }
        zun_core::AnyTag::Int => a.as_int() == b.as_int(),
        zun_core::AnyTag::Bool => a.as_bool() == b.as_bool(),
        zun_core::AnyTag::Object => a.as_object() == b.as_object(),
    }
}

/// Slots per allocation chunk (objects are bump-allocated within a chunk).
const CHUNK_SLOTS: usize = 64 * 1024;

/// The size of one object slot in bytes.
pub use zun_core::SLOT_SIZE;

/// A pointer to an allocated object: slot 0 is the type RID, slot 1 is the
/// identity hash, and the remaining slots are fields.
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

    /// The stable identity hash assigned by the owning heap.
    pub fn identity_hash(self) -> u64 {
        // SAFETY: a valid Obj points at an allocated block with an identity slot.
        unsafe { *self.0.add(1) }
    }

    /// Read raw field `index` (0-based; slot `index + 1`).
    pub fn field(self, index: usize) -> u64 {
        // SAFETY: callers index within the object's field count (the verifier
        // and layout guarantee this for generated code).
        unsafe { *self.0.add(zun_core::OBJECT_HEADER_SLOTS + index) }
    }

    /// Write raw field `index`.
    pub fn set_field(self, index: usize, value: u64) {
        // SAFETY: see `field`.
        unsafe { *self.0.add(zun_core::OBJECT_HEADER_SLOTS + index) = value }
    }
}

/// Handle into the heap's weak-reference table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeakRefId(u32);

impl WeakRefId {
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy)]
struct WeakSlot {
    target: Option<usize>,
}

/// Handle into the heap's ephemeron table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeakMapId(u32);

/// Opaque registration handle for the finalization queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FinalizationId(u32);

#[derive(Debug, Clone, Copy)]
struct Ephemeron {
    key: Option<usize>,
    value: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
struct FinalizerEntry {
    target: usize,
    token: u64,
    queued: bool,
}

/// Bookkeeping for one allocated object, enabling the sweep phase.
#[derive(Debug)]
struct Record {
    addr: *mut u64,
    field_count: usize,
    alive: bool,
    marked: bool,
}

/// The runtime heap: bump-allocated chunks, object records for mark-sweep, a
/// per-size free list, and a string handle table.
#[derive(Debug, Default)]
pub struct Heap {
    /// Boxed chunks; their contents have stable addresses for the heap's life.
    chunks: Vec<Box<[u64]>>,
    /// Bump cursor within the last chunk.
    cursor: usize,
    /// Interned strings, referenced from field slots by index.
    strings: Vec<Rc<str>>,
    /// Symbols belong to this heap/isolate and never share identity globally.
    symbols: zun_core::SymbolRegistry,
    /// Weak references are cleared after marking and never keep targets alive.
    weak_refs: Vec<WeakSlot>,
    /// WeakMap entries promoted only when their keys are marked.
    weak_maps: Vec<Ephemeron>,
    /// Finalizers are queued after sweep and never run during collection.
    finalizers: Vec<FinalizerEntry>,
    finalization_queue: Vec<u64>,
    /// One record per object ever allocated (dead ones are reused).
    records: Vec<Record>,
    /// Dynamic property bags associated with dynamic objects.
    dynamic_objects: HashMap<usize, zun_core::DynamicObject>,
    /// Contiguous array storage associated with array objects.
    arrays: HashMap<usize, zun_core::ArrayStorage>,
    /// Contiguous byte storage associated with ArrayBuffer objects.
    buffers: HashMap<usize, zun_core::ByteBuffer>,
    /// DataView descriptors associated with managed view objects.
    data_views: HashMap<usize, zun_core::DataView>,
    /// Date values associated with Date objects.
    dates: HashMap<usize, zun_core::DateValue>,
    /// Error values associated with Error objects.
    errors: HashMap<usize, zun_core::ErrorValue>,
    /// Ordered hash tables associated with Map objects.
    maps: HashMap<usize, zun_core::OrderedMap>,
    /// Ordered hash tables associated with Set objects.
    sets: HashMap<usize, zun_core::OrderedMap>,
    /// WeakMap handles owned by managed WeakMap objects.
    weak_map_objects: HashMap<usize, WeakMapId>,
    /// Record index by object address, for tracing child references.
    by_addr: HashMap<usize, usize>,
    /// Dead record indices available for reuse, keyed by field count.
    free: HashMap<usize, Vec<usize>>,
    /// Monotonic identity hashes. Zero is reserved as the uninitialized value.
    next_identity_hash: u64,
}

impl Heap {
    /// Create an empty heap.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate an object with `field_count` fields of type `type_rid`,
    /// returning a pointer to it. Fields start zeroed. A freed block of the
    /// same field count is reused before bump-allocating a new one.
    pub fn alloc(&mut self, type_rid: u32, field_count: usize) -> Obj {
        let identity_hash = self.allocate_identity_hash();
        // Reuse a freed block of the exact size, if any.
        if let Some(idx) = self.free.get_mut(&field_count).and_then(Vec::pop) {
            let addr = self.records[idx].addr;
            // SAFETY: the block is `field_count + 1` valid slots we own.
            unsafe {
                std::ptr::write_bytes(addr, 0, zun_core::OBJECT_HEADER_SLOTS + field_count);
                *addr = type_rid as u64;
                *addr.add(1) = identity_hash;
            }
            self.records[idx].alive = true;
            self.records[idx].marked = false;
            return Obj(addr);
        }

        let slots = zun_core::OBJECT_HEADER_SLOTS + field_count;
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
            *base.add(1) = identity_hash;
        }
        let idx = self.records.len();
        self.records.push(Record {
            addr: base,
            field_count,
            alive: true,
            marked: false,
        });
        self.by_addr.insert(base as usize, idx);
        Obj(base)
    }

    fn allocate_identity_hash(&mut self) -> u64 {
        if self.next_identity_hash == 0 {
            self.next_identity_hash = 1;
        }
        let hash = self.next_identity_hash;
        self.next_identity_hash = self
            .next_identity_hash
            .checked_add(1)
            .expect("identity hash exhausted");
        hash
    }

    /// Allocate an object backed by a dynamic property bag.
    pub fn alloc_dynamic(&mut self) -> Obj {
        let object = self.alloc(0, 0);
        self.dynamic_objects
            .insert(object.addr() as usize, zun_core::DynamicObject::new());
        object
    }

    /// Allocate an array object with contiguous core-owned storage.
    pub fn alloc_array(&mut self, length: usize) -> Obj {
        let object = self.alloc(0, 0);
        self.arrays.insert(
            object.addr() as usize,
            zun_core::ArrayStorage::with_length(length),
        );
        object
    }

    pub fn alloc_buffer(&mut self, length: usize) -> Obj {
        let object = self.alloc(0, 0);
        self.buffers
            .insert(object.addr() as usize, zun_core::ByteBuffer::new(length));
        object
    }

    /// Allocate a DataView whose first field keeps its backing buffer alive.
    pub fn alloc_data_view(&mut self, buffer: Obj, offset: usize, length: usize) -> Option<Obj> {
        let view = zun_core::DataView::new(self.buffer_len(buffer)?, offset, length)?;
        let object = self.alloc(0, 1);
        object.set_field(0, buffer.addr() as u64);
        self.data_views.insert(object.addr() as usize, view);
        Some(object)
    }

    pub fn data_view_get_u32(&self, object: Obj, relative: usize, little_endian: bool) -> Option<u32> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_u32(self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian)
    }

    pub fn data_view_set_u32(&mut self, object: Obj, relative: usize, value: u32, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_u32(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_u16(&self, object: Obj, relative: usize, little_endian: bool) -> Option<u16> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_u16(self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian)
    }

    pub fn data_view_set_u16(&mut self, object: Obj, relative: usize, value: u16, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_u16(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn data_view_get_f64(&self, object: Obj, relative: usize, little_endian: bool) -> Option<f64> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_f64(self.buffers.get(&(buffer.addr() as usize))?, relative, little_endian)
    }

    pub fn data_view_set_f64(&mut self, object: Obj, relative: usize, value: f64, little_endian: bool) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_f64(self.buffers.get_mut(&(buffer.addr() as usize))?, relative, value, little_endian))
    }

    pub fn alloc_date(&mut self, millis: Option<i64>) -> Obj {
        let object = self.alloc(0, 0);
        self.dates.insert(
            object.addr() as usize,
            millis.map_or(
                zun_core::DateValue::invalid(),
                zun_core::DateValue::from_millis,
            ),
        );
        object
    }

    pub fn alloc_date_now(&mut self) -> Obj {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .ok();
        self.alloc_date(millis)
    }

    pub fn alloc_error(&mut self, name: impl Into<String>, message: impl Into<String>) -> Obj {
        let object = self.alloc(0, 0);
        self.errors.insert(
            object.addr() as usize,
            zun_core::ErrorValue::new(name, message),
        );
        object
    }

    pub fn error_name(&self, object: Obj) -> Option<&str> {
        self.errors
            .get(&(object.addr() as usize))
            .map(zun_core::ErrorValue::name)
    }

    pub fn error_message(&self, object: Obj) -> Option<&str> {
        self.errors
            .get(&(object.addr() as usize))
            .map(zun_core::ErrorValue::message)
    }

    pub fn error_set_name(&mut self, object: Obj, name: impl Into<String>) -> Option<()> {
        self.errors
            .get_mut(&(object.addr() as usize))?
            .set_name(name);
        Some(())
    }

    pub fn error_set_message(&mut self, object: Obj, message: impl Into<String>) -> Option<()> {
        self.errors
            .get_mut(&(object.addr() as usize))?
            .set_message(message);
        Some(())
    }

    pub fn date_millis(&self, object: Obj) -> Option<i64> {
        self.dates.get(&(object.addr() as usize))?.millis()
    }

    pub fn date_is_valid(&self, object: Obj) -> Option<bool> {
        Some(self.dates.get(&(object.addr() as usize))?.is_valid())
    }

    pub fn date_set_millis(&mut self, object: Obj, millis: i64) -> Option<()> {
        *self.dates.get_mut(&(object.addr() as usize))? = zun_core::DateValue::from_millis(millis);
        Some(())
    }

    pub fn date_invalidate(&mut self, object: Obj) -> Option<()> {
        *self.dates.get_mut(&(object.addr() as usize))? = zun_core::DateValue::invalid();
        Some(())
    }

    pub fn date_compare(&self, left: Obj, right: Obj) -> Option<std::cmp::Ordering> {
        self.dates
            .get(&(left.addr() as usize))?
            .compare(*self.dates.get(&(right.addr() as usize))?)
    }

    pub fn date_utc_date(&self, object: Obj) -> Option<(i32, u32, u32)> {
        self.dates.get(&(object.addr() as usize))?.utc_date()
    }

    pub fn date_utc_time(&self, object: Obj) -> Option<(u32, u32, u32, u32)> {
        self.dates.get(&(object.addr() as usize))?.utc_time()
    }

    pub fn date_weekday(&self, object: Obj) -> Option<u32> {
        self.dates.get(&(object.addr() as usize))?.weekday()
    }

    pub fn date_ordinal(&self, object: Obj) -> Option<u32> {
        self.dates.get(&(object.addr() as usize))?.ordinal()
    }

    pub fn date_iso(&self, object: Obj) -> Option<String> {
        self.dates.get(&(object.addr() as usize))?.iso_date()
    }

    pub fn date_iso_datetime(&self, object: Obj) -> Option<String> {
        self.dates.get(&(object.addr() as usize))?.iso_datetime()
    }

    pub fn buffer_len(&self, object: Obj) -> Option<usize> {
        self.buffers
            .get(&(object.addr() as usize))
            .map(zun_core::ByteBuffer::len)
    }

    pub fn buffer_get(&self, object: Obj, index: usize) -> Option<u8> {
        self.buffers.get(&(object.addr() as usize))?.get(index)
    }

    pub fn buffer_set(&mut self, object: Obj, index: usize, value: u8) -> Option<bool> {
        Some(
            self.buffers
                .get_mut(&(object.addr() as usize))?
                .set(index, value),
        )
    }

    pub fn buffer_read_u32(&self, object: Obj, offset: usize) -> Option<u32> {
        self.buffers
            .get(&(object.addr() as usize))?
            .read_u32(offset)
    }

    pub fn buffer_write_u32(&mut self, object: Obj, offset: usize, value: u32) -> Option<bool> {
        Some(
            self.buffers
                .get_mut(&(object.addr() as usize))?
                .write_u32(offset, value),
        )
    }

    pub fn buffer_read_f64(&self, object: Obj, offset: usize) -> Option<f64> {
        self.buffers
            .get(&(object.addr() as usize))?
            .read_f64(offset)
    }

    pub fn buffer_write_f64(&mut self, object: Obj, offset: usize, value: f64) -> Option<bool> {
        Some(
            self.buffers
                .get_mut(&(object.addr() as usize))?
                .write_f64(offset, value),
        )
    }

    pub fn buffer_view(
        &self,
        object: Obj,
        offset: usize,
        length: usize,
    ) -> Option<zun_core::ByteView> {
        zun_core::ByteView::new(self.buffer_len(object)?, offset, length)
    }

    /// Create an endian-aware, unaligned view over a managed byte buffer.
    pub fn data_view(
        &self,
        object: Obj,
        offset: usize,
        length: usize,
    ) -> Option<zun_core::DataView> {
        zun_core::DataView::new(self.buffer_len(object)?, offset, length)
    }

    pub fn data_view_read_u16(
        &self,
        object: Obj,
        view: zun_core::DataView,
        offset: usize,
        little_endian: bool,
    ) -> Option<u16> {
        view.get_u16(
            self.buffers.get(&(object.addr() as usize))?,
            offset,
            little_endian,
        )
    }

    pub fn data_view_write_u16(
        &mut self,
        object: Obj,
        view: zun_core::DataView,
        offset: usize,
        value: u16,
        little_endian: bool,
    ) -> Option<bool> {
        Some(view.set_u16(
            self.buffers.get_mut(&(object.addr() as usize))?,
            offset,
            value,
            little_endian,
        ))
    }

    pub fn data_view_read_u32(
        &self,
        object: Obj,
        view: zun_core::DataView,
        offset: usize,
        little_endian: bool,
    ) -> Option<u32> {
        view.get_u32(
            self.buffers.get(&(object.addr() as usize))?,
            offset,
            little_endian,
        )
    }

    pub fn data_view_write_u32(
        &mut self,
        object: Obj,
        view: zun_core::DataView,
        offset: usize,
        value: u32,
        little_endian: bool,
    ) -> Option<bool> {
        Some(view.set_u32(
            self.buffers.get_mut(&(object.addr() as usize))?,
            offset,
            value,
            little_endian,
        ))
    }

    pub fn data_view_read_f64(
        &self,
        object: Obj,
        view: zun_core::DataView,
        offset: usize,
        little_endian: bool,
    ) -> Option<f64> {
        view.get_f64(
            self.buffers.get(&(object.addr() as usize))?,
            offset,
            little_endian,
        )
    }

    pub fn data_view_write_f64(
        &mut self,
        object: Obj,
        view: zun_core::DataView,
        offset: usize,
        value: f64,
        little_endian: bool,
    ) -> Option<bool> {
        Some(view.set_f64(
            self.buffers.get_mut(&(object.addr() as usize))?,
            offset,
            value,
            little_endian,
        ))
    }

    pub fn buffer_view_read_u32(
        &self,
        object: Obj,
        view: zun_core::ByteView,
        offset: usize,
    ) -> Option<u32> {
        view.read_u32(self.buffers.get(&(object.addr() as usize))?, offset)
    }

    pub fn buffer_view_write_u32(
        &mut self,
        object: Obj,
        view: zun_core::ByteView,
        offset: usize,
        value: u32,
    ) -> Option<bool> {
        Some(view.write_u32(
            self.buffers.get_mut(&(object.addr() as usize))?,
            offset,
            value,
        ))
    }

    pub fn buffer_view_read_u8(
        &self,
        object: Obj,
        view: zun_core::TypedArrayView,
        index: usize,
    ) -> Option<u8> {
        view.read_u8(self.buffers.get(&(object.addr() as usize))?, index)
    }

    pub fn buffer_view_write_u8(
        &mut self,
        object: Obj,
        view: zun_core::TypedArrayView,
        index: usize,
        value: u8,
    ) -> Option<bool> {
        Some(view.write_u8(
            self.buffers.get_mut(&(object.addr() as usize))?,
            index,
            value,
        ))
    }

    pub fn buffer_view_read_u16(
        &self,
        object: Obj,
        view: zun_core::TypedArrayView,
        index: usize,
    ) -> Option<u16> {
        view.read_u16(self.buffers.get(&(object.addr() as usize))?, index)
    }

    pub fn buffer_view_write_u16(
        &mut self,
        object: Obj,
        view: zun_core::TypedArrayView,
        index: usize,
        value: u16,
    ) -> Option<bool> {
        Some(view.write_u16(
            self.buffers.get_mut(&(object.addr() as usize))?,
            index,
            value,
        ))
    }

    pub fn buffer_view_read_f64(
        &self,
        object: Obj,
        view: zun_core::TypedArrayView,
        index: usize,
    ) -> Option<f64> {
        view.read_f64(self.buffers.get(&(object.addr() as usize))?, index)
    }

    pub fn buffer_view_write_f64(
        &mut self,
        object: Obj,
        view: zun_core::TypedArrayView,
        index: usize,
        value: f64,
    ) -> Option<bool> {
        Some(view.write_f64(
            self.buffers.get_mut(&(object.addr() as usize))?,
            index,
            value,
        ))
    }

    /// Allocate a runtime-backed Map with insertion-ordered hash storage.
    pub fn alloc_map(&mut self) -> Obj {
        let object = self.alloc(0, 0);
        self.maps
            .insert(object.addr() as usize, zun_core::OrderedMap::new());
        object
    }

    /// Insert a key/value pair and return the replaced value, if any.
    pub fn map_insert(
        &mut self,
        object: Obj,
        key: zun_core::Any,
        value: zun_core::Any,
    ) -> Option<zun_core::Any> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        self.maps
            .get_mut(&(object.addr() as usize))?
            .insert(key, value, hash, |candidate| {
                any_same_value_zero(candidate, key, strings)
            })
    }

    /// Look up a key using SameValueZero equality.
    pub fn map_get(&self, object: Obj, key: zun_core::Any) -> Option<zun_core::Any> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        self.maps
            .get(&(object.addr() as usize))?
            .get(hash, |candidate| {
                any_same_value_zero(candidate, key, strings)
            })
    }

    pub fn map_has(&self, object: Obj, key: zun_core::Any) -> Option<bool> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        Some(
            self.maps
                .get(&(object.addr() as usize))?
                .contains(hash, |candidate| {
                    any_same_value_zero(candidate, key, strings)
                }),
        )
    }

    pub fn map_clear(&mut self, object: Obj) -> Option<()> {
        self.maps.get_mut(&(object.addr() as usize))?.clear();
        Some(())
    }

    /// Remove a key and return its key/value pair.
    pub fn map_remove(
        &mut self,
        object: Obj,
        key: zun_core::Any,
    ) -> Option<(zun_core::Any, zun_core::Any)> {
        let hash = self.any_hash(key);
        let strings = &self.strings;
        self.maps
            .get_mut(&(object.addr() as usize))?
            .remove(hash, |candidate| {
                any_same_value_zero(candidate, key, strings)
            })
    }

    pub fn map_len(&self, object: Obj) -> Option<usize> {
        self.maps
            .get(&(object.addr() as usize))
            .map(zun_core::OrderedMap::len)
    }

    pub fn map_entries(&self, object: Obj) -> Option<Vec<(zun_core::Any, zun_core::Any)>> {
        Some(self.maps.get(&(object.addr() as usize))?.iter().collect())
    }

    /// Allocate a runtime-backed Set with insertion-ordered hash storage.
    pub fn alloc_set(&mut self) -> Obj {
        let object = self.alloc(0, 0);
        self.sets
            .insert(object.addr() as usize, zun_core::OrderedMap::new());
        object
    }

    /// Add a value and report whether it was newly inserted.
    pub fn set_add(&mut self, object: Obj, value: zun_core::Any) -> Option<bool> {
        let hash = self.any_hash(value);
        let strings = &self.strings;
        let set = self.sets.get_mut(&(object.addr() as usize))?;
        let existed = set
            .get(hash, |candidate| {
                any_same_value_zero(candidate, value, strings)
            })
            .is_some();
        if !existed {
            set.insert(value, zun_core::Any::bool(true), hash, |candidate| {
                any_same_value_zero(candidate, value, strings)
            });
        }
        Some(!existed)
    }

    pub fn set_has(&self, object: Obj, value: zun_core::Any) -> Option<bool> {
        let hash = self.any_hash(value);
        let strings = &self.strings;
        Some(
            self.sets
                .get(&(object.addr() as usize))?
                .get(hash, |candidate| {
                    any_same_value_zero(candidate, value, strings)
                })
                .is_some(),
        )
    }

    pub fn set_remove(&mut self, object: Obj, value: zun_core::Any) -> Option<bool> {
        let hash = self.any_hash(value);
        let strings = &self.strings;
        Some(
            self.sets
                .get_mut(&(object.addr() as usize))?
                .remove(hash, |candidate| {
                    any_same_value_zero(candidate, value, strings)
                })
                .is_some(),
        )
    }

    pub fn set_len(&self, object: Obj) -> Option<usize> {
        self.sets
            .get(&(object.addr() as usize))
            .map(zun_core::OrderedMap::len)
    }

    pub fn set_clear(&mut self, object: Obj) -> Option<()> {
        self.sets.get_mut(&(object.addr() as usize))?.clear();
        Some(())
    }

    pub fn set_values(&self, object: Obj) -> Option<Vec<zun_core::Any>> {
        Some(
            self.sets
                .get(&(object.addr() as usize))?
                .iter()
                .map(|(key, _)| key)
                .collect(),
        )
    }

    fn any_hash(&self, value: zun_core::Any) -> u64 {
        match value.tag() {
            zun_core::AnyTag::Int => value.as_int().unwrap() as u64 ^ 0x11,
            zun_core::AnyTag::Float => value.as_float().unwrap().to_bits() ^ 0x22,
            zun_core::AnyTag::Bool => value.as_bool().unwrap() as u64 ^ 0x33,
            zun_core::AnyTag::Object => {
                unsafe { Obj::from_addr(value.as_object().unwrap() as *mut u64) }.identity_hash()
                    ^ 0x44
            }
            zun_core::AnyTag::String => {
                self.try_string(value.as_string().unwrap())
                    .map_or(0, |s| hash_bytes(s.as_bytes()))
                    ^ 0x55
            }
        }
    }

    /// Return the length of an array object.
    pub fn array_len(&self, object: Obj) -> Option<usize> {
        self.arrays
            .get(&(object.addr() as usize))
            .map(zun_core::ArrayStorage::len)
    }

    /// Read an array element with bounds checking.
    pub fn array_get(&self, object: Obj, index: usize) -> Option<zun_core::Any> {
        self.arrays.get(&(object.addr() as usize))?.get(index)
    }

    /// Write an array element with bounds checking.
    pub fn array_set(
        &mut self,
        object: Obj,
        index: usize,
        value: zun_core::Any,
    ) -> Option<zun_core::Any> {
        self.arrays
            .get_mut(&(object.addr() as usize))?
            .set(index, value)
    }

    /// Insert an array element with bounds checking.
    pub fn array_insert(&mut self, object: Obj, index: usize, value: zun_core::Any) -> bool {
        self.arrays
            .get_mut(&(object.addr() as usize))
            .is_some_and(|array| array.insert(index, value))
    }

    /// Remove an array element with bounds checking.
    pub fn array_remove(&mut self, object: Obj, index: usize) -> Option<zun_core::Any> {
        self.arrays
            .get_mut(&(object.addr() as usize))?
            .remove(index)
    }

    /// Remove and return an array's last element.
    pub fn array_pop(&mut self, object: Obj) -> Option<zun_core::Any> {
        self.arrays.get_mut(&(object.addr() as usize))?.pop()
    }

    /// Append an element and return the resulting array length.
    pub fn array_push(&mut self, object: Obj, value: zun_core::Any) -> Option<usize> {
        let array = self.arrays.get_mut(&(object.addr() as usize))?;
        array.push(value);
        Some(array.len())
    }

    /// Read a dynamic property from an object allocated by [`Self::alloc_dynamic`].
    pub fn dynamic_get(&self, object: Obj, name: &str) -> Option<zun_core::Any> {
        self.dynamic_objects
            .get(&(object.addr() as usize))
            .and_then(|properties| properties.get(name))
    }

    /// Insert or replace a dynamic property.
    pub fn dynamic_set(
        &mut self,
        object: Obj,
        name: impl Into<String>,
        value: zun_core::Any,
    ) -> Option<zun_core::Any> {
        self.dynamic_objects
            .get_mut(&(object.addr() as usize))?
            .set(name, value)
    }

    /// Remove a dynamic property.
    pub fn dynamic_remove(&mut self, object: Obj, name: &str) -> Option<zun_core::Any> {
        self.dynamic_objects
            .get_mut(&(object.addr() as usize))?
            .remove(name)
    }

    /// Number of currently live objects (for diagnostics and tests).
    pub fn live_count(&self) -> usize {
        self.records.iter().filter(|r| r.alive).count()
    }

    /// Mark-sweep collection. `roots` are the object references currently live
    /// on the stacks; `ref_slots(type_rid)` gives the field-slot indices of an
    /// object's reference fields (so the collector can trace the graph).
    /// Returns how many objects were reclaimed.
    ///
    /// # Safety
    /// Every root and every traced reference must be a valid object in this
    /// heap. In particular, no live stack slot (interpreter or JIT) may hold an
    /// object reference absent from `roots` — otherwise it could be freed while
    /// still in use. (Collecting with precise roots from both engines is the
    /// remaining integration; see PLAN.md Â§4.3.)
    pub unsafe fn collect(
        &mut self,
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
    ) -> usize {
        for r in &mut self.records {
            r.marked = false;
        }

        // Mark reachable objects from the roots.
        let mut work: Vec<usize> = Vec::new();
        for root in roots {
            if let Some(&idx) = self.by_addr.get(&(root.0 as usize)) {
                if !self.records[idx].marked {
                    self.records[idx].marked = true;
                    work.push(idx);
                }
            }
        }
        while let Some(idx) = work.pop() {
            let obj = Obj(self.records[idx].addr);
            for slot in ref_slots(obj.type_rid()) {
                let child = obj.field(slot) as *mut u64;
                if child.is_null() {
                    continue;
                }
                if let Some(&cidx) = self.by_addr.get(&(child as usize)) {
                    if !self.records[cidx].marked {
                        self.records[cidx].marked = true;
                        work.push(cidx);
                    }
                }
            }
            if let Some(properties) = self.dynamic_objects.get(&(obj.addr() as usize)) {
                // Dynamic properties can hold object-tagged Any values.
                let dynamic_children: Vec<usize> = properties.object_references().collect();
                for child in dynamic_children {
                    if let Some(&cidx) = self.by_addr.get(&child) {
                        if !self.records[cidx].marked {
                            self.records[cidx].marked = true;
                            work.push(cidx);
                        }
                    }
                }
            }
            if let Some(array) = self.arrays.get(&(obj.addr() as usize)) {
                for child in array.object_references() {
                    if let Some(&cidx) = self.by_addr.get(&child) {
                        if !self.records[cidx].marked {
                            self.records[cidx].marked = true;
                            work.push(cidx);
                        }
                    }
                }
            }
            if let Some(map) = self.maps.get(&(obj.addr() as usize)) {
                for (key, value) in map.iter() {
                    for child in [key, value].into_iter().filter_map(|item| item.as_object()) {
                        if let Some(&cidx) = self.by_addr.get(&child) {
                            if !self.records[cidx].marked {
                                self.records[cidx].marked = true;
                                work.push(cidx);
                            }
                        }
                    }
                }
            }
            if let Some(set) = self.sets.get(&(obj.addr() as usize)) {
                for (key, _) in set.iter() {
                    if let Some(&cidx) = key.as_object().and_then(|child| self.by_addr.get(&child))
                    {
                        if !self.records[cidx].marked {
                            self.records[cidx].marked = true;
                            work.push(cidx);
                        }
                    }
                }
            }
        }

        // Ephemeron fixed point: a marked key promotes its value, while strong
        // fields of promoted values can make later keys reachable.
        loop {
            let mut promoted = false;
            for entry in &self.weak_maps {
                let (Some(key), Some(value)) = (entry.key, entry.value) else {
                    continue;
                };
                let Some(&key_idx) = self.by_addr.get(&key) else {
                    continue;
                };
                let Some(&value_idx) = self.by_addr.get(&value) else {
                    continue;
                };
                if self.records[key_idx].marked && !self.records[value_idx].marked {
                    self.records[value_idx].marked = true;
                    work.push(value_idx);
                    promoted = true;
                }
            }
            while let Some(idx) = work.pop() {
                let obj = Obj(self.records[idx].addr);
                for slot in ref_slots(obj.type_rid()) {
                    let child = obj.field(slot) as *mut u64;
                    if let Some(&child_idx) = self.by_addr.get(&(child as usize)) {
                        if !self.records[child_idx].marked {
                            self.records[child_idx].marked = true;
                            work.push(child_idx);
                        }
                    }
                }
                if let Some(properties) = self.dynamic_objects.get(&(obj.addr() as usize)) {
                    for child in properties.object_references() {
                        if let Some(&child_idx) = self.by_addr.get(&child) {
                            if !self.records[child_idx].marked {
                                self.records[child_idx].marked = true;
                                work.push(child_idx);
                            }
                        }
                    }
                }
                if let Some(array) = self.arrays.get(&(obj.addr() as usize)) {
                    for child in array.object_references() {
                        if let Some(&child_idx) = self.by_addr.get(&child) {
                            if !self.records[child_idx].marked {
                                self.records[child_idx].marked = true;
                                work.push(child_idx);
                            }
                        }
                    }
                }
                if let Some(map) = self.maps.get(&(obj.addr() as usize)) {
                    for (key, value) in map.iter() {
                        for child in [key, value].into_iter().filter_map(|item| item.as_object()) {
                            if let Some(&child_idx) = self.by_addr.get(&child) {
                                if !self.records[child_idx].marked {
                                    self.records[child_idx].marked = true;
                                    work.push(child_idx);
                                }
                            }
                        }
                    }
                }
                if let Some(set) = self.sets.get(&(obj.addr() as usize)) {
                    for (key, _) in set.iter() {
                        if let Some(&child_idx) =
                            key.as_object().and_then(|child| self.by_addr.get(&child))
                        {
                            if !self.records[child_idx].marked {
                                self.records[child_idx].marked = true;
                                work.push(child_idx);
                            }
                        }
                    }
                }
            }
            if !promoted {
                break;
            }
        }

        // Weak references observe the completed mark set but do not contribute
        // roots. Clear dead targets before sweep removes their addresses.
        for weak in &mut self.weak_refs {
            if let Some(address) = weak.target {
                if self
                    .by_addr
                    .get(&address)
                    .is_none_or(|&idx| !self.records[idx].marked)
                {
                    weak.target = None;
                }
            }
        }

        // Sweep: reclaim live-but-unmarked objects into the free list.
        let mut freed = 0;
        for idx in 0..self.records.len() {
            let r = &mut self.records[idx];
            if r.alive && !r.marked {
                r.alive = false;
                self.by_addr.remove(&(r.addr as usize));
                self.dynamic_objects.remove(&(r.addr as usize));
                self.arrays.remove(&(r.addr as usize));
                self.buffers.remove(&(r.addr as usize));
                self.data_views.remove(&(r.addr as usize));
                self.dates.remove(&(r.addr as usize));
                self.errors.remove(&(r.addr as usize));
                self.maps.remove(&(r.addr as usize));
                self.sets.remove(&(r.addr as usize));
                self.weak_map_objects.remove(&(r.addr as usize));
                self.free.entry(r.field_count).or_default().push(idx);
                freed += 1;
            }
        }
        for entry in &mut self.finalizers {
            if !entry.queued && self.by_addr.get(&entry.target).is_none() {
                entry.queued = true;
                self.finalization_queue.push(entry.token);
            }
        }
        freed
    }

    /// Collect while treating captured environments of function values as
    /// additional roots.
    ///
    /// # Safety
    /// The same root validity requirements as [`Heap::collect`] apply. Every
    /// non-zero environment address in `functions` must identify a live object
    /// in this heap.
    pub unsafe fn collect_with_function_roots(
        &mut self,
        roots: &[Obj],
        functions: &[zun_core::FunctionHandle],
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
    ) -> usize {
        let mut all_roots = roots.to_vec();
        all_roots.extend(functions.iter().filter_map(|function| {
            function
                .environment()
                .map(|address| unsafe { Obj::from_addr(address as *mut u64) })
        }));
        // SAFETY: the caller guarantees all explicit and captured roots are
        // valid objects in this heap.
        unsafe { self.collect(&all_roots, ref_slots) }
    }

    /// Collect using the roots registered by active native frames in addition
    /// to the explicitly supplied interpreter roots.
    ///
    /// # Safety
    /// The same root validity requirements as [`Heap::collect`] apply.
    pub unsafe fn collect_with_native_roots(
        &mut self,
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
    ) -> usize {
        let mut all_roots = roots.to_vec();
        append_native_roots(&mut all_roots);
        // SAFETY: the caller guarantees the explicit roots and registered
        // native roots are valid for this heap.
        unsafe { self.collect(&all_roots, ref_slots) }
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

    /// Resolve a string handle without panicking on malformed dynamic data.
    pub fn try_string(&self, handle: u32) -> Option<&Rc<str>> {
        self.strings.get(handle as usize)
    }

    /// Create a unique symbol in this heap's isolate.
    pub fn symbol_create(&mut self, description: Option<impl Into<String>>) -> zun_core::SymbolId {
        self.symbols.create(description)
    }

    /// Get the canonical symbol for a registry key in this isolate.
    pub fn symbol_for(&mut self, key: impl Into<String>) -> zun_core::SymbolId {
        self.symbols.for_key(key)
    }

    pub fn symbol_description(&self, symbol: zun_core::SymbolId) -> Option<&str> {
        self.symbols.description(symbol)
    }

    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }

    pub fn weak_ref_new(&mut self, target: Obj) -> WeakRefId {
        let id = WeakRefId(self.weak_refs.len() as u32);
        self.weak_refs.push(WeakSlot {
            target: Some(target.addr() as usize),
        });
        id
    }

    pub fn weak_ref_get(&self, id: WeakRefId) -> Option<Obj> {
        let address = self.weak_refs.get(id.0 as usize)?.target?;
        self.by_addr
            .get(&address)
            .map(|&index| Obj(self.records[index].addr))
    }

    pub fn weak_map_new(&mut self) -> WeakMapId {
        let id = WeakMapId(self.weak_maps.len() as u32);
        self.weak_maps.push(Ephemeron {
            key: None,
            value: None,
        });
        id
    }

    /// Allocate the managed object used as a source-level WeakMap handle.
    pub fn alloc_weak_map(&mut self) -> Obj {
        let object = self.alloc(0, 0);
        let id = self.weak_map_new();
        self.weak_map_objects.insert(object.addr() as usize, id);
        object
    }

    pub fn weak_map_set_object(&mut self, object: Obj, key: Obj, value: Obj) -> bool {
        self.weak_map_objects
            .get(&(object.addr() as usize))
            .copied()
            .is_some_and(|id| self.weak_map_set(id, key, value))
    }

    pub fn weak_map_get_object(&self, object: Obj) -> Option<(Obj, Obj)> {
        let id = self.weak_map_objects.get(&(object.addr() as usize)).copied()?;
        self.weak_map_get(id)
    }

    pub fn weak_map_set(&mut self, id: WeakMapId, key: Obj, value: Obj) -> bool {
        let Some(entry) = self.weak_maps.get_mut(id.0 as usize) else {
            return false;
        };
        entry.key = Some(key.addr() as usize);
        entry.value = Some(value.addr() as usize);
        true
    }

    pub fn weak_map_get(&self, id: WeakMapId) -> Option<(Obj, Obj)> {
        let entry = self.weak_maps.get(id.0 as usize)?;
        let key = self.by_addr.get(&entry.key?)?;
        let value = self.by_addr.get(&entry.value?)?;
        Some((Obj(self.records[*key].addr), Obj(self.records[*value].addr)))
    }

    pub fn finalization_register(&mut self, target: Obj, token: u64) -> FinalizationId {
        let id = FinalizationId(self.finalizers.len() as u32);
        self.finalizers.push(FinalizerEntry {
            target: target.addr() as usize,
            token,
            queued: false,
        });
        id
    }

    /// Drain tokens whose targets were reclaimed by the last collection.
    pub fn finalization_drain(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.finalization_queue)
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
pub unsafe extern "C" fn zun_gc_alloc(
    heap: *mut Heap,
    type_rid: u32,
    field_count: usize,
) -> *mut u64 {
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
    /// Native frames currently active above the interpreter. Each entry is a
    /// borrowed root slice registered by a short-lived [`NativeRootScope`].
    static NATIVE_ROOTS: std::cell::RefCell<Vec<(*const Obj, usize)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Guard that exposes object references held by a native/JIT frame to GC.
///
/// The slice must remain alive and unchanged until this guard is dropped.
#[derive(Debug)]
pub struct NativeRootScope {
    _private: (),
}

impl NativeRootScope {
    /// Register `roots` for the duration of a native call.
    ///
    /// # Safety
    /// `roots` must remain allocated and unchanged until the returned guard is
    /// dropped. The caller must also ensure every entry belongs to the current
    /// heap.
    pub unsafe fn enter(roots: &[Obj]) -> Self {
        NATIVE_ROOTS.with(|registered| {
            registered.borrow_mut().push((roots.as_ptr(), roots.len()));
        });
        Self { _private: () }
    }
}

impl Drop for NativeRootScope {
    fn drop(&mut self) {
        NATIVE_ROOTS.with(|registered| {
            registered.borrow_mut().pop();
        });
    }
}

/// Append roots registered by active native frames to a GC root set.
pub fn append_native_roots(roots: &mut Vec<Obj>) {
    NATIVE_ROOTS.with(|registered| {
        for &(ptr, len) in registered.borrow().iter() {
            // SAFETY: each scope promises that its borrowed slice remains
            // valid until the scope is dropped.
            roots.extend_from_slice(unsafe { std::slice::from_raw_parts(ptr, len) });
        }
    });
}

/// A native safepoint description published by a code generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeStackMap {
    /// Absolute program counter of the safepoint.
    pub pc: usize,
    /// Number of bytes covered by the native frame map.
    pub span: u32,
}

/// Registry of native safepoint maps known to the GC boundary.
#[derive(Debug, Default)]
pub struct StackMapRegistry {
    maps: Vec<NativeStackMap>,
}

impl StackMapRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the registered map set with the maps from the current JIT image.
    pub fn replace(&mut self, maps: impl IntoIterator<Item = NativeStackMap>) {
        self.maps.clear();
        self.maps.extend(maps);
        self.maps.sort_unstable_by_key(|map| map.pc);
    }

    /// Find the map covering `pc`, if one is registered.
    pub fn lookup(&self, pc: usize) -> Option<NativeStackMap> {
        let index = match self.maps.binary_search_by_key(&pc, |map| map.pc) {
            Ok(index) => index,
            Err(0) => return None,
            Err(index) => index - 1,
        };
        let map = self.maps[index];
        let end = map.pc.saturating_add(map.span as usize);
        (pc < end).then_some(map)
    }

    /// Number of registered safepoints.
    pub fn len(&self) -> usize {
        self.maps.len()
    }

    /// Whether the registry has no safepoints.
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }
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
        assert_ne!(o.identity_hash(), 0);
        assert_eq!(o.field(0), 0);
        assert_eq!(o.field(1), 0);
        assert_eq!(o.field(2), 0);
    }

    #[test]
    fn function_environment_is_kept_as_a_gc_root() {
        let mut heap = Heap::new();
        let environment = heap.alloc(3, 0);
        let garbage = heap.alloc(4, 0);
        let function = zun_core::FunctionHandle::new(11, Some(environment.addr() as usize));

        let freed = unsafe { heap.collect_with_function_roots(&[], &[function], &|_| Vec::new()) };
        assert_eq!(freed, 1);
        assert_eq!(heap.live_count(), 1);
        assert!(heap.by_addr.contains_key(&(environment.addr() as usize)));
        assert!(!heap.by_addr.contains_key(&(garbage.addr() as usize)));
    }

    #[test]
    fn map_uses_same_value_zero_and_preserves_entries() {
        let mut heap = Heap::new();
        let map = heap.alloc_map();
        let one = heap.intern_string(Rc::from("one"));
        assert_eq!(
            heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::string(one)),
            None
        );
        assert_eq!(
            heap.map_insert(
                map,
                zun_core::Any::float(f64::NAN),
                zun_core::Any::bool(true)
            ),
            None
        );
        assert_eq!(
            heap.map_get(map, zun_core::Any::int(1))
                .and_then(|v| v.as_string())
                .and_then(|h| heap.try_string(h))
                .map(|s| s.as_ref()),
            Some("one")
        );
        assert_eq!(
            heap.map_get(map, zun_core::Any::float(f64::NAN))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(heap.map_len(map), Some(2));
        assert_eq!(
            heap.map_entries(map)
                .unwrap()
                .iter()
                .filter_map(|(key, _)| key.as_int())
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(heap.map_has(map, zun_core::Any::int(1)), Some(true));
        assert_eq!(heap.map_clear(map), Some(()));
        assert_eq!(heap.map_len(map), Some(0));
        assert_eq!(heap.map_has(map, zun_core::Any::int(1)), Some(false));
        let one = heap.intern_string(Rc::from("one-again"));
        heap.map_insert(map, zun_core::Any::int(1), zun_core::Any::string(one));
        assert_eq!(
            heap.map_remove(map, zun_core::Any::int(1))
                .and_then(|(_, value)| value.as_string()),
            Some(one)
        );
        assert_eq!(heap.map_len(map), Some(0));
    }

    #[test]
    fn map_traces_object_keys_and_values() {
        let mut heap = Heap::new();
        let map = heap.alloc_map();
        let key = heap.alloc(1, 0);
        let value = heap.alloc(2, 0);
        heap.map_insert(
            map,
            zun_core::Any::object(key.addr() as usize),
            zun_core::Any::object(value.addr() as usize),
        );
        let no_refs = |_: u32| Vec::<usize>::new();
        let freed = unsafe { heap.collect(&[map], &no_refs) };
        assert_eq!(freed, 0);
        assert_eq!(heap.live_count(), 3);
    }

    #[test]
    fn set_has_same_value_zero_and_traces_object_keys() {
        let mut heap = Heap::new();
        let set = heap.alloc_set();
        let key = heap.alloc(1, 0);
        assert_eq!(
            heap.set_add(set, zun_core::Any::object(key.addr() as usize)),
            Some(true)
        );
        assert_eq!(
            heap.set_add(set, zun_core::Any::object(key.addr() as usize)),
            Some(false)
        );
        assert_eq!(
            heap.set_has(set, zun_core::Any::object(key.addr() as usize)),
            Some(true)
        );
        assert_eq!(heap.set_len(set), Some(1));
        assert_eq!(
            heap.set_values(set)
                .unwrap()
                .iter()
                .filter_map(|value| value.as_object())
                .count(),
            1
        );
        assert_eq!(heap.set_clear(set), Some(()));
        assert_eq!(heap.set_len(set), Some(0));
        assert_eq!(
            heap.set_remove(set, zun_core::Any::object(key.addr() as usize)),
            Some(false)
        );
        assert_eq!(
            heap.set_has(set, zun_core::Any::object(key.addr() as usize)),
            Some(false)
        );
    }

    #[test]
    fn symbols_are_isolate_local_heap_state() {
        let mut first = Heap::new();
        let mut second = Heap::new();
        let first_symbol = first.symbol_for("iterator");
        let second_symbol = second.symbol_for("iterator");
        assert_eq!(first_symbol, second_symbol);
        assert_eq!(first.symbol_description(first_symbol), Some("iterator"));
        assert_eq!(first.symbol_count(), 1);
        assert_eq!(second.symbol_count(), 1);
        let unique = first.symbol_create(Some("iterator"));
        assert_ne!(unique, first_symbol);
    }

    #[test]
    fn weak_reference_does_not_keep_target_alive() {
        let mut heap = Heap::new();
        let target = heap.alloc(1, 0);
        let weak = heap.weak_ref_new(target);
        assert_eq!(heap.weak_ref_get(weak), Some(target));
        let freed = unsafe { heap.collect(&[], &|_| Vec::new()) };
        assert_eq!(freed, 1);
        assert_eq!(heap.weak_ref_get(weak), None);
    }

    #[test]
    fn weak_reference_observes_a_live_root() {
        let mut heap = Heap::new();
        let target = heap.alloc(1, 0);
        let weak = heap.weak_ref_new(target);
        assert_eq!(unsafe { heap.collect(&[target], &|_| Vec::new()) }, 0);
        assert_eq!(heap.weak_ref_get(weak), Some(target));
    }

    #[test]
    fn weak_map_promotes_value_only_when_key_is_live() {
        let mut heap = Heap::new();
        let weak_map = heap.weak_map_new();
        let key = heap.alloc(1, 0);
        let value = heap.alloc(2, 0);
        assert!(heap.weak_map_set(weak_map, key, value));
        assert_eq!(unsafe { heap.collect(&[key], &|_| Vec::new()) }, 0);
        assert_eq!(heap.weak_map_get(weak_map), Some((key, value)));

        let key2 = heap.alloc(3, 0);
        let value2 = heap.alloc(4, 0);
        assert!(heap.weak_map_set(weak_map, key2, value2));
        assert_eq!(unsafe { heap.collect(&[key], &|_| Vec::new()) }, 3);
        assert_eq!(heap.weak_map_get(weak_map), None);
    }

    #[test]
    fn weak_map_promotion_traces_dynamic_value_children() {
        let mut heap = Heap::new();
        let weak_map = heap.weak_map_new();
        let key = heap.alloc(1, 0);
        let value = heap.alloc_dynamic();
        let child = heap.alloc(2, 0);
        heap.dynamic_set(value, "child", zun_core::Any::object(child.addr() as usize));
        heap.weak_map_set(weak_map, key, value);
        assert_eq!(unsafe { heap.collect(&[key], &|_| Vec::new()) }, 0);
        assert_eq!(heap.live_count(), 3);
    }

    #[test]
    fn weak_map_ephemerons_survive_repeated_collections_only_for_live_keys() {
        let mut heap = Heap::new();
        let mut handles = Vec::new();
        let mut live_keys = Vec::new();
        for index in 0..64u32 {
            let weak_map = heap.weak_map_new();
            let key = heap.alloc(index + 1, 0);
            let value = heap.alloc(index + 100, 0);
            assert!(heap.weak_map_set(weak_map, key, value));
            if index % 2 == 0 {
                live_keys.push(key);
            }
            handles.push((weak_map, key, value));
        }

        let no_refs = |_: u32| Vec::<usize>::new();
        for _ in 0..4 {
            unsafe { heap.collect(&live_keys, &no_refs) };
        }
        for (index, (weak_map, key, value)) in handles.into_iter().enumerate() {
            if index % 2 == 0 {
                assert_eq!(heap.weak_map_get(weak_map), Some((key, value)));
            } else {
                assert_eq!(heap.weak_map_get(weak_map), None);
            }
        }
    }

    #[test]
    fn finalization_queues_token_after_target_is_swept() {
        let mut heap = Heap::new();
        let target = heap.alloc(1, 0);
        heap.finalization_register(target, 99);
        assert!(heap.finalization_drain().is_empty());
        assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 1);
        assert_eq!(heap.finalization_drain(), vec![99]);
        assert!(heap.finalization_drain().is_empty());
    }

    #[test]
    fn byte_buffers_roundtrip_through_the_heap() {
        let mut heap = Heap::new();
        let buffer = heap.alloc_buffer(4);
        assert_eq!(heap.buffer_len(buffer), Some(4));
        assert_eq!(heap.buffer_set(buffer, 2, 0xfe), Some(true));
        assert_eq!(heap.buffer_get(buffer, 2), Some(0xfe));
        assert_eq!(heap.buffer_set(buffer, 4, 1), Some(false));
        assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 1);
        assert_eq!(heap.buffer_len(buffer), None);
    }

    #[test]
    fn typed_buffer_views_roundtrip_through_the_heap() {
        let mut heap = Heap::new();
        let buffer = heap.alloc_buffer(16);
        assert_eq!(heap.buffer_write_u32(buffer, 0, 0xfeed_beef), Some(true));
        assert_eq!(heap.buffer_read_u32(buffer, 0), Some(0xfeed_beef));
        assert_eq!(heap.buffer_write_f64(buffer, 8, 2.5), Some(true));
        assert_eq!(heap.buffer_read_f64(buffer, 8), Some(2.5));
        assert_eq!(heap.buffer_write_u32(buffer, 14, 1), Some(false));
        let view = heap.buffer_view(buffer, 4, 8).unwrap();
        assert_eq!(
            heap.buffer_view_write_u32(buffer, view, 0, 0x1234),
            Some(true)
        );
        assert_eq!(heap.buffer_view_read_u32(buffer, view, 0), Some(0x1234));
        assert_eq!(heap.buffer_view_write_u32(buffer, view, 5, 1), Some(false));
        let bytes = zun_core::TypedArrayView::new(16, 0, 4, zun_core::TypedElement::U8).unwrap();
        assert_eq!(heap.buffer_view_write_u8(buffer, bytes, 0, 7), Some(true));
        assert_eq!(heap.buffer_view_read_u8(buffer, bytes, 0), Some(7));
        let shorts = zun_core::TypedArrayView::new(16, 0, 4, zun_core::TypedElement::U16).unwrap();
        assert_eq!(
            heap.buffer_view_write_u16(buffer, shorts, 1, 0xbeef),
            Some(true)
        );
        assert_eq!(heap.buffer_view_read_u16(buffer, shorts, 1), Some(0xbeef));
        let floats = zun_core::TypedArrayView::new(16, 8, 8, zun_core::TypedElement::F64).unwrap();
        assert_eq!(
            heap.buffer_view_write_f64(buffer, floats, 0, 1.5),
            Some(true)
        );
        assert_eq!(heap.buffer_view_read_f64(buffer, floats, 0), Some(1.5));
        let data = heap.data_view(buffer, 1, 10).unwrap();
        assert_eq!(
            heap.data_view_write_u16(buffer, data, 1, 0x1234, true),
            Some(true)
        );
        assert_eq!(heap.data_view_read_u16(buffer, data, 1, true), Some(0x1234));
        assert_eq!(
            heap.data_view_read_u16(buffer, data, 1, false),
            Some(0x3412)
        );
        assert_eq!(
            heap.data_view_write_u32(buffer, data, 3, 0xdead_beef, false),
            Some(true)
        );
        assert_eq!(
            heap.data_view_read_u32(buffer, data, 3, false),
            Some(0xdead_beef)
        );
        assert_eq!(
            heap.data_view_write_f64(buffer, data, 2, -3.5, true),
            Some(true)
        );
        assert_eq!(heap.data_view_read_f64(buffer, data, 2, true), Some(-3.5));
        assert_eq!(
            heap.data_view_write_u32(buffer, data, 7, 1, true),
            Some(false)
        );
    }

    #[test]
    fn date_values_roundtrip_and_invalid_dates_do_not_compare() {
        let mut heap = Heap::new();
        let earlier = heap.alloc_date(Some(100));
        let later = heap.alloc_date(Some(200));
        let invalid = heap.alloc_date(None);
        assert_eq!(heap.date_millis(earlier), Some(100));
        assert_eq!(
            heap.date_compare(earlier, later),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(heap.date_utc_date(earlier), Some((1970, 1, 1)));
        assert_eq!(heap.date_utc_time(earlier), Some((0, 0, 0, 100)));
        assert_eq!(heap.date_weekday(earlier), Some(4));
        assert_eq!(heap.date_ordinal(earlier), Some(1));
        assert_eq!(heap.date_iso(earlier).as_deref(), Some("1970-01-01"));
        assert_eq!(
            heap.date_iso_datetime(earlier).as_deref(),
            Some("1970-01-01T00:00:00.100Z")
        );
        assert_eq!(heap.date_set_millis(earlier, 200), Some(()));
        assert_eq!(heap.date_millis(earlier), Some(200));
        assert_eq!(heap.date_invalidate(earlier), Some(()));
        assert_eq!(heap.date_millis(earlier), None);
        assert_eq!(heap.date_millis(invalid), None);
        assert_eq!(heap.date_is_valid(invalid), Some(false));
        let now = heap.alloc_date_now();
        assert_eq!(heap.date_is_valid(now), Some(true));
        assert!(heap.date_millis(now).is_some());
        assert_eq!(heap.date_compare(earlier, invalid), None);
    }

    #[test]
    fn error_values_roundtrip_through_the_heap() {
        let mut heap = Heap::new();
        let error = heap.alloc_error("TypeError", "bad value");
        assert_eq!(heap.error_name(error), Some("TypeError"));
        assert_eq!(heap.error_message(error), Some("bad value"));
        assert_eq!(heap.error_set_name(error, "RangeError"), Some(()));
        assert_eq!(heap.error_name(error), Some("RangeError"));
        assert_eq!(heap.error_set_message(error, "fixed"), Some(()));
        assert_eq!(heap.error_message(error), Some("fixed"));
        assert_eq!(unsafe { heap.collect(&[], &|_| Vec::new()) }, 1);
        assert_eq!(heap.error_message(error), None);
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
        assert!(
            heap.chunk_count() > 1,
            "should have spanned multiple chunks"
        );
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
    fn dynamic_properties_roundtrip_through_the_heap() {
        let mut heap = Heap::new();
        let object = heap.alloc_dynamic();
        assert_eq!(object.type_rid(), 0);
        assert_eq!(
            heap.dynamic_set(object, "answer", zun_core::Any::int(42)),
            None
        );
        assert_eq!(
            heap.dynamic_get(object, "answer")
                .and_then(zun_core::Any::as_int),
            Some(42)
        );
        assert_eq!(
            heap.dynamic_remove(object, "answer")
                .and_then(zun_core::Any::as_int),
            Some(42)
        );
    }

    #[test]
    fn dynamic_object_references_are_traced_by_collection() {
        let mut heap = Heap::new();
        let child = heap.alloc(7, 0);
        let dynamic = heap.alloc_dynamic();
        heap.dynamic_set(
            dynamic,
            "child",
            zun_core::Any::object(child.addr() as usize),
        );
        let unreachable = heap.alloc(8, 0);
        let no_refs = |_: u32| Vec::<usize>::new();

        let freed = unsafe { heap.collect(&[dynamic], &no_refs) };
        assert_eq!(freed, 1);
        assert_eq!(heap.live_count(), 2);
        assert!(heap.dynamic_get(dynamic, "child").is_some());
        let _ = unreachable;
    }

    #[test]
    fn array_storage_roundtrips_and_traces_object_elements() {
        let mut heap = Heap::new();
        let child = heap.alloc(7, 0);
        let array = heap.alloc_array(1);
        assert_eq!(heap.array_len(array), Some(1));
        assert_eq!(
            heap.array_set(array, 0, zun_core::Any::object(child.addr() as usize)),
            Some(zun_core::Any::int(0))
        );
        assert_eq!(
            heap.array_get(array, 0).and_then(zun_core::Any::as_object),
            Some(child.addr() as usize)
        );
        assert!(heap.array_insert(array, 1, zun_core::Any::int(9)));
        assert_eq!(
            heap.array_remove(array, 1).and_then(zun_core::Any::as_int),
            Some(9)
        );
        let unreachable = heap.alloc(8, 0);
        let no_refs = |_: u32| Vec::<usize>::new();
        let freed = unsafe { heap.collect(&[array], &no_refs) };
        assert_eq!(freed, 1);
        assert_eq!(heap.live_count(), 2);
        let _ = unreachable;
    }

    #[test]
    fn collect_frees_unreachable_and_keeps_reachable() {
        let mut heap = Heap::new();
        // a has one reference field (slot 0) pointing at b; c is unreferenced.
        let b = heap.alloc(2, 0);
        let a = heap.alloc(1, 1);
        a.set_field(0, b.addr() as u64);
        let _c = heap.alloc(3, 0);
        assert_eq!(heap.live_count(), 3);

        // Type 1 has a reference in slot 0; types 2 and 3 have none.
        let ref_slots = |type_rid: u32| if type_rid == 1 { vec![0usize] } else { vec![] };
        // Root set is just `a`; `a` keeps `b` alive; `c` is garbage.
        let freed = unsafe { heap.collect(&[a], &ref_slots) };
        assert_eq!(freed, 1);
        assert_eq!(heap.live_count(), 2);
    }

    #[test]
    fn collection_reuses_freed_blocks() {
        let mut heap = Heap::new();
        let keep = heap.alloc(1, 2);
        let garbage = heap.alloc(1, 2);
        let garbage_addr = garbage.addr();
        let garbage_hash = garbage.identity_hash();
        let no_refs = |_: u32| Vec::<usize>::new();

        let freed = unsafe { heap.collect(&[keep], &no_refs) };
        assert_eq!(freed, 1);
        // A new allocation of the same size reuses the reclaimed block.
        let reused = heap.alloc(9, 2);
        assert_eq!(reused.addr(), garbage_addr);
        assert_ne!(reused.identity_hash(), garbage_hash);
        assert_eq!(reused.type_rid(), 9);
        assert_eq!(reused.field(0), 0); // reused block was zeroed
    }

    #[test]
    fn transitively_reachable_objects_survive() {
        let mut heap = Heap::new();
        // Chain a -> b -> c, all type 1 with a single reference in slot 0.
        let c = heap.alloc(1, 1);
        let b = heap.alloc(1, 1);
        let a = heap.alloc(1, 1);
        b.set_field(0, c.addr() as u64);
        a.set_field(0, b.addr() as u64);
        let unreachable = heap.alloc(1, 1);
        let _ = unreachable;

        let ref_slots = |_: u32| vec![0usize];
        let freed = unsafe { heap.collect(&[a], &ref_slots) };
        assert_eq!(freed, 1); // only the detached object
        assert_eq!(heap.live_count(), 3);
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

    #[test]
    fn native_root_scope_contributes_to_root_set() {
        let mut heap = Heap::new();
        let object = heap.alloc(1, 0);
        let registered = [object];
        let _scope = unsafe { NativeRootScope::enter(&registered) };
        let mut roots = Vec::new();
        append_native_roots(&mut roots);
        assert_eq!(roots, registered);
    }

    #[test]
    fn collection_keeps_native_roots_alive() {
        let mut heap = Heap::new();
        let object = heap.alloc(1, 0);
        let registered = [object];
        let _scope = unsafe { NativeRootScope::enter(&registered) };
        let freed = unsafe { heap.collect_with_native_roots(&[], &|_| Vec::new()) };
        assert_eq!(freed, 0);
        assert_eq!(heap.live_count(), 1);
    }

    #[test]
    fn stack_map_lookup_matches_the_registered_span() {
        let mut registry = StackMapRegistry::new();
        registry.replace([
            NativeStackMap { pc: 200, span: 8 },
            NativeStackMap { pc: 100, span: 16 },
        ]);
        assert_eq!(registry.lookup(100).unwrap().pc, 100);
        assert_eq!(registry.lookup(115).unwrap().pc, 100);
        assert!(registry.lookup(116).is_none());
        assert!(registry.lookup(99).is_none());
        assert_eq!(registry.lookup(203).unwrap().pc, 200);
        assert!(registry.lookup(208).is_none());
    }
}
