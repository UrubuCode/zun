//! Module loading and scope identity for Zun binaries.
//!
//! Loading is deliberately separate from verification and execution. A
//! [`ModuleScope`] owns the bytes decoded from one `.zm` module and gives the
//! future resolver a stable scope boundary for type and member identity.

use zun_module::ReadError;
use zun_types::TypeError;

mod bytes;
mod content;
mod fetch;
mod index;
mod loader;
mod manifest;
mod scope;
mod trust;
mod zmlib;

pub use content::{content_id, CacheError, ContentId, ModuleCache};
pub use fetch::{DirectoryFetcher, HttpFetcher, ModuleFetcher};
pub use index::{ModuleIndex, SignedModuleIndex};
pub use loader::ModuleLoader;
pub use manifest::{ModuleLock, ModuleManifest, SignedModuleLock};
pub use scope::{Export, MemberKind, ModuleScope, ResolvedMember};
pub use trust::TrustedKeySet;
pub use zmlib::Zmlib;

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
    /// A module identity was registered more than once.
    DuplicateModule(String),
    /// A manifest contains an invalid dependency ordering or duplicate.
    InvalidManifest(String),
    /// Two definitions in one module expose the same source-level name.
    DuplicateExport(String),
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "module I/O failed: {error}"),
            Self::Format(error) => write!(f, "invalid module: {error}"),
            Self::Types(error) => write!(f, "invalid type metadata: {error}"),
            Self::UnresolvedModule(name) => write!(f, "unresolved module reference: {name}"),
            Self::DuplicateModule(name) => write!(f, "duplicate module identity: {name}"),
            Self::InvalidManifest(detail) => write!(f, "invalid module manifest: {detail}"),
            Self::DuplicateExport(name) => write!(f, "duplicate module export: {name}"),
        }
    }
}

impl std::error::Error for LoadError {}

#[cfg(test)]
mod tests;

