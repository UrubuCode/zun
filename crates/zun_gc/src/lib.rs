//! The Zun runtime heap.
//!
//! An object allocator shared, by design, between the interpreter and
//! the JIT (PLAN.md §4.3, step 2). An object is a contiguous run of 8-byte
//! slots: slot 0 holds the type RID, slot 1 holds the identity hash, and slots
//! `2..=field_count + 1` hold the fields.
//! Field slots are **raw** — a number's bits, an integer, a boolean, a nested
//! object pointer, or a string-table handle — interpreted by the field's static
//! type. This is what lets native code load and store fields by offset.
//!
//! Objects are normally stable, but the registered JIT safepoint path may move
//! them during a full collection; native roots are rewritten before release.
//! handed to native code remains stable. Mark-sweep reclaims unreachable
//! objects using the type RID to find reference slots; interpreter roots,
//! native root scopes, and stopped JIT frames described by stack maps are
//! supported. Moving collection rewrites heap-owned references and registered
//! native stack-map locations before releasing old storage.
//!
//! # Safety
//! Object pointers are valid only while the owning [`Heap`] is alive. The
//! interpreter and JIT must not outlive it. Raw pointers remain valid until a
//! collection boundary; callers crossing a moving collection must use the
//! registered root and stack-map contracts so references can be rewritten.

// The crate-level module docs (see `# Safety` above) and the per-contract
// comments on the unsafe root/stack-map APIs document the shared invariants;
// individual `# Safety` sections would only restate them.
#![allow(clippy::missing_safety_doc)]

use std::collections::{HashMap, HashSet};

mod handles;
mod heap_context;
mod native_roots;
mod runtime_alloc;
mod stack_maps;
pub use heap_context::{current_heap_ptr, with_current_heap, with_current_heap_mut, HeapScope};
pub use native_roots::{append_native_roots, rewrite_native_roots, NativeRootScope};
pub use runtime_alloc::{zun_gc_alloc, zun_rt_alloc};
pub use stack_maps::{
    with_active_native_frames, with_active_stack_map_registry, ActiveNativeFrame,
    ActiveNativeFrameScope, ActiveStackMapScope, NativeStackMap, StackMapRegistry,
};

mod allocation;
mod arrays;
mod bigints;
mod buffers;
mod collection;
mod dates;
mod diagnostics;
mod dynamic;
mod errors;
mod generators;
mod hashing;
mod iterators;
mod maps;
mod object;
mod proxies;
mod records;
mod regexes;
mod relocation;
mod sets;
mod strings;
mod typed_views;
mod value_ops;
mod weak;
mod weak_state;

pub use handles::{FinalizationId, WeakMapId, WeakRefId};
use records::Record;
use value_ops::any_same_value_zero;
pub use weak_state::FinalizationRecord;
use weak_state::{Ephemeron, FinalizerEntry, WeakSlot};

/// Slots per allocation chunk (objects are bump-allocated within a chunk).
const CHUNK_SLOTS: usize = 64 * 1024;

pub use object::Obj;
pub use relocation::RelocationMap;
/// The size of one object slot in bytes.
pub use zun_core::SLOT_SIZE;

/// Read-only result of the non-moving heap's compaction planning phase.
///
/// Planning is intentionally separate from relocation: the current heap
/// exposes raw object addresses to the interpreter and JIT, so this phase may
/// measure reclaimable space but must not move objects yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionPlan {
    pub live_objects: usize,
    pub reclaimable_objects: usize,
    pub live_slots: usize,
    pub reclaimable_slots: usize,
}

/// Result of an explicit stop-the-world heap compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionResult {
    pub moved_objects: usize,
    pub rewritten_references: usize,
}

/// Outcome of the opt-in mark/sweep plus compaction path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectionCompactionResult {
    pub reclaimed_objects: usize,
    pub moved_objects: usize,
    pub rewritten_references: usize,
}

/// A stopped native frame whose managed slots are described by a stack map.
#[derive(Debug, Clone, Copy)]
pub struct StoppedNativeFrame<'a> {
    pub registry: &'a StackMapRegistry,
    pub pc: usize,
    pub sp: *mut u8,
}

/// Owned suspended generator state stored by the heap. The interpreter
/// converts its `Value`s to `Any` slots at suspension points.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratorFrame {
    pub method_rid: u32,
    pub pc: usize,
    pub locals: Vec<zun_core::Any>,
    pub eval_stack: Vec<zun_core::Any>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeneratorState {
    pub method_rid: u32,
    pub args: Vec<zun_core::Any>,
    pub frame: Option<GeneratorFrame>,
    pub done: bool,
    pub current: zun_core::Any,
    pub has_current: bool,
    /// Value a `return(v)` completion owes the caller once a yielding
    /// `finally` finishes and the generator completes.
    pub pending_return: zun_core::Any,
    pub has_pending_return: bool,
}

