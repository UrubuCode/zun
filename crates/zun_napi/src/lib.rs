//! Minimal stable host boundary used by the eventual Node N-API adapter.
//!
//! The boundary accepts compiled `.zm` bytes, never compiler ASTs. Node-facing
//! code can wrap these functions without making the VM depend on Node headers.

use std::slice;

/// Opaque host state reserved for scheduler and module-loader state.
#[repr(C)]
#[derive(Debug)]
pub struct ZunHost {
    _private: (),
}

/// Create an embedding host.
#[no_mangle]
pub extern "C" fn zun_host_create() -> *mut ZunHost {
    Box::into_raw(Box::new(ZunHost { _private: () }))
}

/// Destroy an embedding host previously returned by [`zun_host_create`].
///
/// # Safety
/// `host` must be null or a pointer returned by `zun_host_create`, used at
/// most once.
#[no_mangle]
pub unsafe extern "C" fn zun_host_destroy(host: *mut ZunHost) {
    if !host.is_null() {
        // SAFETY: guaranteed by the function's ownership contract.
        drop(unsafe { Box::from_raw(host) });
    }
}

/// Execute a compiled module and return its integer result through `out`.
///
/// Returns zero on success, one for invalid pointers, and two for a VM error.
/// The host pointer is currently a lifetime/ownership anchor; VM execution is
/// isolated per call until scheduler state is added to the embedding layer.
#[no_mangle]
pub unsafe extern "C" fn zun_host_execute_i64(
    host: *mut ZunHost,
    module_ptr: *const u8,
    module_len: usize,
    out: *mut i64,
) -> i32 {
    if host.is_null() || module_ptr.is_null() || out.is_null() {
        return 1;
    }
    // SAFETY: caller supplies a valid immutable byte range for this call.
    let bytes = unsafe { slice::from_raw_parts(module_ptr, module_len) };
    let value = match zun_vm::execute(bytes) {
        Ok(value) => value,
        Err(_) => return 2,
    };
    let zun_interp::value::Value::Int(value, _) = value else {
        return 2;
    };
    // SAFETY: caller supplies writable storage for one i64.
    unsafe { *out = value };
    0
}

/// Execute a compiled module and return its floating-point result through
/// `out`. The status codes match [`zun_host_execute_i64`].
#[no_mangle]
pub unsafe extern "C" fn zun_host_execute_f64(
    host: *mut ZunHost,
    module_ptr: *const u8,
    module_len: usize,
    out: *mut f64,
) -> i32 {
    if host.is_null() || module_ptr.is_null() || out.is_null() {
        return 1;
    }
    // SAFETY: caller supplies a valid immutable byte range for this call.
    let bytes = unsafe { slice::from_raw_parts(module_ptr, module_len) };
    let value = match zun_vm::execute(bytes) {
        Ok(value) => value,
        Err(_) => return 2,
    };
    let zun_interp::value::Value::Float(value, _) = value else {
        return 2;
    };
    // SAFETY: caller supplies writable storage for one f64.
    unsafe { *out = value };
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_host_arguments_are_rejected() {
        assert_eq!(unsafe { zun_host_execute_i64(std::ptr::null_mut(), std::ptr::null(), 0, std::ptr::null_mut()) }, 1);
    }

    #[test]
    fn host_executes_a_compiled_integer_module() {
        let module = zun_compiler::compile("function main(): int { return 7; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = 0;
        let status = unsafe {
            zun_host_execute_i64(host, bytes.as_ptr(), bytes.len(), &mut result)
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 0);
        assert_eq!(result, 7);
    }

    #[test]
    fn host_executes_a_compiled_float_module() {
        let module = zun_compiler::compile("function main(): number { return 7.5; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = 0.0;
        let status = unsafe {
            zun_host_execute_f64(host, bytes.as_ptr(), bytes.len(), &mut result)
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 0);
        assert_eq!(result, 7.5);
    }
}
