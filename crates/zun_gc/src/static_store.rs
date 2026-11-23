//! Flat store for class-static data fields.
//!
//! A static field used to be one property of a dynamic object keyed by the
//! string `"{owner}:{field}"`, so every `ldsfld`/`stsfld` paid a hash of the
//! object address plus a hash of a freshly formatted key. A FieldDef RID is
//! unique within a module, so it indexes a dense table directly.
//!
//! The owner still matters: `Sub.count = v` on an *inherited* static stores
//! under `(Sub, Base.count's field)`, leaving the base's value alone. The
//! first owner to write a field claims its slot; any other owner for the same
//! field lives in `overflow`, keyed by the full pair. Behaviour is therefore
//! exactly as discriminating as the old string key (two modules whose RIDs
//! collide on one heap collide here exactly as they did there).
//!
//! The store is a heap-owned root: every collection kind marks every stored
//! object reference and rewrites it on relocation, so no write barrier is
//! needed (a root that is always scanned cannot hide a young object).
//!
//! Slots live in fixed-size chunks that are never moved or freed, so a slot's
//! address is stable for the life of the heap.

use crate::fast_hash::HashMap;
use zun_core::Any;

/// Field RID bits that select a slot within a chunk; native code indexes the
/// chunk table with the remaining high bits.
pub const STATIC_CHUNK_BITS: u32 = 10;
const CHUNK_BITS: u32 = STATIC_CHUNK_BITS;
const CHUNK_LEN: usize = 1 << CHUNK_BITS;
/// Field RIDs above this go to `overflow` instead of allocating chunks.
pub const STATIC_MAX_FLAT_FIELD: u32 = (1 << 20) - 1;
const MAX_FLAT_FIELD: u32 = STATIC_MAX_FLAT_FIELD;
/// Chunk-table length: enough for every flat field RID.
const CHUNK_COUNT: usize = (MAX_FLAT_FIELD >> CHUNK_BITS) as usize + 1;

/// Byte offsets of a [`StaticSlot`]'s parts, for native code.
pub const STATIC_SLOT_SIZE: usize = std::mem::size_of::<StaticSlot>();
pub const STATIC_SLOT_TAG_OFFSET: usize = std::mem::offset_of!(StaticSlot, value);
pub const STATIC_SLOT_PAYLOAD_OFFSET: usize = STATIC_SLOT_TAG_OFFSET + 8;
pub const STATIC_SLOT_OWNER_OFFSET: usize = std::mem::offset_of!(StaticSlot, owner);
pub const STATIC_SLOT_STATE_OFFSET: usize = std::mem::offset_of!(StaticSlot, state);


/// Slot has never been written by any owner.
pub const STATIC_SLOT_UNCLAIMED: u32 = 0;
/// An owner claimed the slot but it holds no value (removed).
pub const STATIC_SLOT_EMPTY: u32 = 1;
/// The slot holds a value.
pub const STATIC_SLOT_SET: u32 = 2;

/// One static field cell. `repr(C)` so native code can address the fields.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct StaticSlot {
    pub value: Any,
    pub owner: u32,
    pub state: u32,
}

impl StaticSlot {
    const fn new() -> Self {
        Self {
            value: Any::void(),
            owner: 0,
            state: STATIC_SLOT_UNCLAIMED,
        }
    }
}

type Chunk = [StaticSlot; CHUNK_LEN];

/// `table` is a fixed-length boxed array of chunk pointers (null = not yet
/// allocated). Both the table and every chunk are heap-allocated and never
/// moved, so native code can hold their addresses; see `heap_context`.
#[derive(Debug)]
pub(crate) struct StaticStore {
    table: Box<[*mut Chunk]>,
    overflow: HashMap<(u32, u32), Any>,
}

impl Default for StaticStore {
    fn default() -> Self {
        Self {
            table: vec![std::ptr::null_mut(); CHUNK_COUNT].into_boxed_slice(),
            overflow: HashMap::default(),
        }
    }
}

impl Drop for StaticStore {
    fn drop(&mut self) {
        for chunk in self.table.iter().copied().filter(|chunk| !chunk.is_null()) {
            // SAFETY: allocated by `Box::into_raw` in `slot_mut`, freed once.
            drop(unsafe { Box::from_raw(chunk) });
        }
    }
}

