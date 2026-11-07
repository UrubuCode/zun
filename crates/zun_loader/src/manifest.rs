use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::bytes::{read_name, read_u16, read_u32, read_u64, take, write_name};
use crate::{ContentId, LoadError, ModuleCache, ModuleLoader, TrustedKeySet};

/// The flattened dependency manifest consumed by the loader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleManifest {
    /// Identity of the module whose entry point should run.
    pub entry: String,
    /// Module identities in deterministic load order.
    pub dependencies: Vec<String>,
    /// Optional relocatable native object associated with the canonical module.
    /// The loader may ignore it and use the `.zm` fallback.
    pub native_object: Option<String>,
    /// Optional relocatable object containing the embedded `.zm` metadata/IL.
    pub metadata_object: Option<String>,
    /// Optional static runtime library used by the AOT linker.
    pub runtime_library: Option<String>,
    /// Fingerprint of the prelude embedded or linked by the compiled modules.
    /// A missing value denotes a legacy or manually assembled manifest.
    pub prelude_fingerprint: Option<u64>,
}

/// A deterministic lockfile binding module identities to immutable content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleLock {
    /// Module names and their content identities in lockfile order.
    pub entries: Vec<(String, ContentId)>,
    /// Capabilities granted to each module identity.
    pub capabilities: Vec<(String, Vec<String>)>,
}

impl ModuleLock {
    const MAGIC: &'static [u8; 5] = b"ZLOCK";
    const VERSION: u16 = 3;

    /// Construct an empty lockfile.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    /// Add a module identity, rejecting duplicate names.
    pub fn push(&mut self, name: impl Into<String>, id: ContentId) -> Result<(), LoadError> {
        let name = name.into();
        if self.entries.iter().any(|(existing, _)| existing == &name) {
            return Err(LoadError::InvalidManifest(format!(
                "duplicate lock entry `{name}`"
            )));
        }
        self.entries.push((name, id));
        Ok(())
    }

    /// Declare capabilities for a locked module.
    pub fn grant_capabilities(
        &mut self,
        name: impl Into<String>,
        capabilities: impl IntoIterator<Item = String>,
    ) -> Result<(), LoadError> {
        let name = name.into();
        if !self.entries.iter().any(|(entry, _)| entry == &name) {
            return Err(LoadError::InvalidManifest(format!(
                "capabilities reference unknown module `{name}`"
            )));
        }
        if self.capabilities.iter().any(|(entry, _)| entry == &name) {
            return Err(LoadError::InvalidManifest(format!(
                "duplicate capabilities for `{name}`"
            )));
        }
        let mut capabilities: Vec<_> = capabilities.into_iter().collect();
        if let Some(unknown) = capabilities.iter().find(|capability| {
            !matches!(
                capability.as_str(),
                "fs" | "net" | "screen" | "native" | "codegen"
            )
        }) {
            return Err(LoadError::InvalidManifest(format!(
                "unknown capability `{unknown}` for `{name}`"
            )));
        }
        capabilities.sort();
        capabilities.dedup();
        self.capabilities.push((name, capabilities));
        self.capabilities
            .sort_by(|left, right| left.0.cmp(&right.0));
        Ok(())
    }

    /// Return the capabilities granted to a module, in canonical order.
    pub fn capabilities_for(&self, name: &str) -> &[String] {
        self.capabilities
            .iter()
            .find(|(entry, _)| entry == name)
            .map_or(&[], |(_, capabilities)| capabilities.as_slice())
    }

    /// Require one capability for a module before performing a host operation.
    pub fn require_capability(&self, name: &str, capability: &str) -> Result<(), LoadError> {
        if self
            .capabilities_for(name)
            .iter()
            .any(|granted| granted == capability)
        {
            Ok(())
        } else {
            Err(LoadError::InvalidManifest(format!(
                "module `{name}` lacks capability `{capability}`"
            )))
        }
    }

