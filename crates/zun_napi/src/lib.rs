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
        fn napi_get_buffer_info(
            env: NapiEnv,
            value: NapiValue,
            data: *mut *mut c_void,
            byte_length: *mut usize,
        ) -> NapiStatus;
        fn napi_create_int64(env: NapiEnv, value: i64, result: *mut NapiValue) -> NapiStatus;
        fn napi_create_bigint_uint64(env: NapiEnv, value: u64, result: *mut NapiValue) -> NapiStatus;
        fn napi_create_double(env: NapiEnv, value: f64, result: *mut NapiValue) -> NapiStatus;
        fn napi_get_boolean(env: NapiEnv, value: bool, result: *mut NapiValue) -> NapiStatus;
        fn napi_create_string_utf8(
            env: NapiEnv,
            str: *const u8,
            length: usize,
            result: *mut NapiValue,
        ) -> NapiStatus;
        fn napi_get_undefined(env: NapiEnv, result: *mut NapiValue) -> NapiStatus;
        fn napi_throw_type_error(env: NapiEnv, code: *const u8, message: *const u8) -> NapiStatus;
        fn napi_throw_error(env: NapiEnv, code: *const u8, message: *const u8) -> NapiStatus;
    }

    fn undefined(env: NapiEnv) -> NapiValue {
        let mut value = core::ptr::null_mut();
        if unsafe { napi_get_undefined(env, &mut value) } == NAPI_OK {
            value
        } else {
            core::ptr::null_mut()
        }
    }

    fn invalid_call(env: NapiEnv) -> NapiValue {
        let message = b"expected an ArrayBuffer containing a valid Zun module\0";
        let _ = unsafe { napi_throw_type_error(env, core::ptr::null(), message.as_ptr()) };
        undefined(env)
    }

    fn registration_failed(env: NapiEnv) -> NapiValue {
        let message = b"failed to register Zun N-API exports\0";
        let _ = unsafe { napi_throw_error(env, core::ptr::null(), message.as_ptr()) };
        undefined(env)
    }

    unsafe extern "C" fn run_i64(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let mut argc = 1;
        let mut argv = [core::ptr::null_mut(); 1];
        if unsafe {
            napi_get_cb_info(
                env,
                info,
                &mut argc,
                argv.as_mut_ptr(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        } != NAPI_OK
            || argc != 1
        {
            return invalid_call(env);
        }
        let mut data = core::ptr::null_mut();
        let mut length = 0;
        let status = unsafe { napi_get_arraybuffer_info(env, argv[0], &mut data, &mut length) };
        let status = if status == NAPI_OK {
            status
        } else {
            unsafe { napi_get_buffer_info(env, argv[0], &mut data, &mut length) }
        };
        if status != NAPI_OK || data.is_null() {
            return invalid_call(env);
        }
        // SAFETY: N-API provides a live ArrayBuffer for the duration of this
        // callback and reports its exact byte length.
        let bytes = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), length) };
        let Ok(zun_interp::value::Value::Int(value, kind)) = zun_vm::execute(bytes) else {
            return invalid_call(env);
        };
        if !matches!(kind, zun_il::NumKind::I8 | zun_il::NumKind::I16 | zun_il::NumKind::I32 | zun_il::NumKind::I64) {
            return invalid_call(env);
        }
        let mut result = core::ptr::null_mut();
        if unsafe { napi_create_int64(env, value, &mut result) } != NAPI_OK {
            invalid_call(env)
        } else {
            result
        }
    }

    unsafe extern "C" fn run_u64(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let mut argc = 1;
        let mut argv = [core::ptr::null_mut(); 1];
        if unsafe {
            napi_get_cb_info(
                env,
                info,
                &mut argc,
                argv.as_mut_ptr(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        } != NAPI_OK
            || argc != 1
        {
            return invalid_call(env);
        }
        let mut data = core::ptr::null_mut();
        let mut length = 0;
        let status = unsafe { napi_get_arraybuffer_info(env, argv[0], &mut data, &mut length) };
        let status = if status == NAPI_OK {
            status
        } else {
            unsafe { napi_get_buffer_info(env, argv[0], &mut data, &mut length) }
        };
        if status != NAPI_OK || data.is_null() {
            return invalid_call(env);
        }
        // SAFETY: N-API owns the ArrayBuffer for the callback duration.
        let bytes = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), length) };
        let Ok(zun_interp::value::Value::Int(value, zun_il::NumKind::U64)) = zun_vm::execute(bytes) else {
            return invalid_call(env);
        };
        let mut result = core::ptr::null_mut();
        if unsafe { napi_create_bigint_uint64(env, value as u64, &mut result) } != NAPI_OK {
            invalid_call(env)
        } else {
            result
        }
    }

    unsafe extern "C" fn run_f64(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let mut argc = 1;
        let mut argv = [core::ptr::null_mut(); 1];
        if unsafe {
            napi_get_cb_info(
                env,
                info,
                &mut argc,
                argv.as_mut_ptr(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        } != NAPI_OK
            || argc != 1
        {
            return invalid_call(env);
        }
        let mut data = core::ptr::null_mut();
        let mut length = 0;
        let status = unsafe { napi_get_arraybuffer_info(env, argv[0], &mut data, &mut length) };
        let status = if status == NAPI_OK {
            status
        } else {
            unsafe { napi_get_buffer_info(env, argv[0], &mut data, &mut length) }
        };
        if status != NAPI_OK || data.is_null() {
            return invalid_call(env);
        }
        // SAFETY: N-API owns the ArrayBuffer for the callback duration.
        let bytes = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), length) };
        let Ok(zun_interp::value::Value::Float(value, _)) = zun_vm::execute(bytes) else {
            return invalid_call(env);
        };
        let mut result = core::ptr::null_mut();
        if unsafe { napi_create_double(env, value, &mut result) } != NAPI_OK {
            invalid_call(env)
        } else {
            result
        }
    }

    unsafe extern "C" fn run_bool(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let mut argc = 1;
        let mut argv = [core::ptr::null_mut(); 1];
        if unsafe {
            napi_get_cb_info(
                env,
                info,
                &mut argc,
                argv.as_mut_ptr(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        } != NAPI_OK
            || argc != 1
        {
            return invalid_call(env);
        }
        let mut data = core::ptr::null_mut();
        let mut length = 0;
        let status = unsafe { napi_get_arraybuffer_info(env, argv[0], &mut data, &mut length) };
        let status = if status == NAPI_OK {
            status
        } else {
            unsafe { napi_get_buffer_info(env, argv[0], &mut data, &mut length) }
        };
        if status != NAPI_OK || data.is_null() {
            return invalid_call(env);
        }
        // SAFETY: N-API owns the ArrayBuffer for the callback duration.
        let bytes = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), length) };
        let Ok(zun_interp::value::Value::Bool(value)) = zun_vm::execute(bytes) else {
            return invalid_call(env);
        };
        let mut result = core::ptr::null_mut();
        if unsafe { napi_get_boolean(env, value, &mut result) } != NAPI_OK {
            invalid_call(env)
        } else {
            result
        }
    }

    unsafe extern "C" fn run_string(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
        let mut argc = 1;
        let mut argv = [core::ptr::null_mut(); 1];
        if unsafe {
            napi_get_cb_info(
                env,
                info,
                &mut argc,
                argv.as_mut_ptr(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        } != NAPI_OK
            || argc != 1
        {
            return invalid_call(env);
        }
        let mut data = core::ptr::null_mut();
        let mut length = 0;
        let status = unsafe { napi_get_arraybuffer_info(env, argv[0], &mut data, &mut length) };
        let status = if status == NAPI_OK {
            status
        } else {
            unsafe { napi_get_buffer_info(env, argv[0], &mut data, &mut length) }
        };
        if status != NAPI_OK || data.is_null() {
            return invalid_call(env);
        }
        // SAFETY: N-API owns the ArrayBuffer for the callback duration.
        let bytes = unsafe { core::slice::from_raw_parts(data.cast::<u8>(), length) };
        let Ok(zun_interp::value::Value::Str(value)) = zun_vm::execute(bytes) else {
            return invalid_call(env);
        };
        let value = value.as_bytes();
        let mut result = core::ptr::null_mut();
        if unsafe { napi_create_string_utf8(env, value.as_ptr(), value.len(), &mut result) }
            != NAPI_OK
        {
            invalid_call(env)
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
        for (name, callback) in [
            (b"runI64\0".as_slice(), run_i64 as NapiCallback),
            (b"runU64\0".as_slice(), run_u64 as NapiCallback),
            (b"runF64\0".as_slice(), run_f64 as NapiCallback),
            (b"runBool\0".as_slice(), run_bool as NapiCallback),
            (b"runString\0".as_slice(), run_string as NapiCallback),
        ] {
            let mut function = core::ptr::null_mut();
            let status = unsafe {
                napi_create_function(
                    env,
                    name.as_ptr(),
                    name.len() - 1,
                    callback,
                    core::ptr::null_mut(),
                    &mut function,
                )
            };
            if status != NAPI_OK {
                return registration_failed(env);
            }
            if unsafe { napi_set_named_property(env, exports, name.as_ptr(), function) }
                != NAPI_OK
            {
                return registration_failed(env);
            }
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

/// Successful host call.
pub const ZUN_HOST_OK: i32 = 0;
/// One or more host pointers or buffer arguments are invalid.
pub const ZUN_HOST_INVALID_ARGUMENT: i32 = 1;
/// The module failed during execution or returned the wrong value type.
pub const ZUN_HOST_VM_ERROR: i32 = 2;
/// The caller's output buffer is too small; the required size is returned.
pub const ZUN_HOST_BUFFER_TOO_SMALL: i32 = 3;

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
        return ZUN_HOST_INVALID_ARGUMENT;
    }
    // SAFETY: caller supplies a valid immutable byte range for this call.
    let bytes = unsafe { slice::from_raw_parts(module_ptr, module_len) };
    let value = match zun_vm::execute(bytes) {
        Ok(value) => value,
        Err(_) => return ZUN_HOST_VM_ERROR,
    };
    let zun_interp::value::Value::Int(value, kind) = value else {
        return ZUN_HOST_VM_ERROR;
    };
    if !matches!(kind, zun_il::NumKind::I8 | zun_il::NumKind::I16 | zun_il::NumKind::I32 | zun_il::NumKind::I64) {
        return ZUN_HOST_VM_ERROR;
    }
    // SAFETY: caller supplies writable storage for one i64.
    unsafe { *out = value };
    ZUN_HOST_OK
}

/// Execute a compiled module and return the raw unsigned integer result.
/// The value is copied without narrowing through the signed C ABI.
#[no_mangle]
pub unsafe extern "C" fn zun_host_execute_u64(
    host: *mut ZunHost,
    module_ptr: *const u8,
    module_len: usize,
    out: *mut u64,
) -> i32 {
    if host.is_null() || module_ptr.is_null() || out.is_null() {
        return ZUN_HOST_INVALID_ARGUMENT;
    }
    // SAFETY: caller supplies a valid immutable byte range for this call.
    let bytes = unsafe { slice::from_raw_parts(module_ptr, module_len) };
    let value = match zun_vm::execute(bytes) {
        Ok(value) => value,
        Err(_) => return ZUN_HOST_VM_ERROR,
    };
    let zun_interp::value::Value::Int(value, zun_il::NumKind::U64) = value else {
        return ZUN_HOST_VM_ERROR;
    };
    // SAFETY: caller supplies writable storage for one u64.
    unsafe { *out = value as u64 };
    ZUN_HOST_OK
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
        return ZUN_HOST_INVALID_ARGUMENT;
    }
    // SAFETY: caller supplies a valid immutable byte range for this call.
    let bytes = unsafe { slice::from_raw_parts(module_ptr, module_len) };
    let value = match zun_vm::execute(bytes) {
        Ok(value) => value,
        Err(_) => return ZUN_HOST_VM_ERROR,
    };
    let zun_interp::value::Value::Float(value, _) = value else {
        return ZUN_HOST_VM_ERROR;
    };
    // SAFETY: caller supplies writable storage for one f64.
    unsafe { *out = value };
    ZUN_HOST_OK
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
        return ZUN_HOST_INVALID_ARGUMENT;
    }
    // SAFETY: caller supplies a valid immutable byte range for this call.
    let bytes = unsafe { slice::from_raw_parts(module_ptr, module_len) };
    let value = match zun_vm::execute(bytes) {
        Ok(value) => value,
        Err(_) => return ZUN_HOST_VM_ERROR,
    };
    let zun_interp::value::Value::Bool(value) = value else {
        return ZUN_HOST_VM_ERROR;
    };
    // SAFETY: caller supplies writable storage for one bool.
    unsafe { *out = value };
    ZUN_HOST_OK
}

/// Execute a compiled module and copy its UTF-8 string result into `out`.
///
/// `written` receives the required byte length. Returns zero on success, one
/// for invalid pointers, two for a VM/type error, and three when `out_len` is
/// too small. A null `out` is accepted only when `out_len` is zero, allowing a
/// caller to query the required size first.
#[no_mangle]
pub unsafe extern "C" fn zun_host_execute_string(
    host: *mut ZunHost,
    module_ptr: *const u8,
    module_len: usize,
    out: *mut u8,
    out_len: usize,
    written: *mut usize,
) -> i32 {
    if host.is_null()
        || module_ptr.is_null()
        || written.is_null()
        || (out.is_null() && out_len != 0)
    {
        return ZUN_HOST_INVALID_ARGUMENT;
    }
    // SAFETY: caller supplies a valid immutable byte range for this call.
    let bytes = unsafe { slice::from_raw_parts(module_ptr, module_len) };
    let value = match zun_vm::execute(bytes) {
        Ok(value) => value,
        Err(_) => return ZUN_HOST_VM_ERROR,
    };
    let zun_interp::value::Value::Str(value) = value else {
        return ZUN_HOST_VM_ERROR;
    };
    let value = value.as_bytes();
    // SAFETY: caller supplies writable storage for the required length.
    unsafe { *written = value.len() };
    if out.is_null() {
        return 0;
    }
    if out_len < value.len() {
        return ZUN_HOST_BUFFER_TOO_SMALL;
    }
    if !value.is_empty() {
        // SAFETY: the preceding size check proves the destination is large
        // enough and the caller owns the writable buffer for this call.
        unsafe { std::ptr::copy_nonoverlapping(value.as_ptr(), out, value.len()) };
    }
    ZUN_HOST_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_host_arguments_are_rejected() {
        assert_eq!(
            unsafe {
                zun_host_execute_i64(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    0,
                    std::ptr::null_mut(),
                )
            },
            1
        );
    }

    #[test]
    fn host_executes_a_compiled_integer_module() {
        let module = zun_compiler::compile("function main(): int { return 7; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = 0;
        let status =
            unsafe { zun_host_execute_i64(host, bytes.as_ptr(), bytes.len(), &mut result) };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 0);
        assert_eq!(result, 7);
    }

    #[test]
    fn host_executes_a_compiled_unsigned_integer_module() {
        let module = zun_compiler::compile("function main(): u64 { return 42; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = 0;
        let status = unsafe {
            zun_host_execute_u64(host, bytes.as_ptr(), bytes.len(), &mut result)
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 0);
        assert_eq!(result, 42);
    }

    #[test]
    fn unsigned_host_export_rejects_signed_integer_modules() {
        let module = zun_compiler::compile("function main(): int { return -1; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = 0u64;
        let status = unsafe {
            zun_host_execute_u64(host, bytes.as_ptr(), bytes.len(), &mut result)
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, ZUN_HOST_VM_ERROR);
    }

    #[test]
    fn signed_host_export_rejects_unsigned_integer_modules() {
        let module = zun_compiler::compile("function main(): u64 { return 42; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = 0i64;
        let status = unsafe {
            zun_host_execute_i64(host, bytes.as_ptr(), bytes.len(), &mut result)
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, ZUN_HOST_VM_ERROR);
    }

    #[test]
    fn host_executes_a_compiled_float_module() {
        let module = zun_compiler::compile("function main(): number { return 7.5; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = 0.0;
        let status =
            unsafe { zun_host_execute_f64(host, bytes.as_ptr(), bytes.len(), &mut result) };
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
        let status =
            unsafe { zun_host_execute_bool(host, bytes.as_ptr(), bytes.len(), &mut result) };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 0);
        assert!(result);
    }

    #[test]
    fn host_executes_a_compiled_string_module() {
        let module = zun_compiler::compile("function main(): string { return \"zun\"; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = [0u8; 3];
        let mut written = 0;
        let status = unsafe {
            zun_host_execute_string(
                host,
                bytes.as_ptr(),
                bytes.len(),
                result.as_mut_ptr(),
                result.len(),
                &mut written,
            )
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 0);
        assert_eq!(written, 3);
        assert_eq!(&result, b"zun");
    }

    #[test]
    fn host_string_execution_reports_required_buffer_size() {
        let module = zun_compiler::compile("function main(): string { return \"zun\"; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut result = [0u8; 2];
        let mut written = 0;
        let status = unsafe {
            zun_host_execute_string(
                host,
                bytes.as_ptr(),
                bytes.len(),
                result.as_mut_ptr(),
                result.len(),
                &mut written,
            )
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 3);
        assert_eq!(written, 3);
    }

    #[test]
    fn host_string_execution_supports_size_query_then_copy() {
        let module = zun_compiler::compile("function main(): string { return \"zun\"; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut written = 0;
        let status = unsafe {
            zun_host_execute_string(
                host,
                bytes.as_ptr(),
                bytes.len(),
                std::ptr::null_mut(),
                0,
                &mut written,
            )
        };
        assert_eq!(status, 0);
        assert_eq!(written, 3);
        let mut result = vec![0u8; written];
        let status = unsafe {
            zun_host_execute_string(
                host,
                bytes.as_ptr(),
                bytes.len(),
                result.as_mut_ptr(),
                result.len(),
                &mut written,
            )
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 0);
        assert_eq!(&result, b"zun");
    }

    #[test]
    fn host_string_execution_rejects_null_output_with_nonzero_capacity() {
        let module = zun_compiler::compile("function main(): string { return \"zun\"; }").unwrap();
        let bytes = module.write();
        let host = zun_host_create();
        let mut written = 0;
        let status = unsafe {
            zun_host_execute_string(
                host,
                bytes.as_ptr(),
                bytes.len(),
                std::ptr::null_mut(),
                1,
                &mut written,
            )
        };
        unsafe { zun_host_destroy(host) };
        assert_eq!(status, 1);
        assert_eq!(written, 0);
    }
}
