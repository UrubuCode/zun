//! External native library loading for the future P/Invoke surface.
//!
//! This module is intentionally separate from `zun_natives`: intrinsics are
//! implemented by Zun, while this API loads third-party native libraries.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;

/// Scalar types supported by the first, blittable FFI ABI.
///
/// These types have a stable C representation on the host ABI. Strings,
/// buffers, aggregates, and callbacks require generated marshalling stubs and
/// are deliberately not represented by this descriptor yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfiScalar {
    I64,
    F64,
    Bool,
}

/// How a borrowed native string reports its valid byte length.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfiStringLength {
    /// The native call writes the length to the named out parameter.
    OutParameter(String),
    /// The native string is terminated by a zero byte.
    NullTerminated,
}

/// Declared ownership for a native string returned by a future marshalling
/// stub. The runtime never guesses ownership from the symbol or return type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfiStringReturn {
    /// The native side retains ownership and supplies a length through the
    /// named companion value or a declared NUL-terminated convention.
    Borrowed { length: FfiStringLength },
    /// The native side transfers ownership and exposes the named deallocator.
    Owned { free_symbol: String },
    /// The caller owns the temporary buffer and the native side returns the
    /// number of initialized bytes.
    OutBuffer { capacity: usize },
}

impl FfiStringReturn {
    /// Reject an incomplete ownership declaration before a symbol is loaded.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Borrowed {
                length: FfiStringLength::OutParameter(name),
            } if name.trim().is_empty() => {
                Err("borrowed string returns require a non-empty length parameter".into())
            }
            Self::Owned { free_symbol } if free_symbol.trim().is_empty() => {
                Err("owned string returns require a free symbol".into())
            }
            Self::OutBuffer { capacity: 0 } => {
                Err("out-buffer string returns require positive capacity".into())
            }
            _ => Ok(()),
        }
    }
}

/// A validated signature for a direct C-ABI call. Non-blittable string
/// signatures use [`FfiStringReturn`] as a separate, explicit contract until
/// their generated marshalling stub is available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfiSignature {
    params: Vec<FfiScalar>,
    result: FfiScalar,
}

impl FfiSignature {
    pub fn new(params: impl Into<Vec<FfiScalar>>, result: FfiScalar) -> Self {
        Self {
            params: params.into(),
            result,
        }
    }

    pub fn params(&self) -> &[FfiScalar] {
        &self.params
    }

    pub fn result(&self) -> FfiScalar {
        self.result
    }

    pub fn arity(&self) -> usize {
        self.params.len()
    }

    /// Validate that this signature belongs to the currently implemented
    /// direct-call ABI. Non-blittable shapes must use a generated marshalling
    /// stub instead of reaching the fixed-arity thunks.
    pub fn validate(&self) -> Result<(), String> {
        if self.matches_i64_call() || self.matches_f64_call() || self.matches_bool_call() {
            Ok(())
        } else {
            Err(format!(
                "unsupported direct FFI signature: {} parameter(s) returning {:?}",
                self.arity(),
                self.result
            ))
        }
    }

    fn matches_i64_call(&self) -> bool {
        self.result == FfiScalar::I64
            && self.params.iter().all(|param| *param == FfiScalar::I64)
            && self.arity() <= 3
    }

    fn matches_f64_call(&self) -> bool {
        self.result == FfiScalar::F64
            && self.params.iter().all(|param| *param == FfiScalar::F64)
            && self.arity() <= 2
    }

    fn matches_bool_call(&self) -> bool {
        self.result == FfiScalar::Bool
            && self.params.iter().all(|param| *param == FfiScalar::Bool)
            && self.arity() <= 3
    }
}

