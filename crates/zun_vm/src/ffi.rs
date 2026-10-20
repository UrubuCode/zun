//! External native library loading for the future P/Invoke surface.
//!
//! This module is intentionally separate from `zun_natives`: intrinsics are
//! implemented by Zun, while this API loads third-party native libraries.

use std::ffi::OsStr;

/// Errors raised while opening a native library or resolving a symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiError {
    message: String,
}

impl core::fmt::Display for FfiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FfiError {}

/// A loaded external native library.
pub struct NativeLibrary {
    library: libloading::Library,
    path: String,
}

impl core::fmt::Debug for NativeLibrary {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NativeLibrary")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl NativeLibrary {
    /// Load a dynamic library from `path`.
    ///
    /// # Safety
    /// Loading arbitrary native code executes platform loader behavior and
    /// must only be allowed for trusted deployment inputs.
    pub unsafe fn open(path: impl AsRef<OsStr>) -> Result<Self, FfiError> {
        let path_ref = path.as_ref();
        // SAFETY: the caller accepts the platform loader side effects.
        let library = unsafe { libloading::Library::new(path_ref) }.map_err(|error| FfiError {
            message: format!("failed to load native library: {error}"),
        })?;
        Ok(Self {
            library,
            path: path_ref.to_string_lossy().into_owned(),
        })
    }

    /// Resolve a typed symbol from the loaded library.
    ///
    /// # Safety
    /// `T` must exactly match the symbol's ABI, calling convention, argument
    /// types, and return type. The returned symbol must not outlive this
    /// library.
    pub unsafe fn symbol<T>(&self, name: &[u8]) -> Result<libloading::Symbol<'_, T>, FfiError> {
        // SAFETY: the caller guarantees the symbol type and lifetime contract.
        unsafe { self.library.get(name) }.map_err(|error| FfiError {
            message: format!("failed to resolve native symbol: {error}"),
        })
    }

    /// Return the path used to load this library.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Call a C symbol with no arguments and an `i64` result.
    ///
    /// # Safety
    /// The symbol must use the C ABI and have the exact signature
    /// `extern "C" fn() -> i64`.
    pub unsafe fn call_i64_0(&self, name: &[u8]) -> Result<i64, FfiError> {
        // SAFETY: the caller guarantees the symbol ABI described above.
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn() -> i64>(name) }?;
        // SAFETY: the resolved function has the declared ABI and signature.
        Ok(unsafe { symbol() })
    }

    /// Call a C symbol with one `i64` argument and an `i64` result.
    ///
    /// # Safety
    /// The symbol must use the C ABI and have the exact signature
    /// `extern "C" fn(i64) -> i64`.
    pub unsafe fn call_i64_1(&self, name: &[u8], arg: i64) -> Result<i64, FfiError> {
        // SAFETY: the caller guarantees the symbol ABI described above.
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn(i64) -> i64>(name) }?;
        // SAFETY: the resolved function has the declared ABI and signature.
        Ok(unsafe { symbol(arg) })
    }

    /// Call a C symbol with two `i64` arguments and an `i64` result.
    ///
    /// # Safety
    /// The symbol must use the C ABI and have the exact signature
    /// `extern "C" fn(i64, i64) -> i64`.
    pub unsafe fn call_i64_2(&self, name: &[u8], first: i64, second: i64) -> Result<i64, FfiError> {
        // SAFETY: the caller guarantees the symbol ABI described above.
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn(i64, i64) -> i64>(name) }?;
        // SAFETY: the resolved function has the declared ABI and signature.
        Ok(unsafe { symbol(first, second) })
    }

    /// Call a C symbol with no arguments and an `f64` result.
    ///
    /// # Safety
    /// The symbol must use the C ABI and have the exact signature
    /// `extern "C" fn() -> f64`.
    pub unsafe fn call_f64_0(&self, name: &[u8]) -> Result<f64, FfiError> {
        // SAFETY: the caller guarantees the symbol ABI described above.
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn() -> f64>(name) }?;
        // SAFETY: the resolved function has the declared ABI and signature.
        Ok(unsafe { symbol() })
    }

    /// Call a C symbol with one `f64` argument and an `f64` result.
    ///
    /// # Safety
    /// The symbol must use the C ABI and have the exact signature
    /// `extern "C" fn(f64) -> f64`.
    pub unsafe fn call_f64_1(&self, name: &[u8], arg: f64) -> Result<f64, FfiError> {
        // SAFETY: the caller guarantees the symbol ABI described above.
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn(f64) -> f64>(name) }?;
        // SAFETY: the resolved function has the declared ABI and signature.
        Ok(unsafe { symbol(arg) })
    }

    /// Call a C symbol with two `f64` arguments and an `f64` result.
    ///
    /// # Safety
    /// The symbol must use the C ABI and have the exact signature
    /// `extern "C" fn(f64, f64) -> f64`.
    pub unsafe fn call_f64_2(&self, name: &[u8], first: f64, second: f64) -> Result<f64, FfiError> {
        // SAFETY: the caller guarantees the symbol ABI described above.
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn(f64, f64) -> f64>(name) }?;
        // SAFETY: the resolved function has the declared ABI and signature.
        Ok(unsafe { symbol(first, second) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_returns_a_typed_error() {
        let result = unsafe { NativeLibrary::open("zun-library-that-does-not-exist") };
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("failed to load"));
    }

    #[test]
    fn missing_typed_symbol_returns_a_typed_error() {
        let path = std::env::current_exe().expect("test executable path");
        let library = unsafe { NativeLibrary::open(path) }.expect("load test executable");
        let result = unsafe { library.call_i64_1(b"zun_missing_symbol", 1) };
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("failed to resolve"));
    }

    #[test]
    fn every_scalar_signature_reports_missing_symbols_consistently() {
        let path = std::env::current_exe().expect("test executable path");
        let library = unsafe { NativeLibrary::open(path) }.expect("load test executable");
        let result = unsafe { library.call_i64_2(b"zun_missing_symbol", 1, 2) };
        assert!(result.is_err());
        let result = unsafe { library.call_f64_0(b"zun_missing_symbol") };
        assert!(result.is_err());
        let result = unsafe { library.call_f64_2(b"zun_missing_symbol", 1.0, 2.0) };
        assert!(result.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn calls_a_real_system_library_symbol() {
        let library = unsafe { NativeLibrary::open("kernel32.dll") }.expect("load kernel32");
        let process_id = unsafe {
            library
                .call_i64_0(b"GetCurrentProcessId")
                .expect("resolve GetCurrentProcessId")
        };
        assert!(process_id > 0);
    }
}
