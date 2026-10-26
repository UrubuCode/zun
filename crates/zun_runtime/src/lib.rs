//! The Zun shared runtime (PLAN section 4.1): one engine, many small `.zm` files.
//!
//! # The `zun_rt_*` C ABI contract
//!
//! This is the stable boundary between the shared engine and a `.zm` payload.
//! Symbols are plain C, no Rust types cross it.
//!
//! * `zun_rt_abi_version() -> u32` - contract version ([`ABI_VERSION`]); a host
//!   must refuse to load an engine whose version differs from the one it expects.
//! * `zun_rt_run(ptr, len) -> i32` - load the `.zm` bytes at `ptr[..len]`, verify
//!   and run it to completion. Returns [`ZUN_RT_OK`] on success, or a negative
//!   `ZUN_RT_ERR_*` code. Never unwinds across the boundary.
//!
//! * `zun_rt_has_eval() -> u32` - (ABI 2) 1 when built with cargo feature `eval`.
//! * `zun_rt_run_eval(ptr, len) -> i32` - (ABI 2) like `zun_rt_run` but with the
//!   compiler-backed `EvalHost`, so `new Function(...)` works; returns
//!   [`ZUN_RT_ERR_UNSUPPORTED`] when built without `eval`. Programs whose module
//!   declares the `codegen` capability must use this entry.
//!
//! `zun_rt_run` uses the AOT-style no-host path (`zun_vm::execute_aot`) in both
//! build flavours: stdout output, no `EvalHost`, so `new Function(...)` is
//! disallowed, same as a static AOT
//! binary. The module bytes are only borrowed for the duration of the call.

use std::panic::{catch_unwind, AssertUnwindSafe};

/// Version of the `zun_rt_*` contract. Bump on any incompatible ABI change.
pub const ABI_VERSION: u32 = 2;
/// The module ran to completion.
pub const ZUN_RT_OK: i32 = 0;
/// Null pointer or zero length.
pub const ZUN_RT_ERR_BAD_ARGS: i32 = -1;
/// The module failed to load, verify, or execute.
pub const ZUN_RT_ERR_RUN: i32 = -2;
/// A panic was caught at the FFI boundary.
pub const ZUN_RT_ERR_PANIC: i32 = -3;

/// Returns the `zun_rt_*` contract version.
#[no_mangle]
pub extern "C" fn zun_rt_abi_version() -> u32 {
    ABI_VERSION
}

/// Load and run a `.zm` byte buffer to completion.
///
/// # Safety
/// `ptr` must be null or valid for reads of `len` bytes for the call duration.
#[no_mangle]
pub unsafe extern "C" fn zun_rt_run(ptr: *const u8, len: usize) -> i32 {
    if ptr.is_null() || len == 0 {
        return ZUN_RT_ERR_BAD_ARGS;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    match catch_unwind(AssertUnwindSafe(|| zun_vm::execute_aot(bytes))) {
        Ok(Ok(_)) => ZUN_RT_OK,
        Ok(Err(_)) => ZUN_RT_ERR_RUN,
        Err(_) => ZUN_RT_ERR_PANIC,
    }
}

/// This build does not include the compiler (`eval` feature off).
pub const ZUN_RT_ERR_UNSUPPORTED: i32 = -4;

/// Whether this build links the compiler-backed `EvalHost` (feature `eval`).
#[no_mangle]
pub extern "C" fn zun_rt_has_eval() -> u32 {
    cfg!(feature = "eval") as u32
}

/// Load and run a `.zm` byte buffer with the compiler-backed `EvalHost`, so
/// `new Function(...)` works. Needs the `eval` feature.
///
/// # Safety
/// `ptr` must be null or valid for reads of `len` bytes for the call duration.
#[no_mangle]
pub unsafe extern "C" fn zun_rt_run_eval(ptr: *const u8, len: usize) -> i32 {
    if ptr.is_null() || len == 0 {
        return ZUN_RT_ERR_BAD_ARGS;
    }
    #[cfg(feature = "eval")]
    {
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        // `zun_vm::execute` injects `CompilerEvalHost` when `zun_vm/eval` is on.
        match catch_unwind(AssertUnwindSafe(|| zun_vm::execute(bytes))) {
            Ok(Ok(_)) => ZUN_RT_OK,
            Ok(Err(_)) => ZUN_RT_ERR_RUN,
            Err(_) => ZUN_RT_ERR_PANIC,
        }
    }
    #[cfg(not(feature = "eval"))]
    {
        ZUN_RT_ERR_UNSUPPORTED
    }
}
