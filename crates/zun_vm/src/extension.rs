//! Loading and registration of trusted Zun native extensions.

use std::collections::HashMap;
use std::path::Path;

use zun_abi::{OwnedFunction, OwnedRegistry, ZunRegisterV1, ZunRegistry, ZunValue};

use crate::{FfiError, NativeLibrary};

/// A loaded extension and the owned descriptors it published.
///
/// The library is kept alive for as long as its function pointers are exposed
/// through [`functions`]. Dropping this value releases the native library;
/// callers must stop invoking its functions before that happens.
pub struct LoadedExtension {
    library: NativeLibrary,
    registry: OwnedRegistry,
}

/// Owns loaded extensions for one VM/host scope and caches them by logical
/// name. A failed load is not cached, so a later retry can observe a repaired
/// deployment.
#[derive(Default)]
pub struct ExtensionResolver {
    extensions: HashMap<String, LoadedExtension>,
}

impl core::fmt::Debug for ExtensionResolver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ExtensionResolver")
            .field("extensions", &self.extensions.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ExtensionResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve and cache a logical extension name.
    ///
    /// # Safety
    /// The resolved library must be trusted native code implementing the v1
    /// registration contract.
    pub unsafe fn load_named(&mut self, name: &str) -> Result<&LoadedExtension, FfiError> {
        if !self.extensions.contains_key(name) {
            // SAFETY: forwarded from the caller's trusted-extension contract.
            let extension = unsafe { LoadedExtension::load_named(name) }?;
            self.extensions.insert(name.to_owned(), extension);
        }
        Ok(self
            .extensions
            .get(name)
            .expect("extension inserted or cached"))
    }

    /// Materialize and register a `.zmlib`'s embedded native extension for
    /// `triple` (the host triple in production). Returns `Ok(false)` when the
    /// package has no object for that triple. The library handle is stored in
    /// this resolver, keeping its mapping valid for the resolver's lifetime;
    /// the cache file is content-hash named, so a repeat load reuses it.
    ///
    /// # Safety
    /// The package lock must already be verified against trusted keys; the
    /// embedded library is then trusted native code implementing the v1
    /// registration contract and runs during registration.
    pub unsafe fn load_zmlib_native(
        &mut self,
        package: &zun_loader::Zmlib,
        cache_root: impl AsRef<Path>,
        triple: &str,
    ) -> Result<bool, FfiError> {
        let path = package
            .materialize_native_object(cache_root, triple)
            .map_err(|error| {
                FfiError::new(format!("native object materialization failed: {error}"))
            })?;
        let Some(path) = path else {
            return Ok(false);
        };
        let path = std::path::absolute(&path).map_err(|error| {
            FfiError::new(format!("native object path is unusable: {error}"))
        })?;
        let key = path.to_string_lossy().into_owned();
        if !self.extensions.contains_key(&key) {
            // SAFETY: forwarded from the caller's trusted-extension contract.
            let extension = unsafe { LoadedExtension::load_path(&path) }?;
            self.extensions.insert(key, extension);
        }
        Ok(true)
    }

    /// Look up a registered function across all stored extensions.
    pub fn find_function(&self, module: &str, key: &str) -> Option<&OwnedFunction> {
        self.extensions
            .values()
            .find_map(|extension| extension.find_function(module, key))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.extensions.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.extensions.len()
    }
}

impl core::fmt::Debug for LoadedExtension {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LoadedExtension")
            .field("path", &self.library.path())
            .field("function_count", &self.registry.functions.len())
            .finish()
    }
}

impl LoadedExtension {
    /// Load an extension from an explicit absolute path and invoke its v1
    /// registration entry point.
    ///
    /// # Safety
    /// The path must identify trusted native code. The extension runs during
    /// registration and its exported function pointers remain valid only
    /// while the returned value is alive.
    pub unsafe fn load_path(path: impl AsRef<Path>) -> Result<Self, FfiError> {
        // SAFETY: the caller accepts native loader side effects and trusted
        // extension execution.
        let library = unsafe { NativeLibrary::open_path(path) }?;
        // SAFETY: the extension contract fixes the symbol ABI and the symbol
        // is used only while `library` remains alive.
        unsafe { Self::from_library(library) }
    }

    /// Load an extension by its logical platform-independent library name.
    ///
    /// # Safety
    /// The resolved library must be trusted native code implementing the v1
    /// registration contract.
    pub unsafe fn load_named(name: &str) -> Result<Self, FfiError> {
        // SAFETY: the caller accepts native loader side effects and trusted
        // extension execution.
        let library = unsafe { NativeLibrary::open_named(name) }?;
        // SAFETY: the extension contract fixes the symbol ABI.
        unsafe { Self::from_library(library) }
    }

    unsafe fn from_library(library: NativeLibrary) -> Result<Self, FfiError> {
        let mut registry = OwnedRegistry::new();
        let mut host_registry: ZunRegistry = registry.as_registry();
        let symbol_name = b"zun_register_v1\0";
        // SAFETY: the extension ABI requires this exact symbol type and the
        // library is stored in the returned object for its full lifetime.
        let register = unsafe { library.symbol::<ZunRegisterV1>(symbol_name) }?;
        // SAFETY: `host_registry` points at `registry`, which stays alive until
        // this call returns; the extension contract requires a valid v1 table.
        let status = unsafe { register(&mut host_registry) };
        if status != 0 {
            return Err(FfiError::new(format!(
                "extension registration failed with status {status}"
            )));
        }
        drop(register);
        Ok(Self { library, registry })
    }

    pub fn path(&self) -> &str {
        self.library.path()
    }

    pub fn functions(&self) -> &[OwnedFunction] {
        &self.registry.functions
    }

    /// Resolve a registered native function by its stable module/key pair.
    pub fn find_function(&self, module: &str, key: &str) -> Option<&OwnedFunction> {
        self.registry.find(module, key)
    }

    /// Invoke a registered extension function through its checked ABI shape.
    ///
    /// The extension library remains owned by `self` for the duration of the
    /// call, and descriptor mismatches are reported before native code runs.
    pub fn invoke(
        &self,
        module: &str,
        key: &str,
        args: &[ZunValue],
    ) -> Result<Option<ZunValue>, FfiError> {
        let function = self.find_function(module, key).ok_or_else(|| {
            FfiError::new(format!(
                "extension function `{module}.{key}` is not registered"
            ))
        })?;
        function.invoke_checked(args).map_err(|status| {
            FfiError::new(format!(
                "extension function `{module}.{key}` failed with status {status}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_loader_rejects_relative_paths_before_loading() {
        let result = unsafe { LoadedExtension::load_path("relative-extension.dll") };
        let error = result.expect_err("relative extension paths must be rejected");
        assert!(error.to_string().contains("absolute"));
    }

    #[test]
    fn extension_resolver_does_not_cache_failed_loads() {
        let mut resolver = ExtensionResolver::new();
        let result = unsafe { resolver.load_named("zun-extension-that-does-not-exist") };
        assert!(result.is_err());
        assert!(!resolver.contains("zun-extension-that-does-not-exist"));
        assert_eq!(resolver.len(), 0);
    }
}

/// The host target triple (compile-time), matching the keys of a `.zmlib`'s
/// embedded native objects.
pub fn host_triple() -> String {
    target_lexicon::Triple::host().to_string()
}