    /// Serialize the lockfile deterministically.
    pub fn write(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(Self::MAGIC);
        bytes.extend_from_slice(&Self::VERSION.to_le_bytes());
        bytes.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (name, id) in &self.entries {
            write_name(&mut bytes, name);
            bytes.extend_from_slice(&id.0);
        }
        bytes.extend_from_slice(&(self.capabilities.len() as u32).to_le_bytes());
        for (name, capabilities) in &self.capabilities {
            write_name(&mut bytes, name);
            bytes.extend_from_slice(&(capabilities.len() as u32).to_le_bytes());
            for capability in capabilities {
                write_name(&mut bytes, capability);
            }
        }
        bytes
    }

    /// Parse and validate a lockfile payload.
    pub fn read(bytes: &[u8]) -> Result<Self, LoadError> {
        if bytes.len() < 11 || &bytes[..5] != Self::MAGIC {
            return Err(LoadError::InvalidManifest("bad lockfile header".into()));
        }
        let version = u16::from_le_bytes(bytes[5..7].try_into().unwrap());
        if version > Self::VERSION {
            return Err(LoadError::InvalidManifest(format!(
                "unsupported lockfile version {version}"
            )));
        }
        let count = u32::from_le_bytes(bytes[7..11].try_into().unwrap()) as usize;
        let mut cursor = 11;
        let mut lock = Self::new();
        for _ in 0..count {
            let name = read_name(bytes, &mut cursor)?;
            let end = cursor
                .checked_add(32)
                .ok_or_else(|| LoadError::InvalidManifest("lockfile overflow".into()))?;
            let raw = bytes
                .get(cursor..end)
                .ok_or_else(|| LoadError::InvalidManifest("truncated lock entry".into()))?;
            cursor = end;
            let mut id = [0u8; 32];
            id.copy_from_slice(raw);
            lock.push(name, ContentId(id))?;
        }
        let capability_count = read_u32(bytes, &mut cursor)
            .ok_or_else(|| LoadError::InvalidManifest("truncated capability section".into()))?
            as usize;
        for _ in 0..capability_count {
            let name = read_name(bytes, &mut cursor)?;
            let count = read_u32(bytes, &mut cursor)
                .ok_or_else(|| LoadError::InvalidManifest("truncated capability count".into()))?
                as usize;
            let mut capabilities = Vec::with_capacity(count);
            for _ in 0..count {
                capabilities.push(read_name(bytes, &mut cursor)?);
            }
            lock.grant_capabilities(name, capabilities)?;
        }
        if cursor != bytes.len() {
            return Err(LoadError::InvalidManifest("trailing lockfile bytes".into()));
        }
        Ok(lock)
    }

    /// Write the lockfile to disk.
    pub fn write_file(&self, path: impl AsRef<Path>) -> Result<(), LoadError> {
        std::fs::write(path, self.write()).map_err(LoadError::Io)
    }

    /// Read and validate a lockfile from disk.
    pub fn read_file(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let bytes = std::fs::read(path).map_err(LoadError::Io)?;
        Self::read(&bytes)
    }

    /// Verify every lock entry is present and intact in a content cache.
    pub fn verify_cache(&self, cache_root: impl AsRef<Path>) -> Result<(), LoadError> {
        for (name, id) in &self.entries {
            if name.ends_with(".d.ts") {
                // Declaration identities are authenticated by the signed lock
                // and carried inside `.zmlib`; they are not executable cache
                // modules and therefore have no `.zm` cache entry.
                continue;
            }
            ModuleCache::load(&cache_root, *id).map_err(|error| {
                LoadError::InvalidManifest(format!(
                    "locked module `{name}` failed verification: {error}"
                ))
            })?;
        }
        Ok(())
    }
}

/// A lockfile plus the public key and signature that authenticate it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedModuleLock {
    /// The authenticated lockfile.
    pub lock: ModuleLock,
    /// Ed25519 verification key used for the signature.
    pub public_key: [u8; 32],
    /// Ed25519 signature over [`ModuleLock::write`].
    pub signature: [u8; 64],
}

impl SignedModuleLock {
    const MAGIC: &'static [u8; 4] = b"ZSIG";
    const VERSION: u16 = 1;

    /// Construct and verify a signed lockfile envelope.
    pub fn new(
        lock: ModuleLock,
        public_key: [u8; 32],
        signature: [u8; 64],
    ) -> Result<Self, LoadError> {
        let signed = Self {
            lock,
            public_key,
            signature,
        };
        signed.verify_signature()?;
        Ok(signed)
    }

