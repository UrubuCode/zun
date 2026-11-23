//! Reference rewriting after a young evacuation.
//!
//! A young evacuation moves only young survivors. A reference to a moved object
//! can therefore live only in a root, in the moved objects themselves, or in an
//! old object that was handed a young reference -- exactly the remembered set,
//! because the write barrier recorded every such store. Everything else is
//! untouched, so the pass costs what survived, not what is tenured.
//!
//! The one caveat is a non-moving collection since the nursery was last
//! released: it can leave old objects in nursery storage, and those move too
//! without any barrier having seen a reference to them. The caller falls back to
//! the complete relocation in that case (`Heap::nursery_pinned`).

use super::*;

impl Heap {
    /// Rewrite every heap-owned reference to an object in `relocation`, given
    /// that only the records in `moved` (record index, old address) moved and
    /// that every reference to them from an old object is remembered.
    pub(crate) fn rewrite_after_young_evacuation(
        &mut self,
        relocation: &RelocationMap,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        moved: &[(usize, usize)],
    ) {
        self.rewrite_global_roots(relocation);
        // The weak and finalizer lists are scanned whole: they hold addresses,
        // not objects, and are as long as the registrations are many.
        self.rewrite_weak_lists(relocation);
        let owners_remembered: Vec<usize> = self.remembered_set.iter().copied().collect();
        for &(idx, old) in moved {
            let new = self.records[idx].addr as usize;
            self.rewrite_owner(new, old, relocation, ref_slots);
        }
        for &owner in &owners_remembered {
            // An old object keeps its address: the remembered set holds the
            // current address of every owner.
            self.rewrite_owner(owner, owner, relocation, ref_slots);
        }
    }

    /// Rewrite everything the object that now lives at `new` (and lived at
    /// `old`) owns: its field slots and its entries in every side table, which
    /// are keyed by address and so are re-keyed when the object moved.
    fn rewrite_owner(
        &mut self,
        new: usize,
        old: usize,
        relocation: &RelocationMap,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) {
        let map = |address: usize| relocation.relocate_address(address);
        let object = unsafe { Obj::from_addr(new as *mut u64) };
        for &field in ref_slots(object.type_rid()).iter() {
            let slot = unsafe { object.addr().add(zun_core::OBJECT_HEADER_SLOTS + field) };
            unsafe { relocation.rewrite_slot(slot) };
        }

        // The table entry for this owner: re-keyed when it moved, then `body`
        // rewrites the references the value itself holds. A table that holds
        // nothing cannot hold this owner, which is what keeps an object that
        // owns no side-table state free of hashed probes.
        macro_rules! touch {
            ($table:ident, |$value:ident| $body:expr) => {
                if !self.$table.is_empty() {
                    if old == new {
                        if let Some($value) = self.$table.get_mut(&old) {
                            $body;
                        }
                    } else if let Some(mut owned) = self.$table.remove(&old) {
                        {
                            let $value = &mut owned;
                            $body;
                        }
                        self.$table.insert(new, owned);
                    }
                }
            };
        }
        touch!(arrays, |array| {
            array.rewrite_object_references(map);
        });
        touch!(any_fields, |fields| {
            for value in fields.values_mut() {
                if let Some(address) = value.as_object() {
                    let mapped = map(address);
                    if mapped != address {
                        *value = value.map_object_address(|_| mapped);
                    }
                }
            }
        });
        touch!(dynamic_objects, |properties| {
            properties.rewrite_object_references(map);
        });
        touch!(maps, |table| {
            table.rewrite_object_references(map);
        });
        touch!(sets, |table| {
            table.rewrite_object_references(map);
        });
        touch!(promises, |promise| {
            promise.rewrite_object_references(map);
        });
        touch!(proxies, |proxy| {
            proxy.target = unsafe { Obj::from_addr(map(proxy.target.addr() as usize) as *mut u64) };
            proxy.handler =
                unsafe { Obj::from_addr(map(proxy.handler.addr() as usize) as *mut u64) };
        });
        touch!(generator_frames, |frame| {
            generators::rewrite_frame_values(frame, relocation);
        });
        touch!(generator_states, |state| {
            generators::rewrite_state_values(state, relocation);
        });
        // The iterators and views carry their source twice: in the table and in
        // header slot 0 of the object itself.
        touch!(array_iterators, |entry| {
            entry.0 = unsafe { Obj::from_addr(map(entry.0.addr() as usize) as *mut u64) };
            object.set_field(0, entry.0.addr() as u64);
        });
        touch!(set_iterators, |entry| {
            entry.0 = unsafe { Obj::from_addr(map(entry.0.addr() as usize) as *mut u64) };
            object.set_field(0, entry.0.addr() as u64);
        });
        touch!(map_iterators, |entry| {
            entry.0 = unsafe { Obj::from_addr(map(entry.0.addr() as usize) as *mut u64) };
            object.set_field(0, entry.0.addr() as u64);
        });
        touch!(typed_views, |entry| {
            entry.0 = unsafe { Obj::from_addr(map(entry.0.addr() as usize) as *mut u64) };
            object.set_field(0, entry.0.addr() as u64);
        });
        if !self.data_views.is_empty() {
            if old != new {
                if let Some(view) = self.data_views.remove(&old) {
                    self.data_views.insert(new, view);
                }
            }
            if self.data_views.contains_key(&new) {
                let buffer = object.field(0) as usize;
                object.set_field(0, map(buffer) as u64);
            }
        }
        if old != new {
            // Tables keyed by the owner alone: nothing inside them to rewrite.
            touch!(buffers, |_value| {});
            touch!(dates, |_value| {});
            touch!(errors, |_value| {});
            touch!(regexes, |_value| {});
            touch!(bigints, |_value| {});
            touch!(weak_map_objects, |_value| {});
            touch!(weak_ref_objects, |_value| {});
            if !self.node_buffers.is_empty() && self.node_buffers.remove(&old) {
                self.node_buffers.insert(new);
            }
            if !self.weak_set_objects.is_empty() && self.weak_set_objects.remove(&old) {
                self.weak_set_objects.insert(new);
            }
        }
    }
}
