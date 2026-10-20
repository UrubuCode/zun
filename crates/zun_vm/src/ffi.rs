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
}
