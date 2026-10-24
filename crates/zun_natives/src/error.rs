//! `zun:error` — the runtime side of the `Error` surface.
//!
//! `captureStackTrace` records a stack trace at `Error` construction time. The
//! real work needs the interpreter's live frame stack and the module's
//! sequence-point debug info, so the interpreter intercepts it on its managed
//! floor (see `zun_interp::collections`), exactly like the collection and
//! `zun:typeinfo` intrinsics. The implementation here is the heap-free fallback
//! used only if the function is ever dispatched outside that floor: it has no
//! access to the frame stack and so returns an empty trace (the `Error` getter
//! still renders its `name`/`message` header on top of it).

use crate::{NativeArg, NativeError, NativeTy::*, NativeFn, NativeValue, Sink};

/// Heap-free fallback (see the module note): no frame stack is reachable here,
/// so the captured trace is empty.
fn capture_stack_trace(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Str(String::new()))
}

/// The runtime side of the `Error` surface.
pub const MODULE: crate::Module = crate::Module {
    name: "zun:error",
    functions: &[NativeFn {
        // Capture the current call stack as the frame lines of a `.stack`
        // string (one `    at <fn> (<span>)` per active frame, innermost
        // first). The `name: message` header is prepended by the managed
        // `Error.stack` getter, which knows the concrete subclass.
        name: "captureStackTrace",
        params: &[],
        ret: Str,
        imp: capture_stack_trace,
    }],
};