/// Return the platform library-name candidates for a logical native library.
///
/// Names that already contain a path separator or a recognized suffix are
/// treated as explicit paths and are not rewritten. This mirrors the runtime
/// loader rule that a caller which supplied a concrete path gets one lookup.
pub fn library_name_variants(name: &str) -> Vec<String> {
    let explicit = name.contains('/') || name.contains('\\');
    let has_suffix = name.ends_with(".dll")
        || name.ends_with(".so")
        || name.contains(".so.")
        || name.ends_with(".dylib");
    if explicit || has_suffix {
        return vec![name.to_owned()];
    }

    #[cfg(windows)]
    {
        vec![format!("{name}.dll"), name.to_owned()]
    }
    #[cfg(target_os = "macos")]
    {
        vec![
            format!("lib{name}.dylib"),
            format!("{name}.dylib"),
            name.to_owned(),
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        vec![
            format!("lib{name}.so"),
            format!("{name}.so"),
            name.to_owned(),
        ]
    }
    #[cfg(not(any(windows, unix)))]
    {
        vec![name.to_owned()]
    }
}

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

/// Owns the native libraries resolved for one VM/host scope.
///
/// The logical name is the cache key, so repeated calls do not reopen the
/// library or repeat platform-name probing. Dropping the resolver releases all
/// handles together with the host scope.
#[derive(Default)]
pub struct NativeLibraryResolver {
    libraries: HashMap<String, NativeLibrary>,
}

impl core::fmt::Debug for NativeLibraryResolver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NativeLibraryResolver")
            .field("libraries", &self.libraries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl NativeLibraryResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve and cache a logical library name, returning the cached handle.
    pub unsafe fn load(&mut self, name: &str) -> Result<&NativeLibrary, FfiError> {
        if !self.libraries.contains_key(name) {
            let library = unsafe { NativeLibrary::open_named(name) }?;
            self.libraries.insert(name.to_owned(), library);
        }
        Ok(self
            .libraries
            .get(name)
            .expect("library inserted or cached"))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.libraries.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.libraries.len()
    }
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

    /// Load an explicit native-library path after requiring it to be absolute.
    ///
    /// Keeping this check separate from [`NativeLibrary::open`] preserves the
    /// low-level unsafe API while giving language/runtime callers a safe policy
    /// boundary against relative-path DLL planting.
    pub unsafe fn open_path(path: impl AsRef<Path>) -> Result<Self, FfiError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(FfiError {
                message: format!("native library path must be absolute: {}", path.display()),
            });
        }
        // SAFETY: the caller accepts loading the explicitly selected path.
        unsafe { Self::open(path.as_os_str()) }
    }

    /// Load a logical library name using platform filename variants.
    ///
    /// Explicit paths are passed through unchanged. For logical names, each
    /// candidate is tried in order and all failures are retained in the final
    /// error so a missing native dependency is diagnosable.
    pub unsafe fn open_named(name: &str) -> Result<Self, FfiError> {
        let mut errors = Vec::new();
        for candidate in library_name_variants(name) {
            match unsafe { Self::open(&candidate) } {
                Ok(library) => return Ok(library),
                Err(error) => errors.push(error.to_string()),
            }
        }
        Err(FfiError {
            message: format!(
                "failed to resolve native library `{name}`; attempts: {}",
                errors.join("; ")
            ),
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

    /// Call a C symbol with three `i64` arguments and an `i64` result.
    ///
    /// # Safety
    /// The symbol must use the C ABI and have the exact signature
    /// `extern "C" fn(i64, i64, i64) -> i64`.
    pub unsafe fn call_i64_3(
        &self,
        name: &[u8],
        first: i64,
        second: i64,
        third: i64,
    ) -> Result<i64, FfiError> {
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn(i64, i64, i64) -> i64>(name) }?;
        Ok(unsafe { symbol(first, second, third) })
    }

    /// Call a blittable `i64` function after validating its explicit ABI
    /// descriptor. This is the first typed bridge between signature metadata
    /// and the existing fixed-arity native thunks.
    pub unsafe fn call_i64_signature(
        &self,
        name: &[u8],
        signature: &FfiSignature,
        args: &[i64],
    ) -> Result<i64, FfiError> {
        if !signature.matches_i64_call() || signature.arity() != args.len() {
            return Err(FfiError {
                message: format!(
                    "unsupported i64 FFI signature: arity {}, arguments {}",
                    signature.arity(),
                    args.len()
                ),
            });
        }
        // SAFETY: the descriptor restricted the call to the matching fixed
        // C-ABI shape and the arity dispatch selects the corresponding thunk.
        unsafe {
            match args {
                [] => self.call_i64_0(name),
                [arg] => self.call_i64_1(name, *arg),
                [first, second] => self.call_i64_2(name, *first, *second),
                [first, second, third] => self.call_i64_3(name, *first, *second, *third),
                _ => unreachable!("signature validation limits i64 calls to three arguments"),
            }
        }
    }

    /// Call a C symbol with no arguments and a C integer-style boolean result.
    ///
    /// # Safety
    /// The symbol must use the C ABI and have the exact signature
    /// `extern "C" fn() -> i32`.
    pub unsafe fn call_bool_0(&self, name: &[u8]) -> Result<bool, FfiError> {
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn() -> i32>(name) }?;
        Ok(unsafe { symbol() } != 0)
    }

    /// Call a C integer-style boolean function with one boolean argument.
    /// The explicit `i32` ABI matches the C representation declared by this
    /// bridge; it is not inferred from Rust's platform-dependent `bool` ABI.
    pub unsafe fn call_bool_1(&self, name: &[u8], arg: bool) -> Result<bool, FfiError> {
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn(i32) -> i32>(name) }?;
        Ok(unsafe { symbol(arg as i32) } != 0)
    }

    /// Call a C integer-style boolean function with two boolean arguments.
    pub unsafe fn call_bool_2(
        &self,
        name: &[u8],
        first: bool,
        second: bool,
    ) -> Result<bool, FfiError> {
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn(i32, i32) -> i32>(name) }?;
        Ok(unsafe { symbol(first as i32, second as i32) } != 0)
    }

    /// Call a C integer-style boolean function with three boolean arguments.
    pub unsafe fn call_bool_3(
        &self,
        name: &[u8],
        first: bool,
        second: bool,
        third: bool,
    ) -> Result<bool, FfiError> {
        let symbol = unsafe { self.symbol::<unsafe extern "C" fn(i32, i32, i32) -> i32>(name) }?;
        Ok(unsafe { symbol(first as i32, second as i32, third as i32) } != 0)
    }

    /// Call a blittable C integer-style boolean after validating its ABI
    /// descriptor. Boolean parameters use the explicit integer-style C ABI.
    pub unsafe fn call_bool_signature(
        &self,
        name: &[u8],
        signature: &FfiSignature,
        args: &[bool],
    ) -> Result<bool, FfiError> {
        if !signature.matches_bool_call() || signature.arity() != args.len() {
            return Err(FfiError {
                message: "unsupported bool FFI signature: expected (bool{0,3}) -> bool".into(),
            });
        }
        // SAFETY: the descriptor restricts every parameter and the arity
        // dispatch selects the matching explicit C integer-style thunk.
        unsafe {
            match args {
                [] => self.call_bool_0(name),
                [arg] => self.call_bool_1(name, *arg),
                [first, second] => self.call_bool_2(name, *first, *second),
                [first, second, third] => self.call_bool_3(name, *first, *second, *third),
                _ => unreachable!("signature validation limits bool calls to three arguments"),
            }
        }
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

    /// Call a blittable `f64` function after validating its ABI descriptor.
    pub unsafe fn call_f64_signature(
        &self,
        name: &[u8],
        signature: &FfiSignature,
        args: &[f64],
    ) -> Result<f64, FfiError> {
        if !signature.matches_f64_call() || signature.arity() != args.len() {
            return Err(FfiError {
                message: format!(
                    "unsupported f64 FFI signature: arity {}, arguments {}",
                    signature.arity(),
                    args.len()
                ),
            });
        }
        // SAFETY: the descriptor restricts the call to the matching fixed C
        // ABI shape selected below.
        unsafe {
            match args {
                [] => {
                    let symbol = self.symbol::<unsafe extern "C" fn() -> f64>(name)?;
                    Ok(symbol())
                }
                [arg] => self.call_f64_1(name, *arg),
                [first, second] => self.call_f64_2(name, *first, *second),
                _ => unreachable!("signature validation limits f64 calls to two arguments"),
            }
        }
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
    fn explicit_library_names_are_not_rewritten() {
        assert_eq!(
            library_name_variants("./native/libzun.so"),
            vec!["./native/libzun.so"]
        );
        assert_eq!(library_name_variants("zun.so.1"), vec!["zun.so.1"]);
    }

    #[test]
    fn explicit_loader_rejects_relative_paths() {
        let result = unsafe { NativeLibrary::open_path("./native/libzun.so") };
        let error = result.expect_err("relative native path must be rejected");
        assert!(error.to_string().contains("must be absolute"));
    }

    #[test]
    fn logical_library_names_include_platform_candidates() {
        let variants = library_name_variants("zun");
        assert!(!variants.is_empty());
        assert!(variants.iter().any(|candidate| candidate == "zun"));
        #[cfg(windows)]
        assert!(variants.contains(&"zun.dll".to_owned()));
        #[cfg(unix)]
        assert!(variants
            .iter()
            .any(|candidate| candidate.starts_with("libzun")));
    }

    #[test]
    fn named_load_reports_all_candidates() {
        let result = unsafe { NativeLibrary::open_named("zun-library-that-does-not-exist") };
        let error = result.expect_err("missing logical library must fail");
        assert!(error.to_string().contains("attempts:"));
    }

    #[test]
    fn resolver_does_not_cache_failed_loads() {
        let mut resolver = NativeLibraryResolver::new();
        let result = unsafe { resolver.load("zun-library-that-does-not-exist") };
        assert!(result.is_err());
        assert_eq!(resolver.len(), 0);
        assert!(!resolver.contains("zun-library-that-does-not-exist"));
    }

    #[test]
    fn ffi_signature_preserves_the_blittable_call_shape() {
        let signature = FfiSignature::new(
            vec![FfiScalar::I64, FfiScalar::F64, FfiScalar::Bool],
            FfiScalar::I64,
        );
        assert_eq!(signature.arity(), 3);
        assert_eq!(
            signature.params(),
            &[FfiScalar::I64, FfiScalar::F64, FfiScalar::Bool]
        );
        assert_eq!(signature.result(), FfiScalar::I64);
    }

    #[test]
    fn string_return_ownership_requires_an_explicit_safe_mode() {
        assert!(FfiStringReturn::Borrowed {
            length: FfiStringLength::OutParameter("out_len".into())
        }
        .validate()
        .is_ok());
        assert!(FfiStringReturn::Borrowed {
            length: FfiStringLength::NullTerminated
        }
        .validate()
        .is_ok());
        assert!(FfiStringReturn::Owned {
            free_symbol: "free_text".into()
        }
        .validate()
        .is_ok());
        assert!(FfiStringReturn::OutBuffer { capacity: 256 }
            .validate()
            .is_ok());
        assert!(FfiStringReturn::Borrowed {
            length: FfiStringLength::OutParameter(String::new())
        }
        .validate()
        .is_err());
        assert!(FfiStringReturn::Owned {
            free_symbol: "  ".into()
        }
        .validate()
        .is_err());
        assert!(FfiStringReturn::OutBuffer { capacity: 0 }
            .validate()
            .is_err());
    }

    #[test]
    fn bool_parameters_use_the_explicit_integer_style_abi() {
        let signature = FfiSignature::new(
            vec![FfiScalar::Bool, FfiScalar::Bool, FfiScalar::Bool],
            FfiScalar::Bool,
        );
        assert!(signature.validate().is_ok());
        assert_eq!(signature.params(), &[FfiScalar::Bool; 3]);
    }

    #[test]
    fn bool_abi_rejects_mixed_types_and_more_than_three_parameters() {
        assert!(
            FfiSignature::new(vec![FfiScalar::Bool, FfiScalar::I64], FfiScalar::Bool,)
                .validate()
                .is_err()
        );
        assert!(
            FfiSignature::new(vec![FfiScalar::Bool; 4], FfiScalar::Bool,)
                .validate()
                .is_err()
        );
    }

    #[test]
    fn typed_i64_call_rejects_an_incompatible_signature_before_lookup() {
        let path = std::env::current_exe().expect("test executable path");
        let library = unsafe { NativeLibrary::open(path) }.expect("load test executable");
        let signature = FfiSignature::new(vec![FfiScalar::F64], FfiScalar::I64);
        let error = unsafe { library.call_i64_signature(b"zun_missing_symbol", &signature, &[1]) }
            .expect_err("incompatible signature must be rejected");
        assert!(error.to_string().contains("unsupported i64 FFI signature"));
    }

    #[test]
    fn typed_f64_call_rejects_an_incompatible_signature_before_lookup() {
        let path = std::env::current_exe().expect("test executable path");
        let library = unsafe { NativeLibrary::open(path) }.expect("load test executable");
        let signature = FfiSignature::new(vec![FfiScalar::I64], FfiScalar::F64);
        let error =
            unsafe { library.call_f64_signature(b"zun_missing_symbol", &signature, &[1.0]) }
                .expect_err("incompatible signature must be rejected");
        assert!(error.to_string().contains("unsupported f64 FFI signature"));
    }

    #[test]
    fn typed_bool_call_rejects_an_incompatible_signature_before_lookup() {
        let path = std::env::current_exe().expect("test executable path");
        let library = unsafe { NativeLibrary::open(path) }.expect("load test executable");
        let signature = FfiSignature::new(vec![FfiScalar::I64], FfiScalar::Bool);
        let error =
            unsafe { library.call_bool_signature(b"zun_missing_symbol", &signature, &[false]) }
                .expect_err("incompatible signature must be rejected");
        assert!(error.to_string().contains("unsupported bool FFI signature"));
    }

    #[cfg(windows)]
    #[test]
    fn typed_i64_signature_calls_a_real_system_symbol() {
        let library = unsafe { NativeLibrary::open("kernel32.dll") }.expect("load kernel32");
        let signature = FfiSignature::new(Vec::new(), FfiScalar::I64);
        let process_id = unsafe {
            library
                .call_i64_signature(b"GetCurrentProcessId", &signature, &[])
                .expect("resolve GetCurrentProcessId through typed ABI")
        };
        assert!(process_id > 0);
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
        let result = unsafe { library.call_i64_3(b"zun_missing_symbol", 1, 2, 3) };
        assert!(result.is_err());
        let result = unsafe { library.call_f64_0(b"zun_missing_symbol") };
        assert!(result.is_err());
        let result = unsafe { library.call_f64_2(b"zun_missing_symbol", 1.0, 2.0) };
        assert!(result.is_err());
        let result = unsafe { library.call_bool_0(b"zun_missing_symbol") };
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

    #[cfg(windows)]
    #[test]
    fn resolver_reuses_a_successfully_loaded_library() {
        let mut resolver = NativeLibraryResolver::new();
        let first_path = {
            let first = unsafe { resolver.load("kernel32") }.expect("load kernel32");
            first.path().to_owned()
        };
        assert!(first_path.ends_with("kernel32.dll"));
        assert_eq!(resolver.len(), 1);
        let second = unsafe { resolver.load("kernel32") }.expect("reuse kernel32");
        assert_eq!(first_path, second.path());
        assert_eq!(resolver.len(), 1);
    }
}
