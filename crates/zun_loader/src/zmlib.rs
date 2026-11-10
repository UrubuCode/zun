use std::path::{Path, PathBuf};

use crate::bytes::{read_name, read_u32, take, write_name};
use crate::{content_id, ContentId, LoadError, SignedModuleLock};

/// A deterministic `.zmlib` package containing signed module metadata and
/// serialized `.zm` payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zmlib {
    /// Authenticated package lockfile.
    pub lock: SignedModuleLock,
    /// Named serialized modules included in the package.
    pub modules: Vec<(String, Vec<u8>)>,
    /// Optional `.zmlib` type declaration payloads keyed by module identity.
    /// Debug information remains in each serialized `.zm` module.
    pub type_declarations: Vec<(String, Vec<u8>)>,
    /// Embedded native extension library bytes keyed by target triple
    /// (e.g. `x86_64-pc-windows-msvc`). Each payload is authenticated by the
    /// lock entry `<triple>.nativeobj`. A pure-ZIL package carries none.
    pub native_objects: Vec<(String, Vec<u8>)>,
    /// Engine core contract version this package was built against
    /// (`zun_module::CORE_CONTRACT_VERSION`); `0` means unspecified (a
    /// pre-v4 package). Compatibility rule: exact match, or `0`.
    ///
    /// Note: the same versioning concern applies to cross-program type
    /// identity (serde #27 follow-up); that is not implemented here.
    pub required_core_version: u32,
}

impl Zmlib {
    const MAGIC: &'static [u8; 5] = b"ZMLIB";
    const VERSION: u16 = 4;

    /// Check this package against an engine core contract version. `0`
    /// (legacy/unspecified) and an exact match are accepted.
    pub fn check_core_contract(&self, engine: u32) -> Result<(), LoadError> {
        if self.required_core_version == 0 || self.required_core_version == engine {
            Ok(())
        } else {
            Err(LoadError::InvalidManifest(format!(
                ".zmlib requires core contract v{}, engine provides v{engine}",
                self.required_core_version
            )))
        }
    }

    /// Construct a package and ensure every payload matches its lock entry.
    pub fn new(lock: SignedModuleLock, modules: Vec<(String, Vec<u8>)>) -> Result<Self, LoadError> {
        lock.verify_signature()?;
        {
            let lock_entries: std::collections::HashMap<&str, ContentId> = lock
                .lock
                .entries
                .iter()
                .map(|(name, id)| (name.as_str(), *id))
                .collect();
            for (name, bytes) in &modules {
                let expected = lock_entries.get(name.as_str()).copied().ok_or_else(|| {
                    LoadError::InvalidManifest(format!("package module `{name}` is not locked"))
                })?;
                let found = content_id(bytes);
                if expected != found {
                    return Err(LoadError::InvalidManifest(format!(
                        "package module `{name}` has content identity mismatch"
                    )));
                }
            }
        }
        Ok(Self {
            lock,
            modules,
            type_declarations: Vec::new(),
            native_objects: Vec::new(),
            required_core_version: zun_module::CORE_CONTRACT_VERSION,
        })
    }

    /// Attach native extension library bytes keyed by target triple. Each
    /// payload must match the lock entry `<triple>.nativeobj` by content hash.
    pub fn with_native_objects(
        mut self,
        objects: Vec<(String, Vec<u8>)>,
    ) -> Result<Self, LoadError> {
        let mut seen = std::collections::HashSet::new();
        for (triple, payload) in &objects {
            if triple.is_empty() || !seen.insert(triple) {
                return Err(LoadError::InvalidManifest(format!(
                    "invalid or duplicate native object triple `{triple}`"
                )));
            }
            let identity_name = format!("{triple}.nativeobj");
            let expected = self
                .lock
                .lock
                .entries
                .iter()
                .find(|(entry, _)| entry == &identity_name)
                .map(|(_, id)| *id)
                .ok_or_else(|| {
                    LoadError::InvalidManifest(format!(
                        "native object `{identity_name}` is not locked"
                    ))
                })?;
            if expected != content_id(payload) {
                return Err(LoadError::InvalidManifest(format!(
                    "native object `{identity_name}` has content identity mismatch"
                )));
            }
        }
        self.native_objects = objects;
        Ok(self)
    }

    /// Native library bytes for an exact target triple, if embedded. A loader
    /// writes these to a file, `libloading`s it, and registers its `zun_abi`
    /// exports.
    pub fn native_object_for(&self, triple: &str) -> Option<&[u8]> {
        self.native_objects
            .iter()
            .find(|(t, _)| t == triple)
            .map(|(_, bytes)| bytes.as_slice())
    }