    /// Verify the Ed25519 signature over the canonical lockfile bytes.
    pub(crate) fn verify_signature(&self) -> Result<(), LoadError> {
        let key = VerifyingKey::from_bytes(&self.public_key).map_err(|error| {
            LoadError::InvalidManifest(format!("invalid lockfile public key: {error}"))
        })?;
        let signature = Signature::from_bytes(&self.signature);
        key.verify(&self.lock.write(), &signature).map_err(|error| {
            LoadError::InvalidManifest(format!("invalid lockfile signature: {error}"))
        })
    }

    /// Verify the signature and require the release-pinned registry key.
    pub fn verify_against(&self, trusted_key: &[u8; 32]) -> Result<(), LoadError> {
        if &self.public_key != trusted_key {
            return Err(LoadError::InvalidManifest(
                "lockfile key is not trusted by the release".into(),
            ));
        }
        self.verify_with_keys(&TrustedKeySet::new([*trusted_key])?)
    }

    pub fn verify_with_keys(&self, trusted: &TrustedKeySet) -> Result<(), LoadError> {
        if !trusted.contains(&self.public_key) {
            return Err(LoadError::InvalidManifest(
                "lockfile key is not in the trusted key set".into(),
            ));
        }
        self.verify_signature()
    }

    /// Verify the signature and require the key to be release-trusted.
    pub fn verify(&self, trusted: &TrustedKeySet) -> Result<(), LoadError> {
        self.verify_with_keys(trusted)
    }

    /// Serialize the signed envelope.
    pub fn write(&self) -> Vec<u8> {
        let payload = self.lock.write();
        let mut bytes = Vec::with_capacity(4 + 2 + 32 + 64 + 4 + payload.len());
        bytes.extend_from_slice(Self::MAGIC);
        bytes.extend_from_slice(&Self::VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.public_key);
        bytes.extend_from_slice(&self.signature);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes
    }

    /// Parse and verify a signed lockfile envelope.
    pub fn read(bytes: &[u8]) -> Result<Self, LoadError> {
        if bytes.len() < 106 || &bytes[..4] != Self::MAGIC {
            return Err(LoadError::InvalidManifest(
                "bad signed lockfile header".into(),
            ));
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if version > Self::VERSION {
            return Err(LoadError::InvalidManifest(format!(
                "unsupported signed lockfile version {version}"
            )));
        }
        let mut public_key = [0u8; 32];
        public_key.copy_from_slice(&bytes[6..38]);
        let mut signature = [0u8; 64];
        signature.copy_from_slice(&bytes[38..102]);
        let payload_len = u32::from_le_bytes(bytes[102..106].try_into().unwrap()) as usize;
        let end = 106usize
            .checked_add(payload_len)
            .ok_or_else(|| LoadError::InvalidManifest("signed lockfile overflow".into()))?;
        if end != bytes.len() {
            return Err(LoadError::InvalidManifest(
                "invalid signed lockfile payload length".into(),
            ));
        }
        Self::new(ModuleLock::read(&bytes[106..end])?, public_key, signature)
    }
}
impl ModuleManifest {
    pub(crate) const MAGIC: &'static [u8; 4] = b"ZMAN";
    const VERSION: u16 = 4;

    /// Construct a manifest for an entry module and its dependencies.
    pub fn new(entry: impl Into<String>, dependencies: impl IntoIterator<Item = String>) -> Self {
        Self {
            entry: entry.into(),
            dependencies: dependencies.into_iter().collect(),
            native_object: None,
            metadata_object: None,
            runtime_library: None,
            prelude_fingerprint: None,
        }
    }

    /// Associate an optional R2R/AOT object path with this manifest.
    pub fn with_native_object(mut self, path: impl Into<String>) -> Self {
        self.native_object = Some(path.into());
        self
    }

    /// Remove a native object when AOT falls back to the embedded interpreter.
    pub fn without_native_object(mut self) -> Self {
        self.native_object = None;
        self
    }

