//! `zun:core` `timers` namespace — the macrotask surface of the VM's unified
//! event loop (`setTimeout`/`setInterval`/`clearTimeout`/`clearInterval`,
//! `queueMicrotask`, `setImmediate`).
//!
//! Timers are a **core capability**, not an external library: the VM owns the
//! event loop, and the macrotask tier that fires timers is the same loop that
//! already drains Promise microtasks (PLAN §5.1). These signatures only fix the
//! dispatch ids and the shape the checker sees; every implementation needs the
//! interpreter's scheduler, timer heap and managed heap, so each is serviced on
//! the interpreter floor (`call_managed_collection`) exactly like the Promise
//! and collection intrinsics. The bodies here are therefore unreachable stubs.
//!
//! The callback parameter is `Opaque`: it arrives as a raw managed value (a
//! function), dispatched on by the interpreter floor. A handle is a `number`
//! (`F64`) — the scheduler [`TaskId`] it names.

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

fn unavailable(_: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Err(NativeError(
        "timer intrinsic requires an interpreter heap and scheduler".into(),
    ))
}

/// The `timers` namespace. Appended last in [`crate::MODULES`] so every existing
/// dispatch id stays stable.
pub const MODULE: crate::Module = crate::Module {
    name: "timers",
    functions: &[
        // `setTimeout(cb, ms)` → numeric handle. The optional `ms` is defaulted
        // to `0` by the checker when omitted.
        NativeFn { name: "setTimeout", params: &[Opaque, F64], ret: F64, imp: unavailable },
        // `setInterval(cb, ms)` → numeric handle (repeats).
        NativeFn { name: "setInterval", params: &[Opaque, F64], ret: F64, imp: unavailable },
        NativeFn { name: "clearTimeout", params: &[F64], ret: Void, imp: unavailable },
        NativeFn { name: "clearInterval", params: &[F64], ret: Void, imp: unavailable },
        // `queueMicrotask(cb)` → enqueue a microtask directly.
        NativeFn { name: "queueMicrotask", params: &[Opaque], ret: Void, imp: unavailable },
        // `setImmediate(cb)` → a 0-delay macrotask; returns a numeric handle.
        NativeFn { name: "setImmediate", params: &[Opaque], ret: F64, imp: unavailable },
    ],
};