/// The runtime heap: bump-allocated chunks, object records for mark-sweep, a
/// per-size free list, and a string handle table.
#[derive(Debug, Default)]
pub struct Heap {
    /// Boxed chunks; their contents have stable addresses for the heap's life.
    chunks: Vec<Box<[u64]>>,
    /// Bump cursor within the last chunk.
    cursor: usize,
    /// Ephemeral chunks owned exclusively by the current nursery epoch.
    nursery_chunks: Vec<Box<[u64]>>,
    /// Bump cursor within the last nursery chunk.
    nursery_cursor: usize,
    /// Bytes allocated in the current young-generation nursery epoch.
    nursery_bytes: usize,
    /// Allocation budget that requests a minor collection at a safepoint.
    nursery_budget: usize,
    /// Interned strings, referenced from field slots by index.
    strings: Vec<zun_core::ZStr>,
    /// Symbols belong to this heap/isolate and never share identity globally.
    symbols: zun_core::SymbolRegistry,
    /// Weak references are cleared after marking and never keep targets alive.
    weak_refs: Vec<WeakSlot>,
    /// WeakMap entries promoted only when their keys are marked.
    weak_maps: Vec<Ephemeron>,
    /// Finalizers are queued after sweep and never run during collection.
    finalizers: Vec<FinalizerEntry>,
    finalization_queue: Vec<FinalizationRecord>,
    /// Cleanup callback per `FinalizationRegistry` object, keyed by the
    /// registry object's address identity. Seeded when the registry is
    /// constructed and copied into each registration at `register` time.
    finalization_registries: HashMap<usize, zun_core::FunctionHandle>,
    /// One record per object ever allocated (dead ones are reused).
    records: Vec<Record>,
    /// Reference-field layouts published by the active module/JIT boundary.
    reference_slots: HashMap<u32, Vec<usize>>,
    /// Allocations since the last native safepoint collection.
    native_allocations: usize,
    /// Young native safepoint collections since the last native compaction.
    native_young_collections: usize,
    /// Dynamic property bags associated with dynamic objects.
    dynamic_objects: HashMap<usize, zun_core::DynamicObject>,
    /// Contiguous array storage associated with array objects.
    arrays: HashMap<usize, zun_core::ArrayStorage>,
    /// Side storage for frozen-layout `any`-typed fields (PLAN §2: the dynamic
    /// `Any` carrier is the explicit, costly exception). A field slot of kind
    /// `FIELD_ANY` is 8 bytes but an `Any` is 16, so the value lives here keyed
    /// by `(owner address, field slot)` — the exact analogue of `arrays` — and
    /// is traced, relocated, swept, and write-barriered like array elements.
    any_fields: HashMap<usize, HashMap<usize, zun_core::Any>>,
    /// Contiguous byte storage associated with ArrayBuffer objects.
    buffers: HashMap<usize, zun_core::ByteBuffer>,
    /// DataView descriptors associated with managed view objects.
    data_views: HashMap<usize, zun_core::DataView>,
    /// Typed-array descriptors associated with managed view objects.
    typed_views: HashMap<usize, (Obj, zun_core::TypedArrayView)>,
    /// Array iterator state associated with managed iterator objects.
    array_iterators: HashMap<usize, (Obj, zun_core::ArrayIterator)>,
    /// Suspended generator state associated with managed generator objects.
    generator_frames: HashMap<usize, GeneratorFrame>,
    /// State-machine metadata associated with managed generator objects.
    generator_states: HashMap<usize, GeneratorState>,
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
    /// WeakRef slots owned by managed WeakRef objects.
    weak_ref_objects: HashMap<usize, WeakRefId>,
    /// Promise state associated with managed Promise objects.
    promises: HashMap<usize, zun_core::ManagedPromise>,
    /// Compiled regular expressions associated with managed RegExp objects.
    regexes: HashMap<usize, zun_core::RegexValue>,
    /// Arbitrary-precision integer payloads associated with BigInt objects.
    bigints: HashMap<usize, zun_core::BigInt>,
    /// Strong target/handler pairs owned by managed Proxy objects.
    proxies: HashMap<usize, proxies::ProxyRecord>,
    /// Record index by object address, for tracing child references.
    by_addr: HashMap<usize, usize>,
    /// Dead record indices available for reuse, keyed by field count.
    free: HashMap<usize, Vec<usize>>,
    /// Monotonic identity hashes. Zero is reserved as the uninitialized value.
    next_identity_hash: u64,
    /// Old objects that have received a reference to a young object.
    ///
    /// This is the remembered-set boundary for the future minor collector;
    /// full collections still trace the complete heap today.
    remembered_set: HashSet<usize>,
    /// Per-heap realm singleton used by `globalThis`/`global` bindings.
    global_object: Option<Obj>,
    /// Per-heap store for class-static field values, keyed by
    /// `(owner type RID, field RID)`. A single dynamic object whose properties
    /// are the static slots, so both the interpreter and JIT reach the same
    /// values through one GC-rooted object (see [`Heap::statics_object`]).
    statics: Option<Obj>,
}

impl Heap {
    /// Return the per-heap realm singleton, allocating it lazily.
    pub fn global_object(&mut self) -> Obj {
        if let Some(object) = self.global_object {
            return object;
        }
        let object = self.alloc_dynamic();
        self.global_object = Some(object);
        object
    }

    /// Return the per-heap class-statics object, allocating it lazily. It is a
    /// dynamic object whose string keys are static-field slots; both tiers read
    /// and write it so static state is shared between interpreter and JIT.
    pub fn statics_object(&mut self) -> Obj {
        if let Some(object) = self.statics {
            return object;
        }
        let object = self.alloc_dynamic();
        self.statics = Some(object);
        object
    }

