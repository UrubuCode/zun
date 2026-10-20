//! Mark-sweep collection and root-expanding collection entry points.

use super::*;

impl Heap {
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
            }
            if !promoted {
                break;
            }
        }

        // Weak references observe the completed mark set but do not contribute
        // roots. Clear dead targets before sweep removes their addresses.
        for entry in &mut self.weak_maps {
            entry.entries.retain(|(key, value)| {
                self.by_addr
                    .get(key)
                    .is_some_and(|idx| self.records[*idx].marked)
                    && self
                        .by_addr
                        .get(value)
                        .is_some_and(|idx| self.records[*idx].marked)
            });
        }
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
                self.typed_views.remove(&(r.addr as usize));
                self.array_iterators.remove(&(r.addr as usize));
                self.set_iterators.remove(&(r.addr as usize));
                self.map_iterators.remove(&(r.addr as usize));
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
            unsafe { self.collect_with_native_frame(roots, registry, pc, sp, ref_slots) }
        })
    }

}