    /// Associate the object containing embedded module metadata/IL.
    pub fn with_metadata_object(mut self, path: impl Into<String>) -> Self {
        self.metadata_object = Some(path.into());
        self
    }

    /// Record the static runtime library used for AOT linking.
    pub fn with_runtime_library(mut self, path: impl Into<String>) -> Self {
        self.runtime_library = Some(path.into());
        self
    }

    /// Record the prelude fingerprint required by this manifest.
    pub fn with_prelude_fingerprint(mut self, fingerprint: u64) -> Self {
        self.prelude_fingerprint = Some(fingerprint);
        self
    }

    /// Resolve the optional native object relative to the manifest location.
    /// Absolute paths are preserved; a manifest without native code returns
    /// `None` so callers can use the canonical `.zm` fallback.
    pub fn native_object_path(&self, manifest_path: impl AsRef<Path>) -> Option<PathBuf> {
        let object = PathBuf::from(self.native_object.as_deref()?);
        if object.is_absolute() {
            Some(object)
        } else {
            Some(
                manifest_path
                    .as_ref()
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(object),
            )
        }
    }

    /// Resolve the optional embedded metadata object relative to the manifest.
    pub fn metadata_object_path(&self, manifest_path: impl AsRef<Path>) -> Option<PathBuf> {
        let object = PathBuf::from(self.metadata_object.as_deref()?);
        if object.is_absolute() {
            Some(object)
        } else {
            Some(
                manifest_path
                    .as_ref()
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(object),
            )
        }
    }

    /// Resolve the optional AOT runtime library relative to the manifest.
    pub fn runtime_library_path(&self, manifest_path: impl AsRef<Path>) -> Option<PathBuf> {
        let library = PathBuf::from(self.runtime_library.as_deref()?);
        if library.is_absolute() {
            Some(library)
        } else {
            Some(
                manifest_path
                    .as_ref()
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(library),
            )
        }
    }

    /// Reject a manifest whose compiled prelude is not the one expected by
    /// the current compiler/runtime.
    pub fn validate_prelude_fingerprint(&self, expected: u64) -> Result<(), LoadError> {
        match self.prelude_fingerprint {
            Some(actual) if actual == expected => Ok(()),
            Some(actual) => Err(LoadError::InvalidManifest(format!(
                "prelude fingerprint mismatch: expected {expected:#x}, found {actual:#x}"
            ))),
            None => Err(LoadError::InvalidManifest(
                "manifest has no prelude fingerprint".into(),
            )),
        }
    }

    /// Serialize the flattened manifest into a deterministic binary payload.
    pub fn write(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(Self::MAGIC);
        bytes.extend_from_slice(&Self::VERSION.to_le_bytes());
        bytes.extend_from_slice(&(self.dependencies.len() as u32).to_le_bytes());
        write_name(&mut bytes, &self.entry);
        for dependency in &self.dependencies {
            write_name(&mut bytes, dependency);
        }
        bytes.push(u8::from(self.native_object.is_some()));
        if let Some(native_object) = &self.native_object {
            write_name(&mut bytes, native_object);
        }
        bytes.push(u8::from(self.metadata_object.is_some()));
        if let Some(metadata_object) = &self.metadata_object {
            write_name(&mut bytes, metadata_object);
        }
        bytes.push(u8::from(self.runtime_library.is_some()));
        if let Some(runtime_library) = &self.runtime_library {
            write_name(&mut bytes, runtime_library);
        }
        bytes.push(u8::from(self.prelude_fingerprint.is_some()));
        if let Some(fingerprint) = self.prelude_fingerprint {
            bytes.extend_from_slice(&fingerprint.to_le_bytes());
        }
        bytes
    }

    /// Write the manifest to a file using its deterministic binary encoding.
    pub fn write_file(&self, path: impl AsRef<Path>) -> Result<(), LoadError> {
        std::fs::write(path, self.write()).map_err(LoadError::Io)
    }

