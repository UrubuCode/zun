//! Mark-sweep collection and root-expanding collection entry points.

use super::*;

/// Whether `address` falls inside one of the `[start, end)` byte ranges. The
/// collector uses it to tell nursery storage from tenured storage, which a
/// record's generation alone does not answer: a non-moving full collection
/// promotes survivors in place, leaving old objects in nursery chunks.
fn address_in(ranges: &[(usize, usize)], address: *mut u64) -> bool {
    let address = address as usize;
    // `ranges` is sorted by start (see `Heap::nursery_ranges`) and disjoint.
    let after = ranges.partition_point(|&(start, _)| start <= address);
    after > 0 && address < ranges[after - 1].1
}

impl Heap {
    /// Collect using interpreter roots plus every stopped native frame
    /// currently registered with the GC boundary.
    ///
    /// # Safety
    /// Every registered frame must remain stopped for the duration of the
    /// collection, and its `(PC, SP)` pair must identify a valid stack-map
    /// safepoint. The caller must uphold the same root contract as
    /// [`Heap::collect`].
    pub unsafe fn collect_with_registered_native_frames(
        &mut self,
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        with_active_native_frames(|active| {
            let mut all_roots = roots.to_vec();
            for frame in active {
                let registry = unsafe { &*frame.registry };
                all_roots.extend(unsafe { registry.roots_at(frame.pc, frame.sp) });
            }
            unsafe { self.collect(&all_roots, ref_slots) }
        })
    }

    pub unsafe fn collect(
        &mut self,
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        // SAFETY: callers uphold the root contract documented on the public
        // entry point; this is the complete-heap policy.
        let roots = roots as *const [Obj] as *mut [Obj];
        unsafe { self.collect_generation(roots, ref_slots, false, false, None) }
    }

    /// Collect only young objects while retaining every old allocation.
    ///
    /// The remembered set supplies old objects as additional traversal roots,
    /// so references written by the barrier keep their young targets alive.
    /// Weak references and finalizers are deliberately processed only by a
    /// full collection until their generation-specific policies are added.
    pub unsafe fn collect_young(
        &mut self,
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        // SAFETY: same root contract as `collect`.
        let roots = roots as *const [Obj] as *mut [Obj];
        unsafe { self.collect_generation(roots, ref_slots, true, false, None) }
    }

    /// Collect young objects and evacuate survivors into tenured chunks.
    pub unsafe fn collect_young_evacuating(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        on_relocation: &mut dyn FnMut(&RelocationMap),
    ) -> usize {
        let roots = roots as *mut [Obj];
        unsafe { self.collect_generation(roots, ref_slots, true, true, Some(on_relocation)) }
    }

