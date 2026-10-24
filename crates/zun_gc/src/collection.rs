//! Mark-sweep collection and root-expanding collection entry points.

use super::*;

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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
    ) -> usize {
        // SAFETY: callers uphold the root contract documented on the public
        // entry point; this is the complete-heap policy.
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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
    ) -> usize {
        // SAFETY: same root contract as `collect`.
        unsafe { self.collect_generation(roots, ref_slots, true, false, None) }
    }

    /// Collect young objects and evacuate survivors into tenured chunks.
    pub unsafe fn collect_young_evacuating(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
        on_relocation: &mut dyn FnMut(&RelocationMap),
    ) -> usize {
        unsafe { self.collect_generation(roots, ref_slots, true, true, Some(on_relocation)) }
    }

    /// Evacuate young survivors while also rewriting references in stopped
    /// native frames described by their precise stack maps.
    pub unsafe fn collect_young_evacuating_with_native_frames(
        &mut self,
        roots: &mut [Obj],
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
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
                    frame.registry.rewrite_slots_at(frame.pc, frame.sp, relocation);
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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
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
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
        young_only: bool,
        evacuate: bool,
        mut on_relocation: Option<&mut dyn FnMut(&RelocationMap)>,
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
        if young_only {
            for address in self.remembered_set.clone() {
                if let Some(&idx) = self.by_addr.get(&address) {
                    if !self.records[idx].marked {
                        self.records[idx].marked = true;
                        work.push(idx);
                    }
                }
            }
        }
        // Finalizer callbacks are strong runtime-owned callables. Their
        // target is weak, but a captured callback environment must survive
        // until the registration is removed or delivered.
        for entry in &self.finalizers {
            let Some(environment) = entry.callback.and_then(|callback| callback.environment())
            else {
                continue;
            };
            let address = environment;
            if let Some(&idx) = self.by_addr.get(&address) {
                if !self.records[idx].marked {
                    self.records[idx].marked = true;
                    work.push(idx);
                }
            }
        }
        // A registry's cleanup callback is a strong runtime-owned callable: its
        // captured environment must survive as long as the registry is reachable,
        // even before the first `register` copies it into a registration.
        for cleanup in self.finalization_registries.values() {
            let Some(environment) = cleanup.environment() else {
                continue;
            };
            if let Some(&idx) = self.by_addr.get(&environment) {
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
            let generator_children: Vec<_> = self.trace_generator_frame(obj).collect();
            for child in generator_children {
                if let Some(&cidx) = self.by_addr.get(&child) {
                    if !self.records[cidx].marked {
                        self.records[cidx].marked = true;
                        work.push(cidx);
                    }
                }
            }
            let generator_state_children: Vec<_> = self.trace_generator_state(obj).collect();
            for child in generator_state_children {
                if let Some(&cidx) = self.by_addr.get(&child) {
                    if !self.records[cidx].marked {
                        self.records[cidx].marked = true;
                        work.push(cidx);
                    }
                }
            }
            if let Some(proxy) = self.proxies.get(&(obj.addr() as usize)) {
                for child in [proxy.target, proxy.handler] {
                    if let Some(&cidx) = self.by_addr.get(&(child.addr() as usize)) {
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
            if let Some(promise) = self.promises.get(&(obj.addr() as usize)) {
                if let Some(child) = promise.result().and_then(|value| value.as_object()) {
                    if let Some(&cidx) = self.by_addr.get(&child) {
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
                for &(key, value) in &entry.entries {
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
                if let Some(promise) = self.promises.get(&(obj.addr() as usize)) {
                    if let Some(child) = promise.result().and_then(|value| value.as_object()) {
                        if let Some(&child_idx) = self.by_addr.get(&child) {
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
        for entry in &mut self.weak_maps {
            entry.entries.retain(|(key, value)| {
                let keep = |address: &usize| {
                    self.by_addr.get(address).is_some_and(|idx| {
                        let record = &self.records[*idx];
                        record.marked || (young_only && record.generation == records::Generation::Old)
                    })
                };
                keep(key) && keep(value)
            });
        }
        for weak in &mut self.weak_refs {
            if let Some(address) = weak.target {
                if self.by_addr.get(&address).is_none_or(|&idx| {
                    !self.records[idx].marked
                        && (!young_only || self.records[idx].generation == records::Generation::Young)
                }) {
                    weak.target = None;
                }
            }
        }

        if evacuate {
            let mut relocation = RelocationMap::new();
            let survivors: Vec<_> = self
                .records
                .iter()
                .enumerate()
                .filter(|(_, record)| {
                    record.alive
                        && record.marked
                        && record.generation == records::Generation::Young
                })
                .map(|(idx, record)| (idx, record.addr, record.field_count))
                .collect();
            for (idx, old, field_count) in survivors {
                let slots = zun_core::OBJECT_HEADER_SLOTS + field_count;
                let new = self.alloc_old_slots(slots);
                unsafe { std::ptr::copy_nonoverlapping(old, new, slots) };
                relocation.insert_addresses(old as usize, new as usize);
                self.records[idx].addr = new;
                self.records[idx].generation = records::Generation::Old;
                self.by_addr.remove(&(old as usize));
                self.by_addr.insert(new as usize, idx);
            }

            let roots = unsafe {
                std::slice::from_raw_parts_mut(roots.as_ptr() as *mut Obj, roots.len())
            };
            for root in roots {
                let address = root.addr() as usize;
                let mapped = relocation.relocate_address(address);
                if mapped != address {
                    *root = unsafe { Obj::from_addr(mapped as *mut u64) };
                }
            }
            self.rewrite_all_references(&relocation, ref_slots);
            unsafe { rewrite_native_roots(&relocation) };
            if let Some(callback) = on_relocation.as_mut() {
                callback(&relocation);
            }
            // A non-moving full collection may have promoted survivors in
            // place. Those records are now old, but their storage can still
            // live in a nursery chunk. Retaining the chunks keeps those
            // promoted objects valid; force the next allocation into a fresh
            // chunk instead of reusing storage that contains old objects.
            self.nursery_cursor = CHUNK_SLOTS;
        }

        // Sweep: reclaim live-but-unmarked objects into the free list.
        let mut freed = 0;
        for idx in 0..self.records.len() {
            let r = &mut self.records[idx];
            let collect_this = !young_only || r.generation == records::Generation::Young;
            if r.alive && collect_this && !r.marked {
                r.alive = false;
                self.by_addr.remove(&(r.addr as usize));
                self.dynamic_objects.remove(&(r.addr as usize));
                self.arrays.remove(&(r.addr as usize));
                self.buffers.remove(&(r.addr as usize));
                self.data_views.remove(&(r.addr as usize));
                self.typed_views.remove(&(r.addr as usize));
                self.array_iterators.remove(&(r.addr as usize));
                self.generator_frames.remove(&(r.addr as usize));
                self.generator_states.remove(&(r.addr as usize));
                self.set_iterators.remove(&(r.addr as usize));
                self.map_iterators.remove(&(r.addr as usize));
                self.dates.remove(&(r.addr as usize));
                self.errors.remove(&(r.addr as usize));
                self.maps.remove(&(r.addr as usize));
                self.sets.remove(&(r.addr as usize));
                self.weak_map_objects.remove(&(r.addr as usize));
                self.weak_ref_objects.remove(&(r.addr as usize));
                self.promises.remove(&(r.addr as usize));
                self.regexes.remove(&(r.addr as usize));
                self.bigints.remove(&(r.addr as usize));
                self.proxies.remove(&(r.addr as usize));
                self.free.entry(r.field_count).or_default().push(idx);
                freed += 1;
            } else if r.alive && (!young_only || r.generation == records::Generation::Young) {
                // A full collection is also the promotion boundary. Future
                // nursery collections can skip these records and rely on a
                // remembered set for old-to-young writes.
                r.generation = records::Generation::Old;
            }
        }
        if evacuate {
            // Dead young records still point into the nursery chunks that are
            // about to be released. Do not put those records back on the free
            // list: reusing their addresses after the chunks are cleared would
            // turn the next allocation into a use-after-free.
            for free_indices in self.free.values_mut() {
                free_indices.retain(|&idx| self.records[idx].generation != records::Generation::Young);
            }
        }
        for entry in &mut self.finalizers {
            if entry.target != 0 && !entry.queued && self.by_addr.get(&entry.target).is_none() {
                entry.queued = true;
                self.finalization_queue.push(super::FinalizationRecord {
                    token: entry.token,
                    callback_id: entry.callback_id,
                    callback: entry.callback,
                });
            }
        }
        self.remembered_set.retain(|address| self.by_addr.contains_key(address));
        if self.records.iter().all(|record| !record.alive || record.generation == records::Generation::Old) {
            self.remembered_set.clear();
        }
        if young_only {
            self.nursery_bytes = 0;
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

    /// Young-only counterpart to [`Heap::collect_with_native_roots`].
    pub unsafe fn collect_young_with_native_roots(
        &mut self,
        roots: &[Obj],
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
    ) -> usize {
        super::with_active_stack_map_registry(|registry| {
            let Some(registry) = registry else {
                return unsafe { self.collect_young_with_native_roots(roots, ref_slots) };
            };
            let mut all_roots = roots.to_vec();
            append_native_roots(&mut all_roots);
            unsafe {
                self.collect_young_with_native_frame(&all_roots, registry, pc, sp, ref_slots)
            }
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
        ref_slots: &dyn Fn(u32) -> Vec<usize>,
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