    /// Materialize the native object for `triple` into
    /// `<cache_root>/native/<content-hash>.<ext>` and return its path, or
    /// `Ok(None)` when the package carries no object for that triple (a
    /// pure-ZIL package, or another platform's object).
    ///
    /// A native library cannot be loaded from memory, and a mapped library
    /// cannot be deleted while in use, so the file is content-addressed: an
    /// existing file whose bytes hash to the locked identity is reused and
    /// never rewritten. New files are written to a temporary name and renamed
    /// so a concurrent load never observes a partial library.
    pub fn materialize_native_object(
        &self,
        cache_root: impl AsRef<Path>,
        triple: &str,
    ) -> Result<Option<PathBuf>, LoadError> {
        let Some(bytes) = self.native_object_for(triple) else {
            return Ok(None);
        };
        let id = content_id(bytes);
        let ext = if triple.contains("windows") {
            "dll"
        } else if triple.contains("apple") || triple.contains("darwin") {
            "dylib"
        } else {
            "so"
        };
        let dir = cache_root.as_ref().join("native");
        let path = dir.join(format!("{}.{ext}", id.hex()));
        if let Ok(existing) = std::fs::read(&path) {
            if content_id(&existing) == id {
                return Ok(Some(path));
            }
        }
        std::fs::create_dir_all(&dir).map_err(LoadError::Io)?;
        let tmp = dir.join(format!("{}.{}.tmp", id.hex(), std::process::id()));
        std::fs::write(&tmp, bytes).map_err(LoadError::Io)?;
        if let Err(error) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            // A concurrent loader may have won the race (and may hold the file
            // mapped on Windows); accept it when the content is right.
            match std::fs::read(&path) {
                Ok(existing) if content_id(&existing) == id => {}
                _ => return Err(LoadError::Io(error)),
            }
        }
        Ok(Some(path))
    }

    /// Verify the package lock against a release-pinned registry key.
    pub fn verify_against(&self, trusted_key: &[u8; 32]) -> Result<(), LoadError> {
        self.lock.verify_against(trusted_key)
    }

    /// Attach deterministic type declaration payloads to locked modules.
    pub fn with_type_declarations(
        mut self,
        declarations: Vec<(String, Vec<u8>)>,
    ) -> Result<Self, LoadError> {
        let mut seen = std::collections::HashSet::new();
        for (name, payload) in &declarations {
            if !self.modules.iter().any(|(module, _)| module == name) {
                return Err(LoadError::InvalidManifest(format!(
                    "type declarations provided for unknown module `{name}`"
                )));
            }
            if !seen.insert(name) {
                return Err(LoadError::InvalidManifest(format!(
                    "duplicate type declarations for module `{name}`"
                )));
            }
            let identity_name = format!("{name}.d.ts");
            let expected = self
                .lock
                .lock
                .entries
                .iter()
                .find(|(entry, _)| entry == &identity_name)
                .map(|(_, id)| *id)
                .ok_or_else(|| {
                    LoadError::InvalidManifest(format!(
                        "type declaration `{identity_name}` is not locked"
                    ))
                })?;
            if expected != content_id(payload) {
                return Err(LoadError::InvalidManifest(format!(
                    "type declaration `{identity_name}` has content identity mismatch"
                )));
            }
        }
        self.type_declarations = declarations;
        Ok(self)
    }

    /// Serialize the package deterministically.
    pub fn write(&self) -> Vec<u8> {
        let lock = self.lock.write();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(Self::MAGIC);
        bytes.extend_from_slice(&Self::VERSION.to_le_bytes());
        bytes.extend_from_slice(&(lock.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&lock);
        bytes.extend_from_slice(&(self.modules.len() as u32).to_le_bytes());
        for (name, payload) in &self.modules {
            write_name(&mut bytes, name);
            bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(payload);
        }
        bytes.extend_from_slice(&(self.type_declarations.len() as u32).to_le_bytes());
        for (name, payload) in &self.type_declarations {
            write_name(&mut bytes, name);
            bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(payload);
        }
        bytes.extend_from_slice(&(self.native_objects.len() as u32).to_le_bytes());
        for (triple, payload) in &self.native_objects {
            write_name(&mut bytes, triple);
            bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(payload);
        }
        bytes.extend_from_slice(&self.required_core_version.to_le_bytes());
        bytes
    }

    /// Parse and validate a `.zmlib` package against the running engine's
    /// core contract version.
    pub fn read(bytes: &[u8]) -> Result<Self, LoadError> {
        Self::read_for_engine(bytes, zun_module::CORE_CONTRACT_VERSION)
    }

    /// Parse a package and check it against an explicit engine contract version.
    pub fn read_for_engine(bytes: &[u8], engine: u32) -> Result<Self, LoadError> {
        let package = Self::read_unchecked(bytes)?;
        package.check_core_contract(engine)?;
        Ok(package)
    }

    /// Parse and validate a `.zmlib` package without the core contract check.
    pub fn read_unchecked(bytes: &[u8]) -> Result<Self, LoadError> {
        if bytes.len() < 11 || &bytes[..5] != Self::MAGIC {
            return Err(LoadError::InvalidManifest("bad zmlib header".into()));
        }
        let version = u16::from_le_bytes(bytes[5..7].try_into().unwrap());
        if version > Self::VERSION {
            return Err(LoadError::InvalidManifest(format!(
                "unsupported zmlib version {version}"
            )));
        }
        let lock_len = u32::from_le_bytes(bytes[7..11].try_into().unwrap()) as usize;
        let lock_end = 11usize
            .checked_add(lock_len)
            .ok_or_else(|| LoadError::InvalidManifest("zmlib overflow".into()))?;
        let lock = SignedModuleLock::read(
            bytes
                .get(11..lock_end)
                .ok_or_else(|| LoadError::InvalidManifest("truncated zmlib lock".into()))?,
        )?;
        let mut cursor = lock_end;
        let count = read_u32(bytes, &mut cursor)
            .ok_or_else(|| LoadError::InvalidManifest("truncated zmlib module count".into()))?
            as usize;
        let mut modules = Vec::with_capacity(count);
        for _ in 0..count {
            let name = read_name(bytes, &mut cursor)?;
            let len = read_u32(bytes, &mut cursor)
                .ok_or_else(|| LoadError::InvalidManifest("truncated zmlib module length".into()))?
                as usize;
            let payload = take(bytes, &mut cursor, len)
                .ok_or_else(|| LoadError::InvalidManifest("truncated zmlib module".into()))?
                .to_vec();
            modules.push((name, payload));
        }
        let mut type_declarations = Vec::new();
        if version >= 2 {
            let declaration_count = read_u32(bytes, &mut cursor).ok_or_else(|| {
                LoadError::InvalidManifest("truncated zmlib declaration count".into())
            })? as usize;
            type_declarations.reserve(declaration_count);
            for _ in 0..declaration_count {
                let name = read_name(bytes, &mut cursor)?;
                let len = read_u32(bytes, &mut cursor).ok_or_else(|| {
                    LoadError::InvalidManifest("truncated zmlib declaration length".into())
                })? as usize;
                let payload = take(bytes, &mut cursor, len)
                    .ok_or_else(|| {
                        LoadError::InvalidManifest("truncated zmlib declaration".into())
                    })?
                    .to_vec();
                type_declarations.push((name, payload));
            }
        }
        let mut native_objects = Vec::new();
        if version >= 3 {
            let count = read_u32(bytes, &mut cursor).ok_or_else(|| {
                LoadError::InvalidManifest("truncated zmlib native object count".into())
            })? as usize;
            for _ in 0..count {
                let triple = read_name(bytes, &mut cursor)?;
                let len = read_u32(bytes, &mut cursor).ok_or_else(|| {
                    LoadError::InvalidManifest("truncated zmlib native object length".into())
                })? as usize;
                let payload = take(bytes, &mut cursor, len)
                    .ok_or_else(|| {
                        LoadError::InvalidManifest("truncated zmlib native object".into())
                    })?
                    .to_vec();
                native_objects.push((triple, payload));
            }
        }
        let mut required_core_version = 0;
        if version >= 4 {
            required_core_version = read_u32(bytes, &mut cursor).ok_or_else(|| {
                LoadError::InvalidManifest("truncated zmlib core contract version".into())
            })?;
        }
        if cursor != bytes.len() {
            return Err(LoadError::InvalidManifest("trailing zmlib bytes".into()));
        }
        let mut package = Self::new(lock, modules)?
            .with_type_declarations(type_declarations)?
            .with_native_objects(native_objects)?;
        package.required_core_version = required_core_version;
        Ok(package)
    }

    /// Write a `.zmlib` package to disk.
    pub fn write_file(&self, path: impl AsRef<Path>) -> Result<(), LoadError> {
        std::fs::write(path, self.write()).map_err(LoadError::Io)
    }

    /// Read and validate a `.zmlib` package from disk.
    pub fn read_file(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let bytes = std::fs::read(path).map_err(LoadError::Io)?;
        Self::read(&bytes)
    }
}
