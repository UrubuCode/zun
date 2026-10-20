//! Interpreter frame root extraction for the managed heap.

use super::value::Value;

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
