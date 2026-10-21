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
    let mut roots: Vec<zun_gc::Obj> = Vec::new();
    roots.extend_from_slice(extra_roots);
    collect_frame_roots(stack, locals, &mut roots);
    let mut link = parent;
    while let Some(l) = link {
        collect_frame_roots(l.stack, l.locals, &mut roots);
        link = l.parent;
    }
    zun_gc::append_native_roots(&mut roots);
    let ref_fn = |type_rid: u32| types.reference_slots(type_rid);
    // SAFETY: roots contains every live interpreter reference at this safepoint.
    zun_gc::with_current_heap_mut(|h| unsafe { h.collect(&roots, &ref_fn) })
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
            Value::Function(handle) => {
                if let Some(address) = handle.environment() {
                    roots.push(unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
                }
            }
            _ => {}
        }
    }
}
