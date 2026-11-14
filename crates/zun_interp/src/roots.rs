//! Interpreter frame root extraction for the managed heap.

use super::value::Value;
use super::RootLink;

/// Run a full, non-moving collection with precise roots: the current frame, the
/// linked caller chain, the interpreter's side tables, and every active native
/// root scope and [`zun_gc::Rooted`] guard.
pub(super) fn collect(
    types: &zun_types::TypeSystem,
    stack: &[Value],
    locals: &[Value],
    parent: Option<&RootLink>,
    extra_roots: &[zun_gc::Obj],
) -> usize {
    let mut roots: Vec<zun_gc::Obj> = Vec::new();
    roots.extend_from_slice(extra_roots);
    collect_frame_roots(stack, locals, &mut roots);
    if let Some(link) = parent {
        unsafe { link.append_roots(&mut roots) };
    }
    zun_gc::append_native_roots(&mut roots);
    let ref_fn = |type_rid: u32| types.reference_slots(type_rid);
    // SAFETY: roots contains every live interpreter reference at this safepoint.
    zun_gc::with_current_heap_mut(|heap| unsafe { heap.collect(&roots, &ref_fn) })
}

/// Run an evacuating minor collection for an explicitly stopped interpreter
/// chain.  Unlike `collect_young`, this path rewrites the current activation,
/// every linked caller, and the external roots in the same relocation
/// callback.  It is kept separate from the normal policy until all callers
/// can provide the complete side-table root contract.
pub(super) fn collect_young_evacuating(
    types: &zun_types::TypeSystem,
    stack: &mut [Value],
    locals: &mut [Value],
    parent: Option<&RootLink>,
    extra_roots: &[zun_gc::Obj],
    rewrite_external: &mut dyn FnMut(&zun_gc::RelocationMap),
) -> usize {
    let mut roots = Vec::new();
    roots.extend_from_slice(extra_roots);
    collect_frame_roots(stack, locals, &mut roots);
    if let Some(link) = parent {
        unsafe { link.append_roots(&mut roots) };
    }
    zun_gc::append_native_roots(&mut roots);
    let ref_fn = |type_rid: u32| types.reference_slots(type_rid);
    zun_gc::with_current_heap_mut(|heap| unsafe {
        heap.collect_young_evacuating(&mut roots, &ref_fn, &mut |relocation| {
            let _ = Value::rewrite_managed_references(stack, relocation);
            let _ = Value::rewrite_managed_references(locals, relocation);
            if let Some(link) = parent {
                let _ = link.rewrite_managed_references(relocation);
            }
            rewrite_external(relocation);
        })
    })
}

/// Run a full collection and compact the heap while the current activation's
/// value storage is exclusively borrowed by the interpreter. Caller frames
/// are intentionally excluded; nested execution remains on mark/sweep until
/// its owned frame chain can be rewritten as one unit.
pub(super) fn collect_and_compact_current(
    types: &zun_types::TypeSystem,
    stack: &mut [Value],
    locals: &mut [Value],
    extra_roots: &[zun_gc::Obj],
    rewrite_external: &mut dyn FnMut(&zun_gc::RelocationMap),
) -> usize {
    let mut roots = Vec::new();
    roots.extend_from_slice(extra_roots);
    collect_frame_roots(stack, locals, &mut roots);
    // Every sibling collector reports the registry's guards; this one used to
    // skip them, so an object held only by a `Rooted` or `RootArray` (a parked
    // caller chain across a JIT bridge, a native's local) was swept, and the
    // compaction then rewrote registry slots that no longer named a live object.
    zun_gc::append_native_roots(&mut roots);
    let ref_fn = |type_rid: u32| types.reference_slots(type_rid);
    let mut rewritten = 0;
    zun_gc::with_current_heap_mut(|heap| unsafe {
        let result =
            heap.collect_and_compact_with_relocation(&mut roots, &ref_fn, &mut |relocation| {
                rewritten += Value::rewrite_managed_references(stack, relocation);
                rewritten += Value::rewrite_managed_references(locals, relocation);
                rewrite_external(relocation);
            });
        result.reclaimed_objects + rewritten
    })
}