    /// Decode a manifest produced by [`ModuleManifest::write`].
    pub fn read(bytes: &[u8]) -> Result<Self, LoadError> {
        let mut cursor = 0;
        if take(bytes, &mut cursor, 4) != Some(Self::MAGIC.as_slice()) {
            return Err(LoadError::InvalidManifest("bad manifest magic".into()));
        }
        let version = read_u16(bytes, &mut cursor)
            .ok_or_else(|| LoadError::InvalidManifest("truncated manifest version".into()))?;
        if version > Self::VERSION {
            return Err(LoadError::InvalidManifest(format!(
                "unsupported manifest version {version}"
            )));
        }
        let count = read_u32(bytes, &mut cursor)
            .ok_or_else(|| LoadError::InvalidManifest("truncated dependency count".into()))?;
        let entry = read_name(bytes, &mut cursor)?;
        let mut dependencies = Vec::with_capacity(count as usize);
        for _ in 0..count {
            dependencies.push(read_name(bytes, &mut cursor)?);
        }
        let native_object = if version == 0 {
            None
        } else {
            match take(bytes, &mut cursor, 1) {
                Some([0]) => None,
                Some([1]) => Some(read_name(bytes, &mut cursor)?),
                _ => {
                    return Err(LoadError::InvalidManifest(
                        "invalid native object flag".into(),
                    ))
                }
            }
        };
        let metadata_object = if version < 3 {
            None
        } else {
            match take(bytes, &mut cursor, 1) {
                Some([0]) => None,
                Some([1]) => Some(read_name(bytes, &mut cursor)?),
                _ => {
                    return Err(LoadError::InvalidManifest(
                        "invalid metadata object flag".into(),
                    ))
                }
            }
        };
        let runtime_library = if version < 4 {
            None
        } else {
            match take(bytes, &mut cursor, 1) {
                Some([0]) => None,
                Some([1]) => Some(read_name(bytes, &mut cursor)?),
                _ => {
                    return Err(LoadError::InvalidManifest(
                        "invalid runtime library flag".into(),
                    ))
                }
            }
        };
        let prelude_fingerprint = if version < 2 {
            None
        } else {
            match take(bytes, &mut cursor, 1) {
                Some([0]) => None,
                Some([1]) => Some(read_u64(bytes, &mut cursor).ok_or_else(|| {
                    LoadError::InvalidManifest("truncated prelude fingerprint".into())
                })?),
                _ => {
                    return Err(LoadError::InvalidManifest(
                        "invalid prelude fingerprint flag".into(),
                    ))
                }
            }
        };
        if cursor != bytes.len() {
            return Err(LoadError::InvalidManifest("trailing manifest bytes".into()));
        }
        Ok(Self {
            entry,
            dependencies,
            native_object,
            metadata_object,
            runtime_library,
            prelude_fingerprint,
        })
    }

    /// Read a manifest from a file without scanning the surrounding directory.
    pub fn read_file(path: impl AsRef<Path>) -> Result<Self, LoadError> {
        let bytes = std::fs::read(path).map_err(LoadError::Io)?;
        Self::read(&bytes)
    }

    /// Validate that every manifest identity is present in a loader.
    pub fn validate(&self, loader: &ModuleLoader) -> Result<(), LoadError> {
        if loader.get(&self.entry).is_none() {
            return Err(LoadError::UnresolvedModule(self.entry.clone()));
        }
        let mut seen = std::collections::HashSet::new();
        for dependency in &self.dependencies {
            if dependency == &self.entry {
                return Err(LoadError::InvalidManifest(
                    "entry module cannot also be a dependency".into(),
                ));
            }
            if !seen.insert(dependency) {
                return Err(LoadError::InvalidManifest(format!(
                    "dependency `{dependency}` appears more than once"
                )));
            }
            if loader.get(dependency).is_none() {
                return Err(LoadError::UnresolvedModule(dependency.clone()));
            }
        }
        Ok(())
    }

    /// Return the deterministic load order represented by this flattened
    /// manifest. Dependencies precede the entry module.
    pub fn load_order<'a>(&'a self, loader: &ModuleLoader) -> Result<Vec<&'a str>, LoadError> {
        self.validate(loader)?;
        let mut order: Vec<&str> = self.dependencies.iter().map(String::as_str).collect();
        order.push(&self.entry);
        Ok(order)
    }
}
