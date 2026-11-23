//! The strong out-edges of one managed object.
//!
//! One definition shared by the marker and by the remembered-set verifier, so
//! the two can never disagree about which side tables an object's liveness
//! flows through.

use super::*;

impl Heap {
    /// Append the address of every managed object that `obj` keeps alive, in no
    /// particular order and possibly with repeats. Zero (null) is never emitted.
    ///
    /// Ephemeron values are not edges of the owner: they flow from the *key*, and
    /// the marker resolves them separately.
    pub(crate) fn children_into(
        &self,
        obj: Obj,
        ref_slots: &dyn Fn(u32) -> std::rc::Rc<[usize]>,
        out: &mut Vec<usize>,
    ) {
        let key = obj.addr() as usize;
        for &slot in ref_slots(obj.type_rid()).iter() {
            let child = obj.field(slot) as usize;
            if child != 0 {
                out.push(child);
            }
        }
        if !self.dynamic_objects.is_empty() {
            if let Some(properties) = self.dynamic_objects.get(&key) {
                out.extend(properties.object_references());
            }
        }
        if !self.generator_frames.is_empty() || !self.generator_states.is_empty() {
            out.extend(self.trace_generator_frame(obj));
            out.extend(self.trace_generator_state(obj));
        }
        // An iterator keeps its source container alive through the side tables
        // (its header slot is a raw address, not a typed reference). A typed
        // array and a DataView reach their ArrayBuffer the same way, so those
        // edges belong here too: a buffer referenced only by its view was
        // otherwise swept while the view was still alive.
        if !self.typed_views.is_empty() {
            if let Some((buffer, _)) = self.typed_views.get(&key) {
                out.push(buffer.addr() as usize);
            }
        }
        if !self.data_views.is_empty() && self.data_views.contains_key(&key) {
            out.push(obj.field(0) as usize);
        }
        if !self.array_iterators.is_empty() {
            if let Some((source, _)) = self.array_iterators.get(&key) {
                out.push(source.addr() as usize);
            }
        }
        if !self.set_iterators.is_empty() {
            if let Some((source, _)) = self.set_iterators.get(&key) {
                out.push(source.addr() as usize);
            }
        }
        if !self.map_iterators.is_empty() {
            if let Some((source, _)) = self.map_iterators.get(&key) {
                out.push(source.addr() as usize);
            }
        }
        if !self.proxies.is_empty() {
            if let Some(proxy) = self.proxies.get(&key) {
                out.push(proxy.target.addr() as usize);
                out.push(proxy.handler.addr() as usize);
            }
        }
        if !self.arrays.is_empty() {
            if let Some(array) = self.arrays.get(&key) {
                out.extend(array.object_references());
            }
        }
        if !self.any_fields.is_empty() {
            if let Some(fields) = self.any_fields.get(&key) {
                out.extend(fields.values().filter_map(|value| value.as_object()));
            }
        }
        if !self.maps.is_empty() {
            if let Some(map) = self.maps.get(&key) {
                for (k, v) in map.iter() {
                    out.extend([k, v].into_iter().filter_map(|item| item.as_object()));
                }
            }
        }
        if !self.sets.is_empty() {
            if let Some(set) = self.sets.get(&key) {
                out.extend(set.iter().filter_map(|(k, _)| k.as_object()));
            }
        }
        if !self.promises.is_empty() {
            if let Some(promise) = self.promises.get(&key) {
                out.extend(
                    [
                        promise.result().and_then(|value| value.as_object()),
                        promise.reason().and_then(|value| value.as_object()),
                    ]
                    .into_iter()
                    .flatten(),
                );
            }
        }
    }
}
