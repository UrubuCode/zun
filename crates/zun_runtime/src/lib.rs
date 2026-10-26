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
//! The run uses the AOT-style no-host path (`zun_vm::execute_aot`): stdout output
//! and no `EvalHost`, so `new Function(...)` is disallowed, same as a static AOT
//! binary. The module bytes are only borrowed for the duration of the call.

use std::panic::{catch_unwind, AssertUnwindSafe};

/// Version of the `zun_rt_*` contract. Bump on any incompatible ABI change.
pub const ABI_VERSION: u32 = 1;
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