    /// The slot key for a class-static field. Centralized here so the
    /// interpreter and the JIT helpers derive the identical key.
    fn static_slot_key(owner_type_rid: u32, field_rid: u32) -> String {
        format!("{owner_type_rid}:{field_rid}")
    }

    /// Remove a class-static field slot (value or function), answering whether
    /// one was present.
    pub fn static_remove(&mut self, owner_type_rid: u32, field_rid: u32) -> bool {
        let Some(object) = self.statics else {
            return false;
        };
        let key = Self::static_slot_key(owner_type_rid, field_rid);
        let had_function = self.dynamic_get_function(object, &key).is_some();
        let removed = self.dynamic_remove(object, &key).is_some();
        removed || had_function
    }

    /// Read a class-static field value (or `None` if unset).
    pub fn static_get(&self, owner_type_rid: u32, field_rid: u32) -> Option<zun_core::Any> {
        let object = self.statics?;
        self.dynamic_get(object, &Self::static_slot_key(owner_type_rid, field_rid))
    }

    /// Write a class-static field value, allocating the statics object on first use.
    pub fn static_set(&mut self, owner_type_rid: u32, field_rid: u32, value: zun_core::Any) {
        let object = self.statics_object();
        self.dynamic_set(
            object,
            Self::static_slot_key(owner_type_rid, field_rid),
            value,
        );
    }

    /// Read a class-static field holding a function value (closures do not box
    /// into `Any`, so they ride the dynamic function slot, like object methods).
    pub fn static_get_function(
        &self,
        owner_type_rid: u32,
        field_rid: u32,
    ) -> Option<zun_core::FunctionHandle> {
        let object = self.statics?;
        self.dynamic_get_function(object, &Self::static_slot_key(owner_type_rid, field_rid))
    }

    /// Write a class-static field holding a function value.
    pub fn static_set_function(
        &mut self,
        owner_type_rid: u32,
        field_rid: u32,
        function: zun_core::FunctionHandle,
    ) {
        let object = self.statics_object();
        self.dynamic_set_function(
            object,
            Self::static_slot_key(owner_type_rid, field_rid),
            function,
        );
    }
    /// Publish the reference slots for a module type before entering JIT code.
    pub fn register_reference_slots(&mut self, type_rid: u32, slots: Vec<usize>) {
        self.reference_slots.insert(type_rid, slots);
    }

    pub(crate) fn native_allocation_due(&mut self) -> bool {
        self.native_allocations = self.native_allocations.saturating_add(1);
        if self.native_allocations >= 256 {
            self.native_allocations = 0;
            true
        } else {
            false
        }
    }

    /// Whether the current nursery epoch has exhausted its allocation budget.
    pub fn nursery_collection_due(&self) -> bool {
        self.nursery_bytes >= self.nursery_budget()
    }

    /// Return the current nursery allocation budget in bytes.
    pub fn nursery_budget(&self) -> usize {
        self.nursery_budget.max(32 * 1024)
    }

    pub(crate) fn native_compaction_due(&mut self) -> bool {
        self.native_young_collections = self.native_young_collections.saturating_add(1);
        if self.native_young_collections >= 8 {
            self.native_young_collections = 0;
            true
        } else {
            false
        }
    }

    /// Calculate the amount of live and reclaimable object storage.
    ///
    /// This is the first plan-phase primitive for future compaction. It has no
    /// side effects and deliberately does not relocate or rewrite references.
    pub fn compaction_plan(&self) -> CompactionPlan {
        self.records.iter().fold(
            CompactionPlan {
                live_objects: 0,
                reclaimable_objects: 0,
                live_slots: 0,
                reclaimable_slots: 0,
            },
            |mut plan, record| {
                let slots = zun_core::OBJECT_HEADER_SLOTS + record.field_count;
                if record.alive {
                    plan.live_objects += 1;
                    plan.live_slots += slots;
                } else {
                    plan.reclaimable_objects += 1;
                    plan.reclaimable_slots += slots;
                }
                plan
            },
        )
    }

