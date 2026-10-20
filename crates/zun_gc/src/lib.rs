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

mod handles;
mod heap_context;
mod native_roots;
mod runtime_alloc;
mod stack_maps;
pub use heap_context::{with_current_heap, with_current_heap_mut, HeapScope};
pub use native_roots::{append_native_roots, NativeRootScope};
pub use runtime_alloc::{zun_gc_alloc, zun_rt_alloc};
pub use stack_maps::{NativeStackMap, StackMapRegistry};

mod allocation;
mod collection;
mod object;
mod records;
mod value_ops;
mod weak;
mod strings;
mod diagnostics;
mod dynamic;
mod arrays;
mod maps;
mod sets;
mod hashing;
mod typed_views;
mod iterators;
mod weak_state;

pub use handles::{FinalizationId, WeakMapId, WeakRefId};
use records::Record;
use value_ops::any_same_value_zero;
use weak_state::{Ephemeron, FinalizerEntry, WeakSlot};

/// Slots per allocation chunk (objects are bump-allocated within a chunk).
const CHUNK_SLOTS: usize = 64 * 1024;

pub use object::Obj;
/// The size of one object slot in bytes.
pub use zun_core::SLOT_SIZE;

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
    /// Typed-array descriptors associated with managed view objects.
    typed_views: HashMap<usize, (Obj, zun_core::TypedArrayView)>,
    /// Array iterator state associated with managed iterator objects.
    array_iterators: HashMap<usize, (Obj, zun_core::ArrayIterator)>,
    /// Set iterator state associated with managed iterator objects.
    set_iterators: HashMap<usize, (Obj, zun_core::SetIterator)>,
    /// Map iterator state associated with managed iterator objects.
    map_iterators: HashMap<usize, (Obj, zun_core::MapIterator)>,
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
    pub fn data_view_get_f64(
        &self,
        object: Obj,
        relative: usize,
        little_endian: bool,
    ) -> Option<f64> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.data_views.get(&(object.addr() as usize))?.get_f64(
            self.buffers.get(&(buffer.addr() as usize))?,
            relative,
            little_endian,
        )
    }

    pub fn data_view_set_f64(
        &mut self,
        object: Obj,
        relative: usize,
        value: f64,
        little_endian: bool,
    ) -> Option<bool> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        let view = *self.data_views.get(&(object.addr() as usize))?;
        Some(view.set_f64(
            self.buffers.get_mut(&(buffer.addr() as usize))?,
            relative,
            value,
            little_endian,
        ))
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
        assert_eq!(unsafe { heap.collect(&[key], &|_| Vec::new()) }, 2);
        assert_eq!(heap.weak_map_get(weak_map), Some((key, value)));
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
    fn weak_map_stores_multiple_entries_until_collection() {
        let mut heap = Heap::new();
        let weak_map = heap.weak_map_new();
        let key_a = heap.alloc(1, 0);
        let value_a = heap.alloc(2, 0);
        let key_b = heap.alloc(3, 0);
        let value_b = heap.alloc(4, 0);
        assert!(heap.weak_map_set(weak_map, key_a, value_a));
        assert!(heap.weak_map_set(weak_map, key_b, value_b));
        let entries = heap.weak_map_entries(weak_map);
        assert_eq!(entries.len(), 2);
        assert!(entries.contains(&(key_a, value_a)));
        assert!(entries.contains(&(key_b, value_b)));
    }

    #[test]
    fn weak_map_stress_cycles_with_collection_at_each_safepoint() {
        let mut heap = Heap::new();
        let weak_map = heap.weak_map_new();
        let mut live_keys = Vec::new();
        let mut entries = Vec::new();
        let ref_slots = |type_rid: u32| {
            if type_rid == 0 {
                Vec::new()
            } else {
                vec![0usize]
            }
        };

        for index in 0..32u32 {
            let key = heap.alloc(index + 1, 1);
            let value = heap.alloc(index + 100, 1);
            key.set_field(0, value.addr() as u64);
            value.set_field(0, key.addr() as u64);
            assert!(heap.weak_map_set(weak_map, key, value));
            if index % 2 == 0 {
                live_keys.push(key);
            }
            entries.push((key, value));
            unsafe { heap.collect(&live_keys, &ref_slots) };
        }

        let remaining = heap.weak_map_entries(weak_map);
        for (index, (key, value)) in entries.into_iter().enumerate() {
            if index % 2 == 0 {
                assert!(remaining.contains(&(key, value)));
            } else {
                assert!(!remaining.iter().any(|(candidate, _)| *candidate == key));
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
    fn symbol_properties_roundtrip_through_the_heap() {
        let mut heap = Heap::new();
        let object = heap.alloc_dynamic();
        let iterator = heap.symbol_for("Symbol.iterator");
        assert_eq!(
            heap.dynamic_set_symbol(object, iterator, zun_core::Any::int(7)),
            None
        );
        assert_eq!(
            heap.dynamic_get_symbol(object, iterator)
                .and_then(zun_core::Any::as_int),
            Some(7)
        );
    }

    #[test]
    fn callable_symbol_properties_keep_closure_environment_live() {
        let mut heap = Heap::new();
        let object = heap.alloc_dynamic();
        let iterator = heap.symbol_for("Symbol.iterator");
        let function = zun_core::FunctionHandle::new(17, Some(0x2000));
        assert_eq!(
            heap.dynamic_set_symbol_function(object, iterator, function),
            None
        );
        assert_eq!(
            heap.dynamic_get_symbol_function(object, iterator),
            Some(function)
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
