//! Interpreter frame root extraction for the managed heap.

use super::value::Value;
use super::RootLink;

pub(super) fn collect(
    types: &zun_types::TypeSystem,
    stack: &[Value],
    locals: &[Value],
    parent: Option<&RootLink>,
    extra_roots: &[zun_gc::Obj],
) -> usize {
    collect_with_policy(types, stack, locals, parent, extra_roots, false)
}

pub(super) fn collect_young(
    types: &zun_types::TypeSystem,
    stack: &[Value],
    locals: &[Value],
    parent: Option<&RootLink>,
    extra_roots: &[zun_gc::Obj],
) -> usize {
    collect_with_policy(types, stack, locals, parent, extra_roots, true)
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
    let ref_fn = |type_rid: u32| types.reference_slots(type_rid);
    let mut rewritten = 0;
    zun_gc::with_current_heap_mut(|heap| unsafe {
        let result = heap.collect_and_compact_with_relocation(
            &mut roots,
            &ref_fn,
            &mut |relocation| {
                rewritten += Value::rewrite_managed_references(stack, relocation);
                rewritten += Value::rewrite_managed_references(locals, relocation);
                rewrite_external(relocation);
            },
        );
        result.reclaimed_objects + rewritten
    })
}

fn collect_with_policy(
    types: &zun_types::TypeSystem,
    stack: &[Value],
    locals: &[Value],
    parent: Option<&RootLink>,
    extra_roots: &[zun_gc::Obj],
    young_only: bool,
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
    zun_gc::with_current_heap_mut(|h| unsafe {
        if young_only {
            h.collect_young(&roots, &ref_fn)
        } else {
            h.collect(&roots, &ref_fn)
        }
    })
}

/// Add object references found in a frame's stack and locals to `roots`.
pub(crate) fn collect_frame_roots(
    stack: &[Value],
    locals: &[Value],
    roots: &mut Vec<zun_gc::Obj>,
) {
    for value in stack.iter().chain(locals.iter()) {
        match value {
            Value::Object(object) => roots.push(*object),
            Value::Any(any) => {
                if let Some(address) = any.as_object() {
                    roots.push(unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                }
            }
            Value::Function(handle) => {
                if let Some(address) = handle.environment() {
                    roots.push(unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                }
            }
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