impl StaticStore {
    /// The chunk-pointer table native code indexes by `field >> CHUNK_BITS`.
    pub(crate) fn table_ptr(&self) -> *const *mut Chunk {
        self.table.as_ptr()
    }

    fn slot(&self, field: u32) -> Option<&StaticSlot> {
        let chunk = *self.table.get((field >> CHUNK_BITS) as usize)?;
        if chunk.is_null() {
            return None;
        }
        // SAFETY: non-null chunks are live boxes owned by this store.
        Some(unsafe { &(*chunk)[field as usize & (CHUNK_LEN - 1)] })
    }

    fn slot_mut(&mut self, field: u32) -> Option<&mut StaticSlot> {
        if field > MAX_FLAT_FIELD {
            return None;
        }
        let entry = &mut self.table[(field >> CHUNK_BITS) as usize];
        if entry.is_null() {
            *entry = Box::into_raw(
                Box::<Chunk>::try_from(vec![StaticSlot::new(); CHUNK_LEN].into_boxed_slice())
                    .expect("chunk length"),
            );
        }
        // SAFETY: the chunk is a live box owned by this store.
        Some(unsafe { &mut (**entry)[field as usize & (CHUNK_LEN - 1)] })
    }

    pub(crate) fn get(&self, owner: u32, field: u32) -> Option<Any> {
        if let Some(slot) = self.slot(field) {
            if slot.state != STATIC_SLOT_UNCLAIMED && slot.owner == owner {
                return (slot.state == STATIC_SLOT_SET).then_some(slot.value);
            }
        }
        if self.overflow.is_empty() {
            return None;
        }
        self.overflow.get(&(owner, field)).copied()
    }

    pub(crate) fn set(&mut self, owner: u32, field: u32, value: Any) {
        if let Some(slot) = self.slot_mut(field) {
            if slot.state == STATIC_SLOT_UNCLAIMED {
                slot.owner = owner;
            }
            if slot.owner == owner {
                slot.value = value;
                slot.state = STATIC_SLOT_SET;
                return;
            }
        }
        self.overflow.insert((owner, field), value);
    }

    pub(crate) fn remove(&mut self, owner: u32, field: u32) -> bool {
        if let Some(slot) = self.slot_mut(field) {
            if slot.state != STATIC_SLOT_UNCLAIMED && slot.owner == owner {
                let had = slot.state == STATIC_SLOT_SET;
                slot.state = STATIC_SLOT_EMPTY;
                slot.value = Any::void();
                return had;
            }
        }
        self.overflow.remove(&(owner, field)).is_some()
    }

    fn values(&self) -> impl Iterator<Item = &Any> {
        self.table
            .iter()
            .filter(|chunk| !chunk.is_null())
            // SAFETY: non-null chunks are live boxes owned by this store.
            .flat_map(|&chunk| unsafe { (*chunk).iter() })
            .filter(|slot| slot.state == STATIC_SLOT_SET)
            .map(|slot| &slot.value)
            .chain(self.overflow.values())
    }

    /// Addresses of every object the store references (GC roots).
    pub(crate) fn object_references(&self) -> impl Iterator<Item = usize> + '_ {
        self.values().filter_map(|value| value.as_object())
    }

    /// Rewrite every stored object reference through `map`; counts the changes.
    pub(crate) fn rewrite_object_references(&mut self, map: impl Fn(usize) -> usize) -> usize {
        let mut rewritten = 0;
        let mut rewrite = |value: &mut Any| {
            if let Some(old) = value.as_object() {
                let new = map(old);
                if new != old {
                    *value = value.map_object_address(|_| new);
                    rewritten += 1;
                }
            }
        };
        for chunk in self.table.iter().copied().filter(|chunk| !chunk.is_null()) {
            // SAFETY: live box owned by this store, exclusively borrowed via `&mut self`.
            for slot in unsafe { (*chunk).iter_mut() }.filter(|slot| slot.state == STATIC_SLOT_SET) {
                rewrite(&mut slot.value);
            }
        }
        for value in self.overflow.values_mut() {
            rewrite(value);
        }
        rewritten
    }
}