    /// Evacuate young survivors while also rewriting references in stopped
    /// native frames described by their precise stack maps.
    pub unsafe fn collect_young_evacuating_with_native_frames(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        frames: &[StoppedNativeFrame<'_>],
        on_relocation: &mut dyn FnMut(&RelocationMap),
    ) -> usize {
        let root_count = roots.len();
        let mut all_roots = roots.to_vec();
        for frame in frames {
            all_roots.extend(unsafe { frame.registry.roots_at(frame.pc, frame.sp) });
        }
        let freed = unsafe {
            self.collect_young_evacuating(&mut all_roots, ref_slots, &mut |relocation| {
                for frame in frames {
                    frame
                        .registry
                        .rewrite_slots_at(frame.pc, frame.sp, relocation);
                }
                on_relocation(relocation);
            })
        };
        roots.copy_from_slice(&all_roots[..root_count]);
        freed
    }

    /// Collect only young objects using roots from an explicit stopped-frame
    /// list in addition to interpreter roots.
    ///
    /// # Safety
    /// Every frame must remain stopped and valid for the duration of the
    /// collection, with its `(PC, SP)` pair covered by its stack map.
    pub unsafe fn collect_young_with_native_frames(
        &mut self,
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        frames: &[StoppedNativeFrame<'_>],
    ) -> usize {
        let mut all_roots = roots.to_vec();
        for frame in frames {
            all_roots.extend(unsafe { frame.registry.roots_at(frame.pc, frame.sp) });
        }
        unsafe { self.collect_young(&all_roots, ref_slots) }
    }

    /// Collect only young objects using every stopped native frame currently
    /// registered at the GC boundary.
    ///
    /// # Safety
    /// Every registered frame must remain stopped for the duration of the
    /// collection and point to valid stack-map metadata and stack memory.
    pub unsafe fn collect_young_with_registered_native_frames(
        &mut self,
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        with_active_native_frames(|active| {
            let frames: Vec<_> = active
                .iter()
                .map(|frame| StoppedNativeFrame {
                    registry: unsafe { &*frame.registry },
                    pc: frame.pc,
                    sp: frame.sp,
                })
                .collect();
            unsafe { self.collect_young_with_native_frames(roots, ref_slots, &frames) }
        })
    }

    unsafe fn collect_generation(
        &mut self,
        roots: *mut [Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        young_only: bool,
        evacuate: bool,
        mut on_relocation: Option<&mut dyn FnMut(&RelocationMap)>,
    ) -> usize {
        debug_assert!(!evacuate || young_only, "only a young collection evacuates");
        self.allocations = 0;
        if young_only {
            if self.verify_barriers.0 {
                self.verify_remembered_set(ref_slots);
            }
        } else {
            // A full collection marks, and so un-marks, the whole record table.
            // A young collection marks only young records and clears exactly
            // those on its way out, so it never needs this pass.
            for r in &mut self.records {
                r.marked = false;
            }
        }

        let mut work: Vec<usize> = Vec::new();
        // Mark `address` if it names a live record this collection traces. A
        // young collection never enters the old generation: an old object is
        // live by definition, and the only way it can keep a young object alive
        // is through the remembered set, which is expanded below.
        let mark = |records: &mut Vec<Record>,
                    by_addr: &HashMap<usize, usize>,
                    work: &mut Vec<usize>,
                    address: usize| {
            if let Some(&idx) = by_addr.get(&address) {
                let record = &mut records[idx];
                if record.marked || (young_only && record.generation == records::Generation::Old) {
                    return;
                }
                record.marked = true;
                work.push(idx);
            }
        };

        // Read through the raw slice: the evacuating path writes the rewritten
        // roots back through it, so a shared reborrow held across that write
        // would let the caller assume its roots never change.
        for root in unsafe { &*roots } {
            mark(&mut self.records, &self.by_addr, &mut work, root.0 as usize);
        }
        // Heap-owned persistent roots: the realm global and the class-statics
        // object outlive any single frame, so they are always live (their stored
        // references are then traced through the work list like any other object).
        for persistent in [self.global_object, self.statics].into_iter().flatten() {
            mark(&mut self.records, &self.by_addr, &mut work, persistent.0 as usize);
        }
        // Class-static data slots are roots too: always scanned, which is why a
        // young object stored there needs no remembered-set entry.
        for address in self.static_store.object_references() {
            mark(&mut self.records, &self.by_addr, &mut work, address);
        }
        let mut children: Vec<usize> = Vec::new();
        if young_only {
            // An old object that was handed a young reference contributes only
            // that edge: its own record is neither marked nor traced further.
            for &owner in &self.remembered_set {
                if let Some(&idx) = self.by_addr.get(&owner) {
                    if self.records[idx].alive {
                        children.clear();
                        self.children_into(Obj(self.records[idx].addr), ref_slots, &mut children);
                        for &child in &children {
                            mark(&mut self.records, &self.by_addr, &mut work, child);
                        }
                    }
                }
            }
        }
        // Finalizer callbacks are strong runtime-owned callables. Their
        // target is weak, but a captured callback environment must survive
        // until the registration is removed or delivered.
        for entry in &self.finalizers {
            // A pending registration's held value is delivered to the cleanup
            // callback, so an object it references must survive until then.
            if !entry.queued {
                if let Some(address) = entry.held.as_object() {
                    mark(&mut self.records, &self.by_addr, &mut work, address);
                }
            }
            if let Some(environment) = entry.callback.and_then(|callback| callback.environment()) {
                mark(&mut self.records, &self.by_addr, &mut work, environment);
            }
        }
        // Already-queued-but-not-yet-delivered records keep their held value
        // (and callback environment) alive until the runtime drains them.
        for record in &self.finalization_queue {
            if let Some(address) = record.held.as_object() {
                mark(&mut self.records, &self.by_addr, &mut work, address);
            }
            if let Some(environment) = record.callback.and_then(|callback| callback.environment()) {
                mark(&mut self.records, &self.by_addr, &mut work, environment);
            }
        }
        // A registry's cleanup callback is a strong runtime-owned callable: its
        // captured environment must survive as long as the registry is reachable,
        // even before the first `register` copies it into a registration.
        for cleanup in self.finalization_registries.values() {
            if let Some(environment) = cleanup.environment() {
                mark(&mut self.records, &self.by_addr, &mut work, environment);
            }
        }
        // Ephemerons: a marked key promotes its values. The key -> values index
        // is built once, so each entry is visited a constant number of times. A
        // young collection never marks an old key, but an old key is live, so
        // its values are promoted up front.
        let mut ephemeron_values: HashMap<usize, Vec<usize>> = HashMap::default();
        if !self.weak_maps.is_empty() {
            for entry in &self.weak_maps {
                for &(key, value) in &entry.entries {
                    ephemeron_values.entry(key).or_default().push(value);
                }
            }
            if young_only {
                for (&key, values) in &ephemeron_values {
                    let old_key = self.by_addr.get(&key).is_some_and(|&idx| {
                        self.records[idx].generation == records::Generation::Old
                    });
                    if old_key {
                        for &value in values {
                            mark(&mut self.records, &self.by_addr, &mut work, value);
                        }
                    }
                }
            }
        }
        while let Some(idx) = work.pop() {
            let obj = Obj(self.records[idx].addr);
            children.clear();
            self.children_into(obj, ref_slots, &mut children);
            if !ephemeron_values.is_empty() {
                if let Some(values) = ephemeron_values.get(&(obj.addr() as usize)) {
                    children.extend(values.iter().copied());
                }
            }
            for &child in &children {
                mark(&mut self.records, &self.by_addr, &mut work, child);
            }
        }

        // Weak references observe the completed mark set but do not contribute
        // roots. Clear dead targets before sweep removes their addresses.
        for entry in &mut self.weak_maps {
            entry.entries.retain(|(key, value)| {
                let keep = |address: &usize| {
                    self.by_addr.get(address).is_some_and(|idx| {
                        let record = &self.records[*idx];
                        record.marked
                            || (young_only && record.generation == records::Generation::Old)
                    })
                };
                keep(key) && keep(value)
            });
        }
        for weak in &mut self.weak_refs {
            if let Some(address) = weak.target {
                if self.by_addr.get(&address).is_none_or(|&idx| {
                    !self.records[idx].marked
                        && (!young_only
                            || self.records[idx].generation == records::Generation::Young)
                }) {
                    weak.target = None;
                }
            }
        }

        // The nursery's address ranges, read once: the evacuate path consults
        // them both before moving survivors out and after sweeping, and the
        // chunk list is only released at the very end.
        let nursery_ranges = if evacuate {
            self.nursery_ranges()
        } else {
            Vec::new()
        };
        // A non-moving collection since the last nursery release may have left
        // old objects, or recycled slots, in nursery storage. Then nothing
        // short of the whole record table says which references are about to
        // change, and the evacuation falls back to the complete relocation.
        let wide = evacuate && self.nursery_pinned;
        let nursery_list = std::mem::take(&mut self.nursery_records);
        let mut moved: Vec<(usize, usize)> = Vec::new();
        let mut survived_bytes = 0usize;
        if evacuate {
            // This collection releases the nursery, so every object that is
            // still alive afterwards has to leave it. Marked young objects are
            // promoted, as before. An object that an earlier non-moving full
            // collection promoted *in place* is already old but its storage can
            // still be nursery storage, and a young-only collection never
            // proves an old object dead, so those move too and keep their
            // generation. The nursery record list names exactly these.
            let survivors: Vec<usize> = nursery_list
                .iter()
                .copied()
                .filter(|&idx| {
                    let record = &self.records[idx];
                    record.alive
                        && ((record.generation == records::Generation::Young && record.marked)
                            || (record.generation == records::Generation::Old
                                && address_in(&nursery_ranges, record.addr)))
                })
                .collect();
            let mut relocation = RelocationMap::with_capacity(survivors.len());
            moved.reserve(survivors.len());
            for idx in survivors {
                let field_count = self.records[idx].field_count;
                let old = self.records[idx].addr;
                let slots = zun_core::OBJECT_HEADER_SLOTS + field_count;
                let bytes = slots * std::mem::size_of::<u64>();
                self.promoted_since_compaction += bytes;
                survived_bytes += bytes;
                let new = self.alloc_old_slots(slots);
                unsafe { std::ptr::copy_nonoverlapping(old, new, slots) };
                relocation.insert_addresses(old as usize, new as usize);
                let record = &mut self.records[idx];
                record.addr = new;
                record.generation = records::Generation::Old;
                record.marked = false;
                self.by_addr.remove(&(old as usize));
                self.by_addr.insert(new as usize, idx);
                moved.push((idx, old as usize));
            }

            for root in unsafe { &mut *roots } {
                let address = root.addr() as usize;
                let mapped = relocation.relocate_address(address);
                if mapped != address {
                    *root = unsafe { Obj::from_addr(mapped as *mut u64) };
                }
            }
            // Nothing moved (every young object was garbage): there is no
            // reference to rewrite, and walking every table to find that out
            // was the dominant cost of a young collection.
            if !relocation.is_empty() {
                if wide {
                    self.rewrite_all_references_impl(&relocation, ref_slots, true);
                } else {
                    self.rewrite_after_young_evacuation(&relocation, ref_slots, &moved);
                }
                unsafe { rewrite_native_roots(&relocation) };
            }
            if let Some(callback) = on_relocation.as_mut() {
                callback(&relocation);
            }
        }

        // Sweep: reclaim live-but-unmarked objects into the free list.
        let mut freed = 0;
        // Dead records whose storage is nursery storage that an evacuating
        // collection is about to hand out again. They skip the address-keyed free
        // list (they would only be scrubbed from it below) and are released
        // straight to the record free list.
        let mut released_dead: Vec<usize> = Vec::new();
        if young_only {
            // Only the nursery's records can be young: the list holds every one.
            let mut kept = Vec::new();
            for &idx in &nursery_list {
                let r = &self.records[idx];
                if !r.alive || r.generation != records::Generation::Young {
                    if r.alive && !evacuate {
                        kept.push(idx);
                    }
                    continue;
                }
                if r.marked {
                    let r = &mut self.records[idx];
                    r.marked = false;
                    r.generation = records::Generation::Old;
                    kept.push(idx);
                } else {
                    freed += 1;
                    self.sweep_dead(idx, evacuate, &nursery_ranges, &mut released_dead);
                }
            }
            if evacuate {
                // Every record left the nursery or died: keep the buffer.
                let mut list = nursery_list;
                list.clear();
                self.nursery_records = list;
            } else {
                self.nursery_records = kept;
            }
        } else {
            for idx in 0..self.records.len() {
                let r = &mut self.records[idx];
                if r.alive && !r.marked {
                    freed += 1;
                    self.sweep_dead(idx, false, &nursery_ranges, &mut released_dead);
                } else if r.alive {
                    // A full collection is also the promotion boundary: from
                    // here a young collection skips these records and relies on
                    // the remembered set for old-to-young writes.
                    r.marked = false;
                    if r.generation == records::Generation::Young {
                        r.generation = records::Generation::Old;
                    }
                }
            }
            let records = &self.records;
            self.nursery_records = nursery_list
                .iter()
                .copied()
                .filter(|&idx| records[idx].alive)
                .collect();
        }
        if !evacuate {
            // Survivors now sit in nursery storage as old objects.
            self.nursery_pinned = true;
        }
        if evacuate {
            // Dead records still point into the nursery storage this
            // collection is about to hand out again, so they cannot go back on
            // the address-preserving free list: that would make the next
            // allocation a use-after-free. Release their storage instead, which
            // keeps the record index reusable at a fresh address.
            let mut stale: Vec<usize> = released_dead;
            if self.nursery_pinned {
                // A non-moving collection may have put nursery-resident slots on
                // the free list; they are scrubbed here, once, by the collection
                // that releases the nursery.
                let mut free = std::mem::take(&mut self.free);
                for free_indices in free.values_mut() {
                    free_indices.retain(|&idx| {
                        if address_in(&nursery_ranges, self.records[idx].addr) {
                            stale.push(idx);
                            false
                        } else {
                            true
                        }
                    });
                }
                self.free = free;
            }
            for idx in stale {
                self.release_record_storage(idx);
            }
            // Nothing alive is left in nursery storage: young survivors were
            // promoted out of it and the sweep has just dropped the rest.
            let allocated = self.nursery_bytes;
            self.reset_nursery();
            self.note_young_collection(allocated, survived_bytes);
        }
        for entry in &mut self.finalizers {
            if entry.target != 0 && !entry.queued && !self.by_addr.contains_key(&entry.target) {
                entry.queued = true;
                self.finalization_queue.push(super::FinalizationRecord {
                    held: entry.held,
                    callback_id: entry.callback_id,
                    callback: entry.callback,
                });
            }
        }
        // Every survivor of any collection is old afterwards, so no old-to-young
        // edge can exist: the set starts over empty.
        self.remembered_set.clear();
        if young_only {
            self.nursery_bytes = 0;
        }
        freed
    }

    /// Drop a dead record: unindex it, forget its side-table state, and queue it
    /// for reuse (at its own address, or at a fresh one when its storage is
    /// nursery storage that is about to be handed out again).
    fn sweep_dead(
        &mut self,
        idx: usize,
        evacuate: bool,
        nursery_ranges: &[(usize, usize)],
        released_dead: &mut Vec<usize>,
    ) {
        let r = &mut self.records[idx];
        r.alive = false;
        r.marked = false;
        let addr = r.addr;
        let address = addr as usize;
        let field_count = r.field_count;
        self.by_addr.remove(&address);
        // A plain object is in none of the side tables below, and a table
        // that holds nothing cannot hold this address: the emptiness check is
        // what keeps a dead object from costing a hashed probe per table.
        macro_rules! forget {
            ($($table:ident),*) => { $( if !self.$table.is_empty() { self.$table.remove(&address); } )* };
        }
        forget!(
            dynamic_objects, arrays, any_fields, buffers, data_views, typed_views,
            array_iterators, generator_frames, generator_states, set_iterators,
            map_iterators, dates, errors, maps, sets, weak_map_objects, weak_set_objects,
            node_buffers, weak_ref_objects, promises, regexes, bigints, proxies
        );
        if evacuate && address_in(nursery_ranges, addr) {
            released_dead.push(idx);
        } else {
            self.free.entry(field_count).or_default().push(idx);
        }
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
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
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
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        let mut all_roots = roots.to_vec();
        append_native_roots(&mut all_roots);
        // SAFETY: the caller guarantees the explicit roots and registered
        // native roots are valid for this heap.
        unsafe { self.collect(&all_roots, ref_slots) }
    }

    /// Young-only counterpart to [`Heap::collect_with_native_roots`].
    pub unsafe fn collect_young_with_native_roots(
        &mut self,
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        let mut all_roots = roots.to_vec();
        append_native_roots(&mut all_roots);
        unsafe { self.collect_young(&all_roots, ref_slots) }
    }

    /// Collect using roots described by a stopped JIT frame's stack map.
    ///
    /// # Safety
    /// `pc` and `sp` must identify the same stopped native frame, and the stack
    /// map must describe valid managed-reference slots for that frame.
    pub unsafe fn collect_with_native_frame(
        &mut self,
        roots: &[Obj],
        registry: &StackMapRegistry,
        pc: usize,
        sp: *const u8,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        let mut all_roots = roots.to_vec();
        all_roots.extend(unsafe { registry.roots_at(pc, sp) });
        unsafe { self.collect(&all_roots, ref_slots) }
    }

    /// Young-only counterpart to [`Heap::collect_with_native_frame`].
    pub unsafe fn collect_young_with_native_frame(
        &mut self,
        roots: &[Obj],
        registry: &StackMapRegistry,
        pc: usize,
        sp: *const u8,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        let mut all_roots = roots.to_vec();
        all_roots.extend(unsafe { registry.roots_at(pc, sp) });
        unsafe { self.collect_young(&all_roots, ref_slots) }
    }

    /// Collect using the innermost stack-map registry active on this thread.
    ///
    /// # Safety
    /// `pc` and `sp` must identify the same stopped native frame, and an
    /// [`ActiveStackMapScope`](super::ActiveStackMapScope) for that frame's
    /// registry must remain active for the duration of this call.
    pub unsafe fn collect_with_active_native_frame(
        &mut self,
        roots: &[Obj],
        pc: usize,
        sp: *const u8,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        super::with_active_stack_map_registry(|registry| {
            let Some(registry) = registry else {
                return unsafe { self.collect_with_native_roots(roots, ref_slots) };
            };
            let mut all_roots = roots.to_vec();
            append_native_roots(&mut all_roots);
            unsafe { self.collect_with_native_frame(&all_roots, registry, pc, sp, ref_slots) }
        })
    }

    /// Collect young objects from the stopped JIT frame at the current
    /// allocation safepoint.
    ///
    /// # Safety
    /// `pc` and `sp` must identify the stopped frame covered by the active
    /// stack-map registry for this thread.
    pub unsafe fn collect_young_with_active_native_frame(
        &mut self,
        roots: &[Obj],
        pc: usize,
        sp: *const u8,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        super::with_active_stack_map_registry(|registry| {
            let Some(registry) = registry else {
                return unsafe { self.collect_young_with_native_roots(roots, ref_slots) };
            };
            let mut all_roots = roots.to_vec();
            append_native_roots(&mut all_roots);
            unsafe { self.collect_young_with_native_frame(&all_roots, registry, pc, sp, ref_slots) }
        })
    }

    /// Evacuate young objects from the stopped JIT frame at the current
    /// allocation safepoint, rewriting both stack-map slots and registered
    /// native roots before execution resumes.
    ///
    /// # Safety
    /// `pc` and `sp` must identify the stopped frame covered by the active
    /// stack-map registry for this thread.
    pub unsafe fn collect_young_evacuating_with_active_native_frame(
        &mut self,
        roots: &mut [Obj],
        pc: usize,
        sp: *mut u8,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
    ) -> usize {
        super::with_active_stack_map_registry(|registry| {
            let Some(registry) = registry else {
                let mut all_roots = roots.to_vec();
                append_native_roots(&mut all_roots);
                return unsafe {
                    self.collect_young_evacuating(&mut all_roots, ref_slots, &mut |_| {})
                };
            };
            let frame = StoppedNativeFrame { registry, pc, sp };
            let root_count = roots.len();
            let mut all_roots = roots.to_vec();
            append_native_roots(&mut all_roots);
            all_roots.extend(unsafe { registry.roots_at(pc, sp) });
            let freed = unsafe {
                self.collect_young_evacuating_with_native_frames(
                    &mut all_roots,
                    ref_slots,
                    &[frame],
                    &mut |_| {},
                )
            };
            roots.copy_from_slice(&all_roots[..root_count]);
            freed
        })
    }
}