/// The managed address a frame value holds, if any. The single definition of
/// which carriers are frame roots, shared by [`collect_frame_roots`] and
/// [`restore_frame_roots`] so their orders cannot drift apart.
fn managed_address(value: &Value) -> Option<usize> {
    match value {
        Value::Object(object) => Some(object.addr() as usize),
        Value::Any(any) => any.as_object(),
        Value::Function(handle) => handle.environment(),
        _ => None,
    }
}

/// Add object references found in a frame's stack and locals to `roots`.
pub(crate) fn collect_frame_roots(stack: &[Value], locals: &[Value], roots: &mut Vec<zun_gc::Obj>) {
    for value in stack.iter().chain(locals.iter()) {
        if let Some(address) = managed_address(value) {
            roots.push(unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
        }
    }
}

/// Write relocated addresses back into a frame whose roots were registered as
/// a [`zun_gc::RootArray`] built from [`collect_frame_roots`] over the same
/// values (stack, then locals). `cursor` is the index of the frame's first
/// root in the array and advances past the frame's last.
///
/// A registered array is rewritten by the collector, but a frame is a copy of
/// those addresses: a native that parks an activation while code it does not
/// control collects (a JIT body reaching the interpreter through the bridge)
/// has to copy the answers back, or the activation resumes on stale pointers.
pub(crate) fn restore_frame_roots(
    stack: &mut [Value],
    locals: &mut [Value],
    roots: &zun_gc::RootArray,
    cursor: &mut usize,
) {
    for value in stack.iter_mut().chain(locals.iter_mut()) {
        if managed_address(value).is_none() {
            continue;
        }
        let relocated = roots.get(*cursor);
        *cursor += 1;
        let Some(object) = relocated else { continue };
        let address = object.addr() as usize;
        match value {
            Value::Object(slot) => *slot = object,
            Value::Any(any) => *any = any.map_object_address(|_| address),
            Value::Function(handle) => *handle = handle.relocate_environment(Some(address)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::collect_frame_roots;
    use crate::value::Value;

    #[test]
    fn object_tagged_any_values_are_frame_roots() {
        let mut roots = Vec::new();
        collect_frame_roots(
            &[Value::Any(zun_core::Any::object(0x1000))],
            &[],
            &mut roots,
        );
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].addr() as usize, 0x1000);
    }
}

#[cfg(test)]
mod restore_tests {
    use super::{collect_frame_roots, restore_frame_roots};
    use crate::value::Value;

    fn obj(address: usize) -> zun_gc::Obj {
        // SAFETY: the address is only compared, never dereferenced.
        unsafe { zun_gc::Obj::from_addr(address as *mut u64) }
    }

    #[test]
    fn restore_copies_the_registry_answers_back_in_collection_order() {
        let mut stack = [Value::Object(obj(0x1000)), Value::Int(1, zun_il::NumKind::I32)];
        let mut locals = [
            Value::Any(zun_core::Any::object(0x2000)),
            Value::Void,
            Value::Object(obj(0x3000)),
        ];
        let mut registered = Vec::new();
        collect_frame_roots(&stack, &locals, &mut registered);
        assert_eq!(registered.len(), 3);
        let roots = zun_gc::RootArray::from_objects(registered);

        // What the collector does to the registry when it moves the objects.
        roots.set(0, obj(0x1100));
        roots.set(1, obj(0x2200));
        roots.set(2, obj(0x3300));

        let mut cursor = 0;
        restore_frame_roots(&mut stack, &mut locals, &roots, &mut cursor);

        assert_eq!(cursor, 3);
        assert!(matches!(stack[0], Value::Object(o) if o.addr() as usize == 0x1100));
        assert!(matches!(stack[1], Value::Int(1, _)));
        assert!(matches!(locals[0], Value::Any(a) if a.as_object() == Some(0x2200)));
        assert!(matches!(locals[2], Value::Object(o) if o.addr() as usize == 0x3300));
    }
}