    /// Compact live objects into fresh chunks and rewrite all heap-owned
    /// references before the old chunks are released.
    ///
    /// # Safety
    /// The caller must stop all managed execution and rewrite any native/JIT
    /// roots separately using the forwarding map before resuming execution.
    /// `roots` must contain every interpreter root, and `ref_slots` must
    /// describe every managed reference field in each object type.
    pub unsafe fn compact(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> CompactionResult {
        unsafe { self.compact_impl(roots, ref_slots, &[], None) }
    }

    /// Compact while also rewriting the reference slots of one stopped JIT
    /// frame selected by its program counter and stack pointer.
    pub unsafe fn compact_with_native_frame(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        registry: &StackMapRegistry,
        pc: usize,
        sp: *mut u8,
    ) -> CompactionResult {
        let frames = [StoppedNativeFrame { registry, pc, sp }];
        unsafe { self.compact_impl(roots, ref_slots, &frames, None) }
    }

    /// Compact while rewriting all stopped native frames supplied by the
    /// runtime's stop-the-world coordinator.
    pub unsafe fn compact_with_native_frames(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        frames: &[StoppedNativeFrame<'_>],
    ) -> CompactionResult {
        unsafe { self.compact_impl(roots, ref_slots, frames, None) }
    }

    /// Compact using every native frame currently registered by the runtime's
    /// stop-the-world coordinator.
    ///
    /// # Safety
    /// The caller must stop managed execution and ensure that every registered
    /// frame remains stopped until this method returns. Each registration must
    /// point to a live [`StackMapRegistry`] and stack memory described by it.
    pub unsafe fn compact_with_registered_native_frames(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> CompactionResult {
        with_active_native_frames(|active| {
            let frames: Vec<_> = active
                .iter()
                .map(|frame| StoppedNativeFrame {
                    registry: unsafe { &*frame.registry },
                    pc: frame.pc,
                    sp: frame.sp,
                })
                .collect();
            unsafe { self.compact_with_native_frames(roots, ref_slots, &frames) }
        })
    }

    /// Compact and invoke `on_relocation` while the forwarding map is valid.
    ///
    /// The callback runs after heap and stopped-native references have been
    /// rewritten, but before the old chunks are released. This is the safe
    /// bridge for owners of managed values that are not stored in the heap.
    ///
    /// # Safety
    /// The caller must stop all managed execution and ensure that the callback
    /// rewrites every external managed reference before it returns.
    pub unsafe fn compact_with_relocation(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        on_relocation: &mut dyn FnMut(&RelocationMap),
    ) -> CompactionResult {
        unsafe { self.compact_impl(roots, ref_slots, &[], Some(on_relocation)) }
    }

    unsafe fn compact_impl(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        native_frames: &[StoppedNativeFrame<'_>],
        mut on_relocation: Option<&mut dyn FnMut(&RelocationMap)>,
    ) -> CompactionResult {
        fn allocate(chunks: &mut Vec<Box<[u64]>>, cursor: &mut usize, slots: usize) -> *mut u64 {
            if chunks.is_empty() || *cursor + slots > CHUNK_SLOTS {
                chunks.push(vec![0u64; CHUNK_SLOTS].into_boxed_slice());
                *cursor = 0;
            }
            let chunk = chunks.last_mut().expect("just pushed a chunk");
            let base = unsafe { chunk.as_mut_ptr().add(*cursor) };
            *cursor += slots;
            base
        }

        let mut new_chunks = Vec::new();
        let mut new_cursor = 0;
        let mut relocation = RelocationMap::new();
        let mut moved_objects = 0;
        for record in self.records.iter_mut() {
            if !record.alive {
                continue;
            }
            let slots = zun_core::OBJECT_HEADER_SLOTS + record.field_count;
            let old = record.addr;
            let new = allocate(&mut new_chunks, &mut new_cursor, slots);
            unsafe {
                std::ptr::copy_nonoverlapping(old, new, slots);
            }
            relocation.insert_addresses(old as usize, new as usize);
            record.addr = new;
            moved_objects += 1;
        }

        for root in roots {
            let address = root.addr() as usize;
            let mapped = relocation.relocate_address(address);
            if mapped != address {
                *root = unsafe { Obj::from_addr(mapped as *mut u64) };
            }
        }
        let mut rewritten_references = self.rewrite_all_references(&relocation, ref_slots);
        rewritten_references += unsafe { rewrite_native_roots(&relocation) };
        for frame in native_frames {
            rewritten_references += unsafe {
                frame
                    .registry
                    .rewrite_slots_at(frame.pc, frame.sp, &relocation)
            };
        }
        if let Some(callback) = on_relocation.as_mut() {
            callback(&relocation);
        }
        for record in self.records.iter_mut().filter(|record| !record.alive) {
            record.addr = std::ptr::null_mut();
        }
        self.free.clear();
        self.chunks = new_chunks;
        self.cursor = new_cursor;
        CompactionResult {
            moved_objects,
            rewritten_references,
        }
    }

    /// Run a full collection and immediately compact its survivors.
    ///
    /// This is opt-in until the runtime has a stop-the-world coordinator for
    /// every native/JIT frame. Native roots must be rewritten separately while
    /// stopped; `roots` contains the interpreter roots owned by this call.
    #[allow(clippy::ptr_arg)] // `roots` is the fixed VM<->GC boundary buffer
    pub unsafe fn collect_and_compact(
        &mut self,
        roots: &mut Vec<Obj>,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> CollectionCompactionResult {
        let mark_roots = roots.clone();
        let reclaimed_objects = unsafe { self.collect(&mark_roots, ref_slots) };
        let compacted = unsafe { self.compact(roots, ref_slots) };
        CollectionCompactionResult {
            reclaimed_objects,
            moved_objects: compacted.moved_objects,
            rewritten_references: compacted.rewritten_references,
        }
    }

    /// Run mark/sweep and compact while exposing the forwarding map to an
    /// owner of managed values stored outside the heap.
    ///
    /// # Safety
    /// The caller must stop managed execution and rewrite every external
    /// managed reference from the callback before it returns.
    #[allow(clippy::ptr_arg)] // `roots` is the fixed VM<->GC boundary buffer
    pub unsafe fn collect_and_compact_with_relocation(
        &mut self,
        roots: &mut Vec<Obj>,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        on_relocation: &mut dyn FnMut(&RelocationMap),
    ) -> CollectionCompactionResult {
        let mark_roots = roots.clone();
        let reclaimed_objects = unsafe { self.collect(&mark_roots, ref_slots) };
        let compacted = unsafe { self.compact_with_relocation(roots, ref_slots, on_relocation) };
        CollectionCompactionResult {
            reclaimed_objects,
            moved_objects: compacted.moved_objects,
            rewritten_references: compacted.rewritten_references,
        }
    }

    /// Run mark/sweep and compaction using interpreter roots plus every
    /// stopped native frame currently registered at the GC boundary.
    ///
    /// # Safety
    /// The caller must stop managed execution and keep every registered frame,
    /// stack-map registry, and stack pointer valid until this method returns.
    pub unsafe fn collect_and_compact_with_registered_native_frames(
        &mut self,
        roots: &mut Vec<Obj>,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> CollectionCompactionResult {
        with_active_native_frames(|active| {
            let frames: Vec<_> = active
                .iter()
                .map(|frame| StoppedNativeFrame {
                    registry: unsafe { &*frame.registry },
                    pc: frame.pc,
                    sp: frame.sp,
                })
                .collect();
            unsafe { self.collect_and_compact_with_native_frames(roots, ref_slots, &frames) }
        })
    }

    /// Run mark/sweep and compaction using an explicit stop-the-world frame
    /// list supplied by the runtime coordinator.
    ///
    /// # Safety
    /// Every frame must remain stopped and valid, with a live registry and a
    /// stack pointer covered by its stack map, until this method returns.
    #[allow(clippy::ptr_arg)] // `roots` is the fixed VM<->GC boundary buffer
    pub unsafe fn collect_and_compact_with_native_frames(
        &mut self,
        roots: &mut Vec<Obj>,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        frames: &[StoppedNativeFrame<'_>],
    ) -> CollectionCompactionResult {
        let mut mark_roots = roots.clone();
        append_native_roots(&mut mark_roots);
        for frame in frames {
            mark_roots.extend(unsafe { frame.registry.roots_at(frame.pc, frame.sp) });
        }
        let reclaimed_objects = unsafe { self.collect(&mark_roots, ref_slots) };
        let compacted = unsafe { self.compact_with_native_frames(roots, ref_slots, frames) };
        CollectionCompactionResult {
            reclaimed_objects,
            moved_objects: compacted.moved_objects,
            rewritten_references: compacted.rewritten_references,
        }
    }

    /// Rewrite managed references stored in object fields using a forwarding
    /// map. This is the object-heap portion of a future relocate phase;
    /// interpreter roots, side tables, and native stack-map slots are handled
    /// by their respective owners before compaction is enabled.
    pub fn rewrite_object_references(
        &mut self,
        relocation: &RelocationMap,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        let mut rewritten = 0;
        for record in self.records.iter().filter(|record| record.alive) {
            let object = Obj(record.addr);
            for &field in ref_slots(object.type_rid()).iter() {
                let slot = unsafe { object.addr().add(zun_core::OBJECT_HEADER_SLOTS + field) };
                if unsafe { relocation.rewrite_slot(slot) } {
                    rewritten += 1;
                }
            }
        }
        rewritten
    }

    /// Rewrite strong object references held by collection side tables. Hashes
    /// remain valid because object identity hashes are stored independently of
    /// addresses; weak tables and iterator/handle state need separate passes.
    pub fn rewrite_auxiliary_references(&mut self, relocation: &RelocationMap) -> usize {
        let map = |address| relocation.relocate_address(address);
        let mut rewritten = 0;
        let arrays = std::mem::take(&mut self.arrays);
        let mut mapped_arrays = HashMap::with_capacity(arrays.len());
        for (address, mut array) in arrays {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            rewritten += array.rewrite_object_references(map);
            mapped_arrays.insert(mapped, array);
        }
        self.arrays = mapped_arrays;

        let any_fields = std::mem::take(&mut self.any_fields);
        let mut mapped_any_fields = HashMap::with_capacity(any_fields.len());
        for (address, mut fields) in any_fields {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            for value in fields.values_mut() {
                if let Some(old) = value.as_object() {
                    let new = map(old);
                    if new != old {
                        *value = value.map_object_address(|_| new);
                        rewritten += 1;
                    }
                }
            }
            mapped_any_fields.insert(mapped, fields);
        }
        self.any_fields = mapped_any_fields;

        let objects = std::mem::take(&mut self.dynamic_objects);
        let mut mapped_objects = HashMap::with_capacity(objects.len());
        for (address, mut object) in objects {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            rewritten += object.rewrite_object_references(map);
            mapped_objects.insert(mapped, object);
        }
        self.dynamic_objects = mapped_objects;

        let maps = std::mem::take(&mut self.maps);
        let mut mapped_maps = HashMap::with_capacity(maps.len());
        for (address, mut map_storage) in maps {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            rewritten += map_storage.rewrite_object_references(map);
            mapped_maps.insert(mapped, map_storage);
        }
        self.maps = mapped_maps;

        let sets = std::mem::take(&mut self.sets);
        let mut mapped_sets = HashMap::with_capacity(sets.len());
        for (address, mut set_storage) in sets {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            rewritten += set_storage.rewrite_object_references(map);
            mapped_sets.insert(mapped, set_storage);
        }
        self.sets = mapped_sets;
        rewritten
    }

    /// Rewrite strong references in iterator and typed-view side tables,
    /// including their address-keyed indexes and object header fields.
    pub fn rewrite_iterator_references(&mut self, relocation: &RelocationMap) -> usize {
        fn relocated(relocation: &RelocationMap, object: Obj) -> Obj {
            unsafe {
                Obj::from_addr(relocation.relocate_address(object.addr() as usize) as *mut u64)
            }
        }

        let mut rewritten = 0;
        let mut arrays = std::mem::take(&mut self.array_iterators);
        let mut new_arrays = HashMap::with_capacity(arrays.len());
        for (address, (array, state)) in arrays.drain() {
            let iterator = unsafe { Obj::from_addr(address as *mut u64) };
            let new_iterator = relocated(relocation, iterator);
            let new_array = relocated(relocation, array);
            if new_iterator.addr() != iterator.addr() {
                rewritten += 1;
            }
            if new_array.addr() != array.addr() {
                rewritten += 1;
            }
            new_iterator.set_field(0, new_array.addr() as u64);
            new_arrays.insert(new_iterator.addr() as usize, (new_array, state));
        }
        self.array_iterators = new_arrays;

        let mut sets = std::mem::take(&mut self.set_iterators);
        let mut new_sets = HashMap::with_capacity(sets.len());
        for (address, (set, state)) in sets.drain() {
            let iterator = unsafe { Obj::from_addr(address as *mut u64) };
            let new_iterator = relocated(relocation, iterator);
            let new_set = relocated(relocation, set);
            if new_iterator.addr() != iterator.addr() {
                rewritten += 1;
            }
            if new_set.addr() != set.addr() {
                rewritten += 1;
            }
            new_iterator.set_field(0, new_set.addr() as u64);
            new_sets.insert(new_iterator.addr() as usize, (new_set, state));
        }
        self.set_iterators = new_sets;

        let mut maps = std::mem::take(&mut self.map_iterators);
        let mut new_maps = HashMap::with_capacity(maps.len());
        for (address, (map, state)) in maps.drain() {
            let iterator = unsafe { Obj::from_addr(address as *mut u64) };
            let new_iterator = relocated(relocation, iterator);
            let new_map = relocated(relocation, map);
            if new_iterator.addr() != iterator.addr() {
                rewritten += 1;
            }
            if new_map.addr() != map.addr() {
                rewritten += 1;
            }
            new_iterator.set_field(0, new_map.addr() as u64);
            new_maps.insert(new_iterator.addr() as usize, (new_map, state));
        }
        self.map_iterators = new_maps;

        let mut views = std::mem::take(&mut self.typed_views);
        let mut new_views = HashMap::with_capacity(views.len());
        for (address, (buffer, view)) in views.drain() {
            let object = unsafe { Obj::from_addr(address as *mut u64) };
            let new_object = relocated(relocation, object);
            let new_buffer = relocated(relocation, buffer);
            if new_object.addr() != object.addr() {
                rewritten += 1;
            }
            if new_buffer.addr() != buffer.addr() {
                rewritten += 1;
            }
            new_object.set_field(0, new_buffer.addr() as u64);
            new_views.insert(new_object.addr() as usize, (new_buffer, view));
        }
        self.typed_views = new_views;
        rewritten
    }

    /// Relocate weak metadata without changing its weak/ephemeron liveness
    /// rules. Targets are translated only so the next collection can still
    /// observe the same registrations after object movement.
    pub fn rewrite_weak_references(&mut self, relocation: &RelocationMap) -> usize {
        let map = |address: usize| relocation.relocate_address(address);
        let mut rewritten = 0;
        for weak in &mut self.weak_refs {
            if let Some(target) = weak.target {
                let mapped = map(target);
                if mapped != target {
                    rewritten += 1;
                    weak.target = Some(mapped);
                }
            }
        }
        for ephemeron in &mut self.weak_maps {
            for (key, value) in &mut ephemeron.entries {
                let mapped_key = map(*key);
                let mapped_value = map(*value);
                if mapped_key != *key {
                    rewritten += 1;
                    *key = mapped_key;
                }
                if mapped_value != *value {
                    rewritten += 1;
                    *value = mapped_value;
                }
            }
        }
        let owners = std::mem::take(&mut self.weak_map_objects);
        let mut mapped_owners = HashMap::with_capacity(owners.len());
        for (owner, id) in owners {
            let mapped = map(owner);
            if mapped != owner {
                rewritten += 1;
            }
            mapped_owners.insert(mapped, id);
        }
        self.weak_map_objects = mapped_owners;
        let ref_owners = std::mem::take(&mut self.weak_ref_objects);
        let mut mapped_ref_owners = HashMap::with_capacity(ref_owners.len());
        for (owner, id) in ref_owners {
            let mapped = map(owner);
            if mapped != owner {
                rewritten += 1;
            }
            mapped_ref_owners.insert(mapped, id);
        }
        self.weak_ref_objects = mapped_ref_owners;
        for finalizer in &mut self.finalizers {
            let mapped = map(finalizer.target);
            if mapped != finalizer.target {
                rewritten += 1;
                finalizer.target = mapped;
            }
            if let Some(address) = finalizer.held.as_object() {
                let mapped_held = map(address);
                if mapped_held != address {
                    rewritten += 1;
                    finalizer.held = zun_core::Any::object(mapped_held);
                }
            }
            if finalizer.unregister_token != 0 {
                let mapped_token = map(finalizer.unregister_token);
                if mapped_token != finalizer.unregister_token {
                    rewritten += 1;
                    finalizer.unregister_token = mapped_token;
                }
            }
            if let Some(callback) = finalizer.callback {
                if let Some(environment) = callback.environment() {
                    let mapped_environment = map(environment);
                    if mapped_environment != environment {
                        rewritten += 1;
                        finalizer.callback = Some(zun_core::FunctionHandle::new(
                            callback.method_rid(),
                            Some(mapped_environment),
                        ));
                    }
                }
            }
        }
        for record in &mut self.finalization_queue {
            if let Some(address) = record.held.as_object() {
                let mapped_held = map(address);
                if mapped_held != address {
                    rewritten += 1;
                    record.held = zun_core::Any::object(mapped_held);
                }
            }
            if let Some(callback) = record.callback {
                if let Some(environment) = callback.environment() {
                    let mapped_environment = map(environment);
                    if mapped_environment != environment {
                        rewritten += 1;
                        record.callback = Some(zun_core::FunctionHandle::new(
                            callback.method_rid(),
                            Some(mapped_environment),
                        ));
                    }
                }
            }
        }
        let registries = std::mem::take(&mut self.finalization_registries);
        let mut mapped_registries = HashMap::with_capacity(registries.len());
        for (owner, mut cleanup) in registries {
            let mapped_owner = map(owner);
            if mapped_owner != owner {
                rewritten += 1;
            }
            if let Some(environment) = cleanup.environment() {
                let mapped_environment = map(environment);
                if mapped_environment != environment {
                    rewritten += 1;
                    cleanup = zun_core::FunctionHandle::new(
                        cleanup.method_rid(),
                        Some(mapped_environment),
                    );
                }
            }
            mapped_registries.insert(mapped_owner, cleanup);
        }
        self.finalization_registries = mapped_registries;
        rewritten
    }

    /// Rewrite remaining strong side-table state held by promises and
    /// proxies, including their address-keyed indexes.
    pub fn rewrite_runtime_references(&mut self, relocation: &RelocationMap) -> usize {
        let map = |address| relocation.relocate_address(address);
        let mut rewritten = 0;

        let buffers = std::mem::take(&mut self.buffers);
        let mut mapped_buffers = HashMap::with_capacity(buffers.len());
        for (address, buffer) in buffers {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            mapped_buffers.insert(mapped, buffer);
        }
        self.buffers = mapped_buffers;

        let data_views = std::mem::take(&mut self.data_views);
        let mut mapped_data_views = HashMap::with_capacity(data_views.len());
        for (address, view) in data_views {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            let object = unsafe { Obj::from_addr(mapped as *mut u64) };
            let old_buffer = object.field(0) as usize;
            let new_buffer = map(old_buffer);
            if new_buffer != old_buffer {
                object.set_field(0, new_buffer as u64);
                rewritten += 1;
            }
            mapped_data_views.insert(mapped, view);
        }
        self.data_views = mapped_data_views;

        let dates = std::mem::take(&mut self.dates);
        let mut mapped_dates = HashMap::with_capacity(dates.len());
        for (address, value) in dates {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            mapped_dates.insert(mapped, value);
        }
        self.dates = mapped_dates;

        let errors = std::mem::take(&mut self.errors);
        let mut mapped_errors = HashMap::with_capacity(errors.len());
        for (address, value) in errors {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            mapped_errors.insert(mapped, value);
        }
        self.errors = mapped_errors;

        let regexes = std::mem::take(&mut self.regexes);
        let mut mapped_regexes = HashMap::with_capacity(regexes.len());
        for (address, value) in regexes {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            mapped_regexes.insert(mapped, value);
        }
        self.regexes = mapped_regexes;

        let bigints = std::mem::take(&mut self.bigints);
        let mut mapped_bigints = HashMap::with_capacity(bigints.len());
        for (address, value) in bigints {
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
            }
            mapped_bigints.insert(mapped, value);
        }
        self.bigints = mapped_bigints;

        let records = &mut self.records;
        for record in records.iter_mut().filter(|record| record.alive) {
            let address = record.addr as usize;
            let mapped = map(address);
            if mapped != address {
                rewritten += 1;
                record.addr = mapped as *mut u64;
            }
        }
        let by_addr = std::mem::take(&mut self.by_addr);
        let mut mapped_by_addr = HashMap::with_capacity(by_addr.len());
        for (address, index) in by_addr {
            mapped_by_addr.insert(map(address), index);
        }
        self.by_addr = mapped_by_addr;
        self.remembered_set = self
            .remembered_set
            .iter()
            .map(|&address| map(address))
            .collect();

        let promises = std::mem::take(&mut self.promises);
        let mut mapped_promises = HashMap::with_capacity(promises.len());
        for (address, mut promise) in promises {
            let mapped_address = map(address);
            if mapped_address != address {
                rewritten += 1;
            }
            rewritten += promise.rewrite_object_references(map);
            mapped_promises.insert(mapped_address, promise);
        }
        self.promises = mapped_promises;

        let proxies = std::mem::take(&mut self.proxies);
        let mut mapped_proxies = HashMap::with_capacity(proxies.len());
        for (address, proxy) in proxies {
            let mapped_address = map(address);
            let target = unsafe { Obj::from_addr(map(proxy.target.addr() as usize) as *mut u64) };
            let handler = unsafe { Obj::from_addr(map(proxy.handler.addr() as usize) as *mut u64) };
            if mapped_address != address {
                rewritten += 1;
            }
            if target.addr() != proxy.target.addr() {
                rewritten += 1;
            }
            if handler.addr() != proxy.handler.addr() {
                rewritten += 1;
            }
            mapped_proxies.insert(mapped_address, proxies::ProxyRecord { target, handler });
        }
        self.proxies = mapped_proxies;
        rewritten
    }

    /// Run every heap-owned relocation pass in dependency order.
    ///
    /// Native roots and stopped JIT frames are intentionally not implicit:
    /// callers must invoke [`rewrite_native_roots`] and
    /// [`StackMapRegistry::rewrite_slots_at`] while those frames are stopped.
    pub fn rewrite_all_references(
        &mut self,
        relocation: &RelocationMap,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        let mut rewritten = 0;
        if let Some(object) = self.global_object {
            let mapped = relocation.relocate(object);
            if mapped != object {
                self.global_object = Some(mapped);
                rewritten += 1;
            }
        }
        if let Some(object) = self.statics {
            let mapped = relocation.relocate(object);
            if mapped != object {
                self.statics = Some(mapped);
                rewritten += 1;
            }
        }
        rewritten
            + self.rewrite_object_references(relocation, ref_slots)
            + self.rewrite_auxiliary_references(relocation)
            + self.rewrite_generator_frames(relocation)
            + self.rewrite_generator_states(relocation)
            + self.rewrite_iterator_references(relocation)
            + self.rewrite_weak_references(relocation)
            + self.rewrite_runtime_references(relocation)
    }

    /// Return the number of live objects in each generation.
    pub fn generation_counts(&self) -> (usize, usize) {
        self.records
            .iter()
            .filter(|record| record.alive)
            .fold((0, 0), |(young, old), record| match record.generation {
                records::Generation::Young => (young + 1, old),
                records::Generation::Old => (young, old + 1),
            })
    }

    /// Store a raw field and record an old-to-young reference when applicable.
    /// The caller has already resolved the field's metadata; this helper only
    /// owns the GC write-barrier bookkeeping.
    pub fn store_field(&mut self, owner: Obj, index: usize, value: u64, is_reference: bool) {
        owner.set_field(index, value);
        if is_reference {
            self.write_barrier(owner, value);
        }
    }

    /// Store an `Any` into a frozen-layout `any`-typed field slot. The value
    /// lives in the `any_fields` side table (an `Any` does not fit the 8-byte
    /// slot); the slot itself is left as a sentinel. An object-tagged payload is
    /// write-barriered like any other old-to-young reference.
    pub fn store_any_field(&mut self, owner: Obj, slot: usize, value: zun_core::Any) {
        self.any_fields
            .entry(owner.addr() as usize)
            .or_default()
            .insert(slot, value);
        if let Some(child) = value.as_object() {
            self.write_barrier(owner, child as u64);
        }
    }

    /// Read an `Any` from a frozen-layout `any`-typed field slot. An absent
    /// entry means the field was never assigned (JS `undefined`).
    pub fn load_any_field(&self, owner: Obj, slot: usize) -> Option<zun_core::Any> {
        self.any_fields
            .get(&(owner.addr() as usize))
            .and_then(|fields| fields.get(&slot).copied())
    }

    /// Apply the generational write barrier for a raw object reference.
    pub fn write_barrier(&mut self, owner: Obj, value: u64) {
        let Some(&owner_index) = self.by_addr.get(&(owner.addr() as usize)) else {
            return;
        };
        let Some(&child_index) = self.by_addr.get(&(value as usize)) else {
            return;
        };
        if self.records[owner_index].alive
            && self.records[child_index].alive
            && self.records[owner_index].generation == records::Generation::Old
            && self.records[child_index].generation == records::Generation::Young
        {
            self.remembered_set.insert(owner.addr() as usize);
        }
    }

    /// Number of old objects currently recorded in the remembered set.
    pub fn remembered_set_len(&self) -> usize {
        self.remembered_set.len()
    }

    pub fn data_view_get_f64(
        &self,
        object: Obj,
        relative: usize,
        little_endian: bool,
    ) -> Option<f64> {
        let buffer = unsafe { Obj::from_addr(object.field(0) as *mut u64) };
        self.resolved_data_view(object)?.get_f64(
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
        let view = self.resolved_data_view(object)?;
        Some(view.set_f64(
            self.buffers.get_mut(&(buffer.addr() as usize))?,
            relative,
            value,
            little_endian,
        ))
    }
}

#[cfg(test)]
mod tests;
