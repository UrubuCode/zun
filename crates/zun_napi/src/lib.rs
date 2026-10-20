//! Minimal stable host boundary used by the eventual Node N-API adapter.
//!
//! The boundary accepts compiled `.zm` bytes, never compiler ASTs. Node-facing
//! code can wrap these functions without making the VM depend on Node headers.

use std::slice;

#[cfg(feature = "node-api")]
mod node_api {
    use core::ffi::c_void;

    /// Opaque N-API environment handle.
    type NapiEnv = *mut c_void;
    /// Opaque N-API JavaScript value handle.
    type NapiValue = *mut c_void;
    type NapiCallbackInfo = *mut c_void;
    type NapiStatus = i32;
    type NapiCallback = unsafe extern "C" fn(NapiEnv, NapiCallbackInfo) -> NapiValue;

    const NAPI_OK: NapiStatus = 0;

    extern "C" {
        fn napi_create_function(
            env: NapiEnv,
            utf8name: *const u8,
            length: usize,
            cb: NapiCallback,
            data: *mut c_void,
            result: *mut NapiValue,
        ) -> NapiStatus;
        fn napi_set_named_property(
            env: NapiEnv,
            object: NapiValue,
            utf8name: *const u8,
            value: NapiValue,
        ) -> NapiStatus;
        fn napi_get_cb_info(
            env: NapiEnv,
            cbinfo: NapiCallbackInfo,
            argc: *mut usize,
            argv: *mut NapiValue,
            this_arg: *mut NapiValue,
            data: *mut *mut c_void,
        ) -> NapiStatus;
        fn napi_get_arraybuffer_info(
            env: NapiEnv,
            value: NapiValue,
            data: *mut *mut c_void,
            byte_length: *mut usize,
        ) -> NapiStatus;
        fn napi_create_int64(env: NapiEnv, value: i64, result: *mut NapiValue) -> NapiStatus;
    }

    unsafe extern "C" fn run_i64(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let mut argc = 1;
        let mut argv = core::ptr::null_mut();
        if unsafe {
            napi_get_cb_info(
                env,
                info,
                &mut argc,
                &mut argv,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        } != NAPI_OK
            || argc != 1
        {
            return core::ptr::null_mut();
        }
        let mut data = core::ptr::null_mut();
        let mut length = 0;
        if unsafe { napi_get_arraybuffer_info(env, argv, &mut data, &mut length) } != NAPI_OK
            || data.is_null()
        {
            return core::ptr::null_mut();
        }
        // SAFETY: N-API provides a live ArrayBuffer for the duration of this
        // callback and reports its exact byte length.
        let bytes = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), length) };
        let Ok(zun_interp::value::Value::Int(value, _)) = zun_vm::execute(bytes) else {
            return core::ptr::null_mut();
        };
        let mut result = core::ptr::null_mut();
        if unsafe { napi_create_int64(env, value, &mut result) } != NAPI_OK {
            core::ptr::null_mut()
        } else {
            result
        }
    }

    /// N-API module entry point. `exports.runI64(arrayBuffer)` executes an
    /// integer-returning compiled `.zm` module.
    #[no_mangle]
    pub unsafe extern "C" fn napi_register_module_v1(
        env: NapiEnv,
        exports: NapiValue,
    ) -> NapiValue {
        let mut function = core::ptr::null_mut();
        let name = b"runI64";
        if unsafe {
            napi_create_function(
                env,
                name.as_ptr(),
                name.len(),
                run_i64,
                core::ptr::null_mut(),
                &mut function,
            )
        } == NAPI_OK
        {
            let _ = unsafe { napi_set_named_property(env, exports, name.as_ptr(), function) };
        }
        exports
    }
}

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

/// Execute a compiled module and return its boolean result through `out`.
/// The status codes match [`zun_host_execute_i64`].
#[no_mangle]
pub unsafe extern "C" fn zun_host_execute_bool(
    host: *mut ZunHost,
    module_ptr: *const u8,
    module_len: usize,
    out: *mut bool,
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
    let zun_interp::value::Value::Bool(value) = value else {
        return 2;
    };
    // SAFETY: caller supplies writable storage for one bool.
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

    #[test]
    fn host_executes_a_compiled_boolean_module() {
        let module = zun_compiler::compile("function main(): boolean { return true; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = false;
        let status = unsafe {
            zun_host_execute_bool(host, bytes.as_ptr(), bytes.len(), &mut result)
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 0);
        assert!(result);
    }
}
