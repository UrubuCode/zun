//! Module loading and scope identity for Zun binaries.
//!
//! Loading is deliberately separate from verification and execution. A
//! [`ModuleScope`] owns the bytes decoded from one `.zm` module and gives the
//! future resolver a stable scope boundary for type and member identity.

use std::path::Path;
use std::collections::HashMap;
use zun_module::{Module, ReadError};
use zun_metadata::{TableKind, Token};
use zun_types::{TypeError, TypeSystem};

/// A loaded module together with its loader-visible identity.
#[derive(Debug)]
pub struct ModuleScope {
    name: String,
    module: Module,
    types: TypeSystem,
}

impl ModuleScope {
    /// Load a module from serialized `.zm` bytes.
    pub fn from_bytes(name: impl Into<String>, bytes: &[u8]) -> Result<Self, LoadError> {
        let module = Module::read(bytes).map_err(LoadError::Format)?;
        let types = TypeSystem::from_module(&module).map_err(LoadError::Types)?;
        Ok(Self {
            name: name.into(),
            module,
            types,
        })
    }

    /// Load a module from a file path, preserving the path as its identity.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(LoadError::Io)?;
        Self::from_bytes(path.to_string_lossy(), &bytes)
    }

    /// The loader identity of this module.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Access the decoded module for verification or execution.
    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Access the reified runtime types built for this module.
    pub fn types(&self) -> &TypeSystem {
        &self.types
    }

    /// Consume the scope and return its decoded module.
    pub fn into_module(self) -> Module {
        self.module
    }
}

/// Errors while loading a module from disk.
#[derive(Debug)]
pub enum LoadError {
    /// The operating system rejected the file operation.
    Io(std::io::Error),
    /// The bytes are not a valid `.zm` container.
    Format(ReadError),
    /// The metadata cannot be materialized as runtime types.
    Types(TypeError),
    /// A module reference token is invalid or unresolved.
    UnresolvedModule(String),
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "module I/O failed: {error}"),
            Self::Format(error) => write!(f, "invalid module: {error}"),
            Self::Types(error) => write!(f, "invalid type metadata: {error}"),
            Self::UnresolvedModule(name) => write!(f, "unresolved module reference: {name}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Registry of loaded module scopes, keyed by their manifest identity.
#[derive(Debug, Default)]
pub struct ModuleLoader {
    modules: HashMap<String, ModuleScope>,
}

impl ModuleLoader {
    /// Create an empty module loader.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load and register a module under `name`.
    pub fn load_bytes(&mut self, name: impl Into<String>, bytes: &[u8]) -> Result<(), LoadError> {
        let name = name.into();
        let scope = ModuleScope::from_bytes(name.clone(), bytes)?;
        self.modules.insert(name, scope);
        Ok(())
    }

    /// Load and register a module using its path as identity.
    pub fn load_file(&mut self, path: impl AsRef<Path>) -> Result<(), LoadError> {
        let scope = ModuleScope::from_file(path)?;
        let name = scope.name().to_owned();
        self.modules.insert(name, scope);
        Ok(())
    }

    /// Resolve a previously loaded module by exact identity.
    pub fn get(&self, name: &str) -> Option<&ModuleScope> {
        self.modules.get(name)
    }

    /// Resolve a ModuleRef token using the requesting module's string heap.
    pub fn resolve_module_ref<'a>(
        &'a self,
        requester: &ModuleScope,
        token: Token,
    ) -> Result<&'a ModuleScope, LoadError> {
        if token.table() != Some(TableKind::ModuleRef) {
            return Err(LoadError::UnresolvedModule(format!("invalid token 0x{:08x}", token.to_raw())));
        }
        let module_ref = requester
            .module
            .tables
            .module_refs
            .get(token.rid() as usize - 1)
            .ok_or_else(|| LoadError::UnresolvedModule(format!("RID {}", token.rid())))?;
        let name = requester.module.strings.get(module_ref.name);
        self.get(name)
            .ok_or_else(|| LoadError::UnresolvedModule(name.to_string()))
    }

    /// Number of registered modules.
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    /// Whether no modules have been registered.
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_and_preserves_scope_identity() {
        let module = Module::new();
        let scope = ModuleScope::from_bytes("demo", &module.write()).expect("load");
        assert_eq!(scope.name(), "demo");
    }

    #[test]
    fn registry_resolves_multiple_module_scopes() {
        let bytes = Module::new().write();
        let mut loader = ModuleLoader::new();
        loader.load_bytes("core", &bytes).expect("load core");
        loader.load_bytes("app", &bytes).expect("load app");
        assert_eq!(loader.len(), 2);
        assert_eq!(loader.get("app").unwrap().name(), "app");
    }
}
