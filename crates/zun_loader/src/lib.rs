//! Module loading and scope identity for Zun binaries.
//!
//! Loading is deliberately separate from verification and execution. A
//! [`ModuleScope`] owns the bytes decoded from one `.zm` module and gives the
//! future resolver a stable scope boundary for type and member identity.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use zun_metadata::{TableKind, Token};
use zun_module::{Module, ReadError};
use zun_types::{CoreTypes, TypeError, TypeSystem};

/// A deterministic content identifier for a serialized module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentId(pub [u8; 32]);

/// Release-provisioned Ed25519 keys trusted to authenticate registry metadata.
/// Keys are deliberately supplied outside signed envelopes so an attacker cannot
/// make an arbitrary self-generated key authoritative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedKeySet {
    keys: Vec<[u8; 32]>,
}

impl TrustedKeySet {
    pub fn new(keys: impl IntoIterator<Item = [u8; 32]>) -> Result<Self, LoadError> {
        let mut trusted = Self { keys: Vec::new() };
        for key in keys {
            if trusted.keys.contains(&key) {
                return Err(LoadError::InvalidManifest(
                    "duplicate trusted registry key".into(),
                ));
            }
            trusted.keys.push(key);
        }
        if trusted.keys.is_empty() {
            return Err(LoadError::InvalidManifest(
                "trusted registry key set is empty".into(),
            ));
        }
        Ok(trusted)
    }

    fn contains(&self, key: &[u8; 32]) -> bool {
        self.keys.iter().any(|trusted| trusted == key)
    }

    /// Return the next trusted set after applying signed-index rotation data.
    /// The signed envelope VERSION selects the rotation schema; callers replace
    /// the set only after authenticating that signed index.
    pub fn rotate(
        &self,
        additions: impl IntoIterator<Item = [u8; 32]>,
        revocations: impl IntoIterator<Item = [u8; 32]>,
    ) -> Result<Self, LoadError> {
        let revoked: Vec<_> = revocations.into_iter().collect();
        let mut keys: Vec<_> = self
            .keys
            .iter()
            .copied()
            .filter(|key| !revoked.contains(key))
            .collect();
        for key in additions {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        Self::new(keys)
    }

    /// Return a new set with one key revoked.
    pub fn revoke(&self, key: [u8; 32]) -> Result<Self, LoadError> {
        self.rotate([], [key])
    }
}

impl ContentId {
    /// Return the lowercase hexadecimal cache path component.
    pub fn hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

/// Compute the loader's stable content identifier.
///
/// The identifier is deliberately independent of module names and paths. It
/// is a cache key only; signature verification remains a separate registry
/// concern.
pub fn content_id(bytes: &[u8]) -> ContentId {
    ContentId(Sha256::digest(bytes).into())
}

/// A content-addressed cache of serialized module bytes.
#[derive(Debug, Default)]
pub struct ModuleCache {
    entries: HashMap<ContentId, Vec<u8>>,
}

/// Errors raised by the filesystem-backed content cache.
#[derive(Debug)]
pub enum CacheError {
    /// Filesystem access failed.
    Io(std::io::Error),
    /// The file contents do not match their requested content identity.
    Integrity {
        expected: ContentId,
        found: ContentId,
    },
}

impl core::fmt::Display for CacheError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "module cache I/O failed: {error}"),
            Self::Integrity { expected, found } => {
                write!(
                    f,
                    "module cache integrity mismatch: expected {}, found {}",
                    expected.hex(),
                    found.hex()
                )
            }
        }
    }
}

impl std::error::Error for CacheError {}

impl From<std::io::Error> for CacheError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl ModuleCache {
    /// Insert bytes and return their content identifier.
    pub fn insert(&mut self, bytes: &[u8]) -> ContentId {
        let id = content_id(bytes);
        self.entries.entry(id).or_insert_with(|| bytes.to_vec());
        id
    }

    /// Retrieve the bytes for an identifier, if present.
    pub fn get(&self, id: ContentId) -> Option<&[u8]> {
        self.entries.get(&id).map(Vec::as_slice)
    }

    /// Number of unique content entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Store bytes below `root/<sha256>.zm`, returning the content identity.
    pub fn store(&mut self, root: impl AsRef<Path>, bytes: &[u8]) -> Result<ContentId, CacheError> {
        let id = self.insert(bytes);
        let root = root.as_ref();
        std::fs::create_dir_all(root)?;
        let path = root.join(format!("{}.zm", id.hex()));
        if !path.exists() {
            std::fs::write(path, bytes)?;
        }
        Ok(id)
    }

    /// Read and integrity-check a content-addressed module from disk.
    pub fn load(root: impl AsRef<Path>, id: ContentId) -> Result<Vec<u8>, CacheError> {
        let bytes = std::fs::read(root.as_ref().join(format!("{}.zm", id.hex())))?;
        let found = content_id(&bytes);
        if found != id {
            return Err(CacheError::Integrity {
                expected: id,
                found,
            });
        }
        Ok(bytes)
    }
}

/// A loaded module together with its loader-visible identity.
#[derive(Debug)]
pub struct ModuleScope {
    name: String,
    module: Module,
    types: TypeSystem,
    core_types: Option<CoreTypes>,
    exports: HashMap<String, Export>,
}

/// A definition exported by a loaded module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Export {
    /// A top-level or instance method definition.
    Method(Token),
    /// A type definition.
    Type(Token),
}

impl ModuleScope {
    /// Load a module from serialized `.zm` bytes.
    pub fn from_bytes(name: impl Into<String>, bytes: &[u8]) -> Result<Self, LoadError> {
        let name = name.into();
        let module = Module::read(bytes).map_err(LoadError::Format)?;
        let types = TypeSystem::from_module(&module).map_err(LoadError::Types)?;
        let core_types = if name == "zun:core" {
            Some(CoreTypes::from_module(&module, &types))
        } else {
            None
        };
        let mut exports = HashMap::new();
        for (rid, method) in module.tables.method_defs.iter().enumerate() {
            let method_rid = rid as u32 + 1;
            let is_type_method = module
                .tables
                .type_defs
                .iter()
                .any(|ty| (ty.method_start..ty.method_end).contains(&method_rid));
            if is_type_method {
                continue;
            }
            let name = module.strings.get(method.name).to_owned();
            if exports
                .insert(
                    name.clone(),
                    Export::Method(Token::new(TableKind::MethodDef, method_rid)),
                )
                .is_some()
            {
                return Err(LoadError::DuplicateExport(name));
            }
        }
        for (rid, ty) in module.tables.type_defs.iter().enumerate() {
            let name = module.strings.get(ty.name).to_owned();
            if exports
                .insert(
                    name.clone(),
                    Export::Type(Token::new(TableKind::TypeDef, rid as u32 + 1)),
                )
                .is_some()
            {
                return Err(LoadError::DuplicateExport(name));
            }
        }
        Ok(Self {
            name,
            module,
            types,
            core_types,
            exports,
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

    /// Access the immutable core-type bindings, when this is `zun:core`.
    pub fn core_types(&self) -> Option<&CoreTypes> {
        self.core_types.as_ref()
    }

    /// Resolve an exported definition by its source-level name in O(1).
    pub fn export(&self, name: &str) -> Option<Export> {
        self.exports.get(name).copied()
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

/// Registry of loaded module scopes, keyed by their manifest identity.
#[derive(Debug, Default)]
pub struct ModuleLoader {
    modules: HashMap<String, ModuleScope>,
    trusted_keys: Option<TrustedKeySet>,
}

/// Registry-facing byte fetch boundary. Network, authentication, and index
/// policy stay outside the loader and return canonical module bytes here.
pub trait ModuleFetcher {
    /// Fetch one module by registry identity.
    fn fetch(&self, name: &str) -> Result<Vec<u8>, LoadError>;
}

/// Fetcher for a trusted local registry mirror. Module names are resolved
/// beneath `root` and may not escape it through absolute or parent paths.
#[derive(Debug, Clone)]
pub struct DirectoryFetcher {
    root: PathBuf,
}

impl DirectoryFetcher {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl ModuleFetcher for DirectoryFetcher {
    fn fetch(&self, name: &str) -> Result<Vec<u8>, LoadError> {
        let relative = Path::new(name);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(LoadError::InvalidManifest(format!(
                "unsafe registry module path `{name}`"
            )));
        }
        let path = self.root.join(relative).with_extension("zm");
        std::fs::read(path).map_err(LoadError::Io)
    }
}

/// Fetcher for a registry served over HTTP(S). Content identity and signature
/// checks remain in the loader; this type is transport-only.
#[derive(Debug, Clone)]
pub struct HttpFetcher {
    base_url: String,
}

impl HttpFetcher {
    pub fn new(base_url: impl Into<String>) -> Result<Self, LoadError> {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        if !(base_url.starts_with("http://") || base_url.starts_with("https://")) {
            return Err(LoadError::InvalidManifest(
                "HTTP registry base URL must use http:// or https://".into(),
            ));
        }
        Ok(Self { base_url })
    }

    /// Fetch and decode the registry's deterministic `index.zidx` document.
    pub fn fetch_index(&self) -> Result<ModuleIndex, LoadError> {
        let url = format!("{}/index.zidx", self.base_url);
        use std::io::Read;
        let response = ureq::get(&url).call().map_err(|error| {
            LoadError::InvalidManifest(format!("HTTP registry index fetch failed: {error}"))
        })?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(LoadError::Io)?;
        ModuleIndex::read(&bytes)
    }

    /// Fetch and verify the signed registry index envelope.
    pub fn fetch_signed_index(&self) -> Result<SignedModuleIndex, LoadError> {
        let url = format!("{}/index.zidx", self.base_url);
        use std::io::Read;
        let response = ureq::get(&url).call().map_err(|error| {
            LoadError::InvalidManifest(format!("HTTP signed registry index fetch failed: {error}"))
        })?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(LoadError::Io)?;
        SignedModuleIndex::read(&bytes)
    }

    /// Fetch the registry index and enforce the release-pinned trust anchor.
    pub fn fetch_verified_index(
        &self,
        trusted_key: &[u8; 32],
    ) -> Result<ModuleIndex, LoadError> {
        let signed = self.fetch_signed_index()?;
        signed.verify_against(trusted_key)?;
        Ok(signed.index)
    }
}

impl ModuleFetcher for HttpFetcher {
    fn fetch(&self, name: &str) -> Result<Vec<u8>, LoadError> {
        let relative = Path::new(name);
        if name.is_empty()
            || relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(LoadError::InvalidManifest(format!(
                "unsafe registry module path `{name}`"
            )));
        }
        let url = format!("{}/{}.zm", self.base_url, name);
        use std::io::Read;
        let response = ureq::get(&url).call().map_err(|error| {
            LoadError::InvalidManifest(format!("HTTP module fetch failed: {error}"))
        })?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .read_to_end(&mut bytes)
            .map_err(LoadError::Io)?;
        Ok(bytes)
    }
}

/// A deterministic registry index mapping module identities to immutable
/// content IDs. The index is metadata only; bytes remain in the cache or are
/// supplied by a [`ModuleFetcher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleIndex {
    pub entries: Vec<(String, ContentId)>,
}

impl ModuleIndex {
    const MAGIC: &'static [u8; 5] = b"ZINDX";
    const VERSION: u16 = 1;

    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn push(&mut self, name: impl Into<String>, id: ContentId) -> Result<(), LoadError> {
        let name = name.into();
        if name.is_empty() || self.entries.iter().any(|(existing, _)| existing == &name) {
            return Err(LoadError::InvalidManifest(format!(
                "duplicate or empty index entry `{name}`"
            )));
        }
        self.entries.push((name, id));
        Ok(())
    }

    pub fn resolve(&self, name: &str) -> Option<ContentId> {
        self.entries
            .iter()
            .find(|(entry, _)| entry == name)
            .map(|(_, id)| *id)
    }

    pub fn write(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(Self::MAGIC);
        bytes.extend_from_slice(&Self::VERSION.to_le_bytes());
        bytes.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (name, id) in &self.entries {
            write_name(&mut bytes, name);
            bytes.extend_from_slice(&id.0);
        }
        bytes
    }

    pub fn read(bytes: &[u8]) -> Result<Self, LoadError> {
        let mut cursor = 0;
        if take(bytes, &mut cursor, 5) != Some(Self::MAGIC.as_slice()) {
            return Err(LoadError::InvalidManifest(
                "bad registry index magic".into(),
            ));
        }
        if read_u16(bytes, &mut cursor) != Some(Self::VERSION) {
            return Err(LoadError::InvalidManifest(
                "unsupported registry index version".into(),
            ));
        }
        let count = read_u32(bytes, &mut cursor)
            .ok_or_else(|| LoadError::InvalidManifest("truncated registry index".into()))?;
        let mut index = Self::new();
        for _ in 0..count {
            let name = read_name(bytes, &mut cursor)?;
            let raw = take(bytes, &mut cursor, 32)
                .ok_or_else(|| LoadError::InvalidManifest("truncated content ID".into()))?;
            let mut id = [0u8; 32];
            id.copy_from_slice(raw);
            index.push(name, ContentId(id))?;
        }
        if cursor != bytes.len() {
            return Err(LoadError::InvalidManifest(
                "trailing registry index data".into(),
            ));
        }
        Ok(index)
    }
}

/// A registry index authenticated by an Ed25519 signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedModuleIndex {
    pub index: ModuleIndex,
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
}

impl SignedModuleIndex {
    const MAGIC: &'static [u8; 4] = b"ZISG";
    const VERSION: u16 = 1;

    pub fn new(
        index: ModuleIndex,
        public_key: [u8; 32],
        signature: [u8; 64],
    ) -> Result<Self, LoadError> {
        let signed = Self {
            index,
            public_key,
            signature,
        };
        signed.verify_signature()?;
        Ok(signed)
    }

    fn verify_signature(&self) -> Result<(), LoadError> {
        let key = VerifyingKey::from_bytes(&self.public_key).map_err(|error| {
            LoadError::InvalidManifest(format!("invalid index public key: {error}"))
        })?;
        let signature = Signature::from_bytes(&self.signature);
        key.verify(&self.index.write(), &signature)
            .map_err(|error| {
                LoadError::InvalidManifest(format!("invalid index signature: {error}"))
            })
    }

    /// Verify that this index was signed by a release-pinned root key.
    pub fn verify_against(&self, trusted_key: &[u8; 32]) -> Result<(), LoadError> {
        if &self.public_key != trusted_key {
            return Err(LoadError::InvalidManifest(
                "registry index key is not trusted by the release".into(),
            ));
        }
        self.verify_signature()
    }

    pub fn verify_with_keys(&self, trusted: &TrustedKeySet) -> Result<(), LoadError> {
        if !trusted.contains(&self.public_key) {
            return Err(LoadError::InvalidManifest(
                "registry index key is not in the trusted key set".into(),
            ));
        }
        self.verify_signature()
    }

    pub fn write(&self) -> Vec<u8> {
        let payload = self.index.write();
        let mut bytes = Vec::with_capacity(106 + payload.len());
        bytes.extend_from_slice(Self::MAGIC);
        bytes.extend_from_slice(&Self::VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.public_key);
        bytes.extend_from_slice(&self.signature);
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes
    }

    pub fn read(bytes: &[u8]) -> Result<Self, LoadError> {
        if bytes.len() < 106 || &bytes[..4] != Self::MAGIC {
            return Err(LoadError::InvalidManifest(
                "bad signed registry index header".into(),
            ));
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if version > Self::VERSION {
            return Err(LoadError::InvalidManifest(format!(
                "unsupported signed registry index version {version}"
            )));
        }
        let mut public_key = [0u8; 32];
        public_key.copy_from_slice(&bytes[6..38]);
        let mut signature = [0u8; 64];
        signature.copy_from_slice(&bytes[38..102]);
        let payload_len = u32::from_le_bytes(bytes[102..106].try_into().unwrap()) as usize;
        let end = 106usize
            .checked_add(payload_len)
            .ok_or_else(|| LoadError::InvalidManifest("signed registry index overflow".into()))?;
        if end != bytes.len() {
            return Err(LoadError::InvalidManifest(
                "invalid signed registry index payload length".into(),
            ));
        }
        Self::new(ModuleIndex::read(&bytes[106..end])?, public_key, signature)
    }
}

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
    fn verify_signature(&self) -> Result<(), LoadError> {
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
}

impl Zmlib {
    const MAGIC: &'static [u8; 5] = b"ZMLIB";
    const VERSION: u16 = 2;

    /// Construct a package and ensure every payload matches its lock entry.
    pub fn new(lock: SignedModuleLock, modules: Vec<(String, Vec<u8>)>) -> Result<Self, LoadError> {
        lock.verify_signature()?;
        for (name, bytes) in &modules {
            let expected = lock
                .lock
                .entries
                .iter()
                .find(|(entry, _)| entry == name)
                .map(|(_, id)| *id)
                .ok_or_else(|| {
                    LoadError::InvalidManifest(format!("package module `{name}` is not locked"))
                })?;
            let found = content_id(bytes);
            if expected != found {
                return Err(LoadError::InvalidManifest(format!(
                    "package module `{name}` has content identity mismatch"
                )));
            }
        }
        Ok(Self {
            lock,
            modules,
            type_declarations: Vec::new(),
        })
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
        bytes
    }

    /// Parse and validate a `.zmlib` package.
    pub fn read(bytes: &[u8]) -> Result<Self, LoadError> {
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
        if cursor != bytes.len() {
            return Err(LoadError::InvalidManifest("trailing zmlib bytes".into()));
        }
        Self::new(lock, modules)?.with_type_declarations(type_declarations)
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

impl ModuleManifest {
    const MAGIC: &'static [u8; 4] = b"ZMAN";
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

fn write_name(bytes: &mut Vec<u8>, name: &str) {
    bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
    bytes.extend_from_slice(name.as_bytes());
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = cursor.checked_add(len)?;
    let value = bytes.get(*cursor..end)?;
    *cursor = end;
    Some(value)
}

fn read_u16(bytes: &[u8], cursor: &mut usize) -> Option<u16> {
    Some(u16::from_le_bytes(take(bytes, cursor, 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> Option<u32> {
    Some(u32::from_le_bytes(take(bytes, cursor, 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], cursor: &mut usize) -> Option<u64> {
    Some(u64::from_le_bytes(take(bytes, cursor, 8)?.try_into().ok()?))
}

fn read_name(bytes: &[u8], cursor: &mut usize) -> Result<String, LoadError> {
    let len = read_u32(bytes, cursor)
        .ok_or_else(|| LoadError::InvalidManifest("truncated name length".into()))?;
    let raw = take(bytes, cursor, len as usize)
        .ok_or_else(|| LoadError::InvalidManifest("truncated name".into()))?;
    String::from_utf8(raw.to_vec())
        .map_err(|_| LoadError::InvalidManifest("manifest name is not UTF-8".into()))
}

/// The kind of member resolved from a MemberRef.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// A method definition.
    Method,
    /// A field definition.
    Field,
    /// A type definition.
    Type,
}

/// A member token together with the scope that owns it.
#[derive(Debug)]
pub struct ResolvedMember<'a> {
    /// The loaded module that owns the definition.
    pub scope: &'a ModuleScope,
    /// The destination definition token.
    pub token: Token,
    /// Whether the token points to a method or field.
    pub kind: MemberKind,
}

impl ModuleLoader {
    /// Create an empty module loader.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a loader whose signed package operations use release-provisioned
    /// registry keys.
    pub fn new_with_trusted_keys(trusted_keys: TrustedKeySet) -> Self {
        Self {
            modules: HashMap::new(),
            trusted_keys: Some(trusted_keys),
        }
    }

    /// Load and register a module under `name`.
    pub fn load_bytes(&mut self, name: impl Into<String>, bytes: &[u8]) -> Result<(), LoadError> {
        let name = name.into();
        let scope = ModuleScope::from_bytes(name.clone(), bytes)?;
        self.insert(scope)
    }

    /// Load a lockfile-selected artifact from the content-addressed cache.
    pub fn load_cached(
        &mut self,
        cache_root: impl AsRef<Path>,
        name: impl Into<String>,
        id: ContentId,
    ) -> Result<(), LoadError> {
        let name = name.into();
        let bytes = ModuleCache::load(cache_root, id).map_err(|error| {
            LoadError::InvalidManifest(format!("cached module `{name}` unavailable: {error}"))
        })?;
        self.load_bytes(name, &bytes)
    }

    /// Fetch, verify, and register one module selected by a lockfile entry.
    pub fn load_fetched(
        &mut self,
        name: &str,
        expected: ContentId,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let bytes = fetcher.fetch(name)?;
        let found = content_id(&bytes);
        if found != expected {
            return Err(LoadError::InvalidManifest(format!(
                "fetched module `{name}` has content identity mismatch"
            )));
        }
        self.load_bytes(name.to_owned(), &bytes)
    }

    /// Fetch and register every module in lockfile order.
    pub fn load_lock(
        &mut self,
        lock: &ModuleLock,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        for (name, id) in &lock.entries {
            if name.ends_with(".d.ts") {
                continue;
            }
            self.load_fetched(name, *id, fetcher)?;
        }
        Ok(())
    }

    /// Fetch and register every module published by a registry index in its
    /// deterministic order. Content IDs are checked before any scope is
    /// inserted, so an index cannot silently select altered bytes.
    pub fn load_index(
        &mut self,
        index: &ModuleIndex,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        for (name, id) in &index.entries {
            self.load_fetched(name, *id, fetcher)?;
        }
        Ok(())
    }

    /// Verify a signed registry index against the release trust anchor before
    /// fetching any of its modules.
    pub fn load_signed_index(
        &mut self,
        signed: &SignedModuleIndex,
        trusted_key: &[u8; 32],
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        signed.verify_against(trusted_key)?;
        self.load_index(&signed.index, fetcher)
    }

    /// Verify a signed index against a release-provisioned key set before
    /// fetching any selected module.
    pub fn load_signed_index_with_keys(
        &mut self,
        signed: &SignedModuleIndex,
        trusted: &TrustedKeySet,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        signed.verify_with_keys(trusted)?;
        self.load_index(&signed.index, fetcher)
    }

    /// Load a signed index using the key set retained by this loader.
    pub fn load_signed_index_trusted(
        &mut self,
        signed: &SignedModuleIndex,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let trusted = self.trusted_keys.as_ref().ok_or_else(|| {
            LoadError::InvalidManifest("loader has no trusted registry key set".into())
        })?;
        signed.verify_with_keys(trusted)?;
        self.load_index(&signed.index, fetcher)
    }

    /// Verify and register every module carried by a `.zmlib` package.
    pub fn load_zmlib(&mut self, package: &Zmlib) -> Result<(), LoadError> {
        if let Some(trusted) = &self.trusted_keys {
            package.lock.verify_with_keys(trusted)?;
        } else {
            package.lock.verify_signature()?;
        }
        for (name, bytes) in &package.modules {
            self.load_bytes(name.clone(), bytes)?;
        }
        Ok(())
    }

    /// Load a package only after its lock key is anchored in the release key set.
    pub fn load_zmlib_with_keys(
        &mut self,
        package: &Zmlib,
        trusted: &TrustedKeySet,
    ) -> Result<(), LoadError> {
        package.lock.verify_with_keys(trusted)?;
        for (name, bytes) in &package.modules {
            self.load_bytes(name.clone(), bytes)?;
        }
        Ok(())
    }

    /// Load and register a module using its path as identity.
    pub fn load_file(&mut self, path: impl AsRef<Path>) -> Result<(), LoadError> {
        let scope = ModuleScope::from_file(path)?;
        self.insert(scope)
    }

    /// Load every module named by a flattened manifest in dependency order.
    ///
    /// Manifest identities are file paths for this file-based host API. A
    /// higher-level package host can resolve identities to bytes first and use
    /// [`ModuleLoader::load_bytes`] when paths are not the distribution key.
    pub fn load_manifest(&mut self, manifest: &ModuleManifest) -> Result<(), LoadError> {
        let mut seen = std::collections::HashSet::new();
        for name in manifest
            .dependencies
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(manifest.entry.as_str()))
        {
            if !seen.insert(name) {
                return Err(LoadError::InvalidManifest(format!(
                    "module `{name}` appears more than once"
                )));
            }
            self.load_file(name)?;
        }
        manifest.validate(self)
    }

    /// Load a manifest only when its prelude matches the active compiler
    /// fingerprint. This is the cache-aware entry point; [`Self::load_manifest`]
    /// remains available for legacy or manually assembled manifests.
    pub fn load_manifest_with_prelude(
        &mut self,
        manifest: &ModuleManifest,
        expected_prelude: u64,
    ) -> Result<(), LoadError> {
        manifest.validate_prelude_fingerprint(expected_prelude)?;
        self.load_manifest(manifest)
    }

    /// Register an already loaded scope, rejecting duplicate identities.
    pub fn insert(&mut self, scope: ModuleScope) -> Result<(), LoadError> {
        let name = scope.name().to_owned();
        if self.modules.contains_key(&name) {
            return Err(LoadError::DuplicateModule(name));
        }
        self.modules.insert(name, scope);
        Ok(())
    }

    /// Resolve a previously loaded module by exact identity.
    pub fn get(&self, name: &str) -> Option<&ModuleScope> {
        self.modules.get(name)
    }

    /// Resolve an exported definition from a loaded module by module and
    /// source-level name. The module and export indexes are both O(1).
    pub fn resolve_export<'a>(
        &'a self,
        module: &str,
        export: &str,
    ) -> Result<(&'a ModuleScope, Export), LoadError> {
        let scope = self
            .get(module)
            .ok_or_else(|| LoadError::UnresolvedModule(module.to_owned()))?;
        let definition = scope
            .export(export)
            .ok_or_else(|| LoadError::UnresolvedModule(format!("{module}::{export}")))?;
        Ok((scope, definition))
    }

    /// Resolve a ModuleRef token using the requesting module's string heap.
    pub fn resolve_module_ref<'a>(
        &'a self,
        requester: &ModuleScope,
        token: Token,
    ) -> Result<&'a ModuleScope, LoadError> {
        if token.table() != Some(TableKind::ModuleRef) {
            return Err(LoadError::UnresolvedModule(format!(
                "invalid token 0x{:08x}",
                token.to_raw()
            )));
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

    /// Resolve a MemberRef to a definition in the local or referenced scope.
    pub fn resolve_member_ref<'a>(
        &'a self,
        requester: &'a ModuleScope,
        token: Token,
    ) -> Result<ResolvedMember<'a>, LoadError> {
        if token.table() != Some(TableKind::MemberRef) {
            return Err(LoadError::UnresolvedModule(format!(
                "invalid member token 0x{:08x}",
                token.to_raw()
            )));
        }
        let member = requester
            .module
            .tables
            .member_refs
            .get(token.rid() as usize - 1)
            .ok_or_else(|| LoadError::UnresolvedModule(format!("member RID {}", token.rid())))?;

        if member.parent.table() == Some(TableKind::ModuleRef) {
            let target = self.resolve_module_ref(requester, member.parent)?;
            let member_name = requester.module.strings.get(member.name);
            return match target.export(member_name) {
                Some(Export::Method(token)) => Ok(ResolvedMember {
                    scope: target,
                    token,
                    kind: MemberKind::Method,
                }),
                Some(Export::Type(token)) => Ok(ResolvedMember {
                    scope: target,
                    token,
                    kind: MemberKind::Type,
                }),
                None => Err(LoadError::UnresolvedModule(format!(
                    "{}::{member_name}",
                    target.name()
                ))),
            };
        }

        let target = match member.parent.table() {
            Some(TableKind::TypeDef) => requester,
            Some(TableKind::TypeRef) => {
                let type_ref = requester
                    .module
                    .tables
                    .type_refs
                    .get(member.parent.rid() as usize - 1)
                    .ok_or_else(|| LoadError::UnresolvedModule("invalid type reference".into()))?;
                self.resolve_module_ref(requester, type_ref.scope)?
            }
            _ => {
                return Err(LoadError::UnresolvedModule(
                    "member parent is not a type".into(),
                ))
            }
        };
        let type_name = match member.parent.table() {
            Some(TableKind::TypeDef) => requester
                .module
                .strings
                .get(requester.module.tables.type_defs[member.parent.rid() as usize - 1].name),
            Some(TableKind::TypeRef) => {
                let type_ref = &requester.module.tables.type_refs[member.parent.rid() as usize - 1];
                requester.module.strings.get(type_ref.name)
            }
            _ => unreachable!(),
        };
        let ty = target
            .module
            .tables
            .type_defs
            .iter()
            .find(|ty| target.module.strings.get(ty.name) == type_name)
            .ok_or_else(|| LoadError::UnresolvedModule(type_name.to_string()))?;
        let member_name = requester.module.strings.get(member.name);
        let method = (ty.method_start..ty.method_end).find(|rid| {
            target
                .module
                .strings
                .get(target.module.tables.method_defs[*rid as usize - 1].name)
                == member_name
        });
        if let Some(rid) = method {
            return Ok(ResolvedMember {
                scope: target,
                token: Token::new(TableKind::MethodDef, rid),
                kind: MemberKind::Method,
            });
        }
        let field = (ty.field_start..ty.field_end).find(|rid| {
            target
                .module
                .strings
                .get(target.module.tables.field_defs[*rid as usize - 1].name)
                == member_name
        });
        field
            .map(|rid| ResolvedMember {
                scope: target,
                token: Token::new(TableKind::FieldDef, rid),
                kind: MemberKind::Field,
            })
            .ok_or_else(|| LoadError::UnresolvedModule(member_name.to_string()))
    }

    /// Resolve a top-level member reference stored in a raw requesting module.
    /// This is used by runtimes before a full scope-aware frame is available.
    pub fn resolve_member_ref_in_module<'a>(
        &'a self,
        requester: &Module,
        token: Token,
    ) -> Result<ResolvedMember<'a>, LoadError> {
        if token.table() != Some(TableKind::MemberRef) {
            return Err(LoadError::UnresolvedModule(format!(
                "invalid member token 0x{:08x}",
                token.to_raw()
            )));
        }
        let member = requester
            .tables
            .member_refs
            .get(token.rid() as usize - 1)
            .ok_or_else(|| LoadError::UnresolvedModule(format!("member RID {}", token.rid())))?;
        if member.parent.table() != Some(TableKind::ModuleRef) {
            return Err(LoadError::UnresolvedModule(
                "only top-level external members are supported".into(),
            ));
        }
        let module_ref = requester
            .tables
            .module_refs
            .get(member.parent.rid() as usize - 1)
            .ok_or_else(|| LoadError::UnresolvedModule("invalid module reference".into()))?;
        let module_name = requester.strings.get(module_ref.name);
        let target = self
            .get(module_name)
            .ok_or_else(|| LoadError::UnresolvedModule(module_name.to_owned()))?;
        let member_name = requester.strings.get(member.name);
        match target.export(member_name) {
            Some(Export::Method(token)) => Ok(ResolvedMember {
                scope: target,
                token,
                kind: MemberKind::Method,
            }),
            Some(Export::Type(_)) => Err(LoadError::UnresolvedModule(format!(
                "{}::{member_name} is not a method",
                target.name()
            ))),
            None => Err(LoadError::UnresolvedModule(format!(
                "{}::{member_name}",
                target.name()
            ))),
        }
    }

    /// Resolve a member reference and require that it names a callable method.
    pub fn resolve_method_ref<'a>(
        &'a self,
        requester: &'a ModuleScope,
        token: Token,
    ) -> Result<(&'a ModuleScope, Token), LoadError> {
        let resolved = self.resolve_member_ref(requester, token)?;
        if resolved.kind != MemberKind::Method {
            return Err(LoadError::UnresolvedModule(format!(
                "member token 0x{:08x} is not a method",
                token.to_raw()
            )));
        }
        Ok((resolved.scope, resolved.token))
    }

    /// Validate every member reference declared by the modules in a manifest.
    pub fn validate_member_refs(&self, manifest: &ModuleManifest) -> Result<(), LoadError> {
        for name in manifest.load_order(self)? {
            let requester = self
                .get(name)
                .ok_or_else(|| LoadError::UnresolvedModule(name.to_owned()))?;
            for rid in 1..=requester.module.tables.member_refs.len() as u32 {
                self.resolve_member_ref(requester, Token::new(TableKind::MemberRef, rid))?;
            }
        }
        Ok(())
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
    fn content_cache_deduplicates_identical_module_bytes() {
        let mut cache = ModuleCache::default();
        let first = cache.insert(b"module");
        let second = cache.insert(b"module");
        assert_eq!(first, second);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(first), Some(&b"module"[..]));
    }

    #[test]
    fn content_id_does_not_depend_on_module_name() {
        assert_eq!(content_id(b"same"), content_id(b"same"));
        assert_ne!(content_id(b"same"), content_id(b"different"));
    }

    #[test]
    fn filesystem_cache_roundtrips_and_rejects_corruption() {
        let root = std::env::temp_dir().join(format!("zun-content-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut cache = ModuleCache::default();
        let id = cache.store(&root, b"module bytes").unwrap();
        assert_eq!(ModuleCache::load(&root, id).unwrap(), b"module bytes");
        std::fs::write(root.join(format!("{}.zm", id.hex())), b"tampered").unwrap();
        assert!(matches!(
            ModuleCache::load(&root, id),
            Err(CacheError::Integrity { .. })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lockfile_roundtrips_content_identities_and_rejects_duplicates() {
        let mut lock = ModuleLock::new();
        let root = std::env::temp_dir().join(format!("zun-lock-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut cache = ModuleCache::default();
        let std_id = cache.store(&root, b"std").unwrap();
        let app_id = cache.store(&root, b"app").unwrap();
        lock.push("zun:std", std_id).unwrap();
        lock.push("app", app_id).unwrap();
        lock.grant_capabilities("app", ["net".to_owned(), "fs".to_owned(), "net".to_owned()])
            .unwrap();
        assert!(lock.push("app", content_id(b"other")).is_err());
        let decoded = ModuleLock::read(&lock.write()).unwrap();
        assert_eq!(decoded, lock);
        decoded.verify_cache(&root).unwrap();
        let mut metadata_lock = ModuleLock::new();
        metadata_lock.push("app", app_id).unwrap();
        metadata_lock
            .push("app.d.ts", content_id(b"declare const x: int;"))
            .unwrap();
        metadata_lock.verify_cache(&root).unwrap();
        assert_eq!(
            decoded.capabilities,
            vec![("app".to_owned(), vec!["fs".to_owned(), "net".to_owned()])]
        );
        assert!(decoded.require_capability("app", "fs").is_ok());
        assert!(decoded.require_capability("app", "camera").is_err());
        let path = std::env::temp_dir().join(format!("zun-lock-{}.zlock", std::process::id()));
        lock.write_file(&path).unwrap();
        assert_eq!(ModuleLock::read_file(&path).unwrap(), lock);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn signed_lockfile_roundtrips_and_rejects_tampering() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(b"app")).unwrap();
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let signature = signing.sign(&lock.write()).to_bytes();
        let signed =
            SignedModuleLock::new(lock, signing.verifying_key().to_bytes(), signature).unwrap();
        let bytes = signed.write();
        assert_eq!(SignedModuleLock::read(&bytes).unwrap(), signed);
        assert!(signed
            .verify_against(&signing.verifying_key().to_bytes())
            .is_ok());
        assert!(signed.verify_against(&[99u8; 32]).is_err());
        let mut tampered = bytes;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(SignedModuleLock::read(&tampered).is_err());
    }

    #[test]
    fn signed_lockfile_valid_anchored_key_passes() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(b"app")).unwrap();
        let signing = SigningKey::from_bytes(&[21u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            signing.verifying_key().to_bytes(),
            signing.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let trusted = TrustedKeySet::new([signing.verifying_key().to_bytes()]).unwrap();
        assert!(signed.verify_with_keys(&trusted).is_ok());
    }

    #[test]
    fn signed_lockfile_invalid_self_declared_key_is_rejected() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(b"app")).unwrap();
        let attacker = SigningKey::from_bytes(&[22u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            attacker.verifying_key().to_bytes(),
            attacker.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let trusted = TrustedKeySet::new([[23u8; 32]]).unwrap();
        assert!(signed.verify_with_keys(&trusted).is_err());
    }

    #[test]
    fn trusted_key_set_valid_rotation_and_invalid_empty_revocation() {
        let old = [41u8; 32];
        let next = [42u8; 32];
        let trusted = TrustedKeySet::new([old]).unwrap();
        let rotated = trusted.rotate([next], [old]).unwrap();
        assert!(!rotated.contains(&old));
        assert!(rotated.contains(&next));
        assert!(rotated.revoke(next).is_err());
    }

    #[test]
    fn zmlib_roundtrips_signed_modules_and_rejects_identity_mismatch() {
        use ed25519_dalek::{Signer, SigningKey};
        let payload = b"serialized zm".to_vec();
        let declaration = b"declare function main(): int;".to_vec();
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(&payload)).unwrap();
        lock.push("app.d.ts", content_id(&declaration)).unwrap();
        let signing = SigningKey::from_bytes(&[9u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            signing.verifying_key().to_bytes(),
            signing.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let package = Zmlib::new(signed, vec![("app".into(), payload.clone())])
            .unwrap()
            .with_type_declarations(vec![("app".into(), declaration.clone())])
            .unwrap();
        assert!(package
            .verify_against(&signing.verifying_key().to_bytes())
            .is_ok());
        assert!(package.verify_against(&[99u8; 32]).is_err());
        assert_eq!(Zmlib::read(&package.write()).unwrap(), package);
        assert!(
            Zmlib::new(package.lock.clone(), vec![("app".into(), payload.clone())])
                .unwrap()
                .with_type_declarations(vec![("missing".into(), b"declare const x: int;".to_vec())])
                .is_err()
        );
        assert!(
            Zmlib::new(package.lock.clone(), vec![("app".into(), payload.clone())])
                .unwrap()
                .with_type_declarations(vec![
                    ("app".into(), b"declare function main(): int;".to_vec()),
                    ("app".into(), b"declare const x: int;".to_vec()),
                ])
                .is_err()
        );
        assert!(
            Zmlib::new(package.lock.clone(), vec![("app".into(), payload.clone())])
                .unwrap()
                .with_type_declarations(vec![("app".into(), b"tampered declarations".to_vec())])
                .is_err()
        );
        let path = std::env::temp_dir().join(format!("zun-package-{}.zmlib", std::process::id()));
        package.write_file(&path).unwrap();
        let loaded = Zmlib::read_file(&path).unwrap();
        let mut loader = ModuleLoader::new();
        assert!(loader.load_zmlib(&loaded).is_err());
        std::fs::remove_file(path).unwrap();
        assert!(Zmlib::new(package.lock, vec![("app".into(), b"tampered".to_vec())]).is_err());
    }

    #[test]
    fn loader_registers_a_verified_cached_module() {
        let root = std::env::temp_dir().join(format!("zun-loader-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bytes = Module::new().write();
        let mut cache = ModuleCache::default();
        let id = cache.store(&root, &bytes).unwrap();
        let mut loader = ModuleLoader::new();
        loader.load_cached(&root, "cached", id).unwrap();
        assert!(loader.get("cached").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trusted_loader_rejects_an_unanchored_package_key() {
        use ed25519_dalek::{Signer, SigningKey};
        let payload = Module::new().write();
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(&payload)).unwrap();
        let attacker = SigningKey::from_bytes(&[31u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            attacker.verifying_key().to_bytes(),
            attacker.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let package = Zmlib::new(signed, vec![("app".into(), payload)]).unwrap();
        let trusted = TrustedKeySet::new([[32u8; 32]]).unwrap();
        let mut loader = ModuleLoader::new_with_trusted_keys(trusted);
        assert!(loader.load_zmlib(&package).is_err());
    }

    #[test]
    fn loader_verifies_fetched_content_before_registration() {
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let bytes = Module::new().write();
        let registry = Registry(bytes.clone());
        let mut loader = ModuleLoader::new();
        loader
            .load_fetched("remote", content_id(&bytes), &registry)
            .unwrap();
        assert!(loader.get("remote").is_some());
        assert!(loader
            .load_fetched("bad", content_id(b"other"), &registry)
            .is_err());
        let mut lock = ModuleLock::new();
        lock.push("remote-a", content_id(&bytes)).unwrap();
        lock.push("remote-b", content_id(&bytes)).unwrap();
        let mut all = ModuleLoader::new();
        all.load_lock(&lock, &registry).unwrap();
        assert!(all.get("remote-a").is_some());
        assert!(all.get("remote-b").is_some());
    }

    #[test]
    fn registry_index_roundtrips_and_rejects_duplicate_names() {
        let mut index = ModuleIndex::new();
        let id = content_id(b"module");
        index.push("@scope/module", id).unwrap();
        assert_eq!(index.resolve("@scope/module"), Some(id));
        assert_eq!(index.resolve("missing"), None);
        assert!(index.push("@scope/module", id).is_err());
        assert_eq!(ModuleIndex::read(&index.write()).unwrap(), index);
        let mut trailing = index.write();
        trailing.push(0);
        assert!(ModuleIndex::read(&trailing).is_err());
    }

    #[test]
    fn signed_registry_index_roundtrips_and_rejects_tampering() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut index = ModuleIndex::new();
        index.push("app", content_id(b"app")).unwrap();
        let signing = SigningKey::from_bytes(&[13u8; 32]);
        let signature = signing.sign(&index.write()).to_bytes();
        let signed =
            SignedModuleIndex::new(index, signing.verifying_key().to_bytes(), signature).unwrap();
        let bytes = signed.write();
        assert_eq!(SignedModuleIndex::read(&bytes).unwrap(), signed);
        assert!(signed
            .verify_against(&signing.verifying_key().to_bytes())
            .is_ok());
        assert!(signed.verify_against(&[99u8; 32]).is_err());
        let mut tampered = bytes;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(SignedModuleIndex::read(&tampered).is_err());
    }

    #[test]
    fn loader_consumes_registry_index_with_content_verification() {
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let bytes = Module::new().write();
        let id = content_id(&bytes);
        let mut index = ModuleIndex::new();
        index.push("indexed", id).unwrap();
        let mut loader = ModuleLoader::new();
        loader.load_index(&index, &Registry(bytes)).unwrap();
        assert!(loader.get("indexed").is_some());

        let mut bad = ModuleIndex::new();
        bad.push("indexed", content_id(b"different")).unwrap();
        assert!(ModuleLoader::new()
            .load_index(&bad, &Registry(Module::new().write()))
            .is_err());
    }

    #[test]
    fn loader_requires_the_pinned_key_before_signed_index_fetches() {
        use ed25519_dalek::{Signer, SigningKey};
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let bytes = Module::new().write();
        let mut index = ModuleIndex::new();
        index.push("indexed", content_id(&bytes)).unwrap();
        let signing = SigningKey::from_bytes(&[17u8; 32]);
        let index_bytes = index.write();
        let signed = SignedModuleIndex::new(
            index,
            signing.verifying_key().to_bytes(),
            signing.sign(&index_bytes).to_bytes(),
        )
        .unwrap();
        let mut loader = ModuleLoader::new();
        assert!(loader
            .load_signed_index(&signed, &[99u8; 32], &Registry(bytes.clone()))
            .is_err());
        loader
            .load_signed_index(&signed, &signing.verifying_key().to_bytes(), &Registry(bytes))
            .unwrap();
        assert!(loader.get("indexed").is_some());
        let mut trusted_loader = ModuleLoader::new_with_trusted_keys(
            TrustedKeySet::new([signing.verifying_key().to_bytes()]).unwrap(),
        );
        trusted_loader
            .load_signed_index_trusted(&signed, &Registry(Module::new().write()))
            .unwrap();
        assert!(trusted_loader.get("indexed").is_some());
    }

    #[test]
    fn loader_skips_metadata_entries_in_signed_lock_fetches() {
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, name: &str) -> Result<Vec<u8>, LoadError> {
                if name.ends_with(".d.ts") {
                    return Err(LoadError::InvalidManifest(format!(
                        "metadata fetch attempted: {name}"
                    )));
                }
                Ok(self.0.clone())
            }
        }
        let bytes = Module::new().write();
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(&bytes)).unwrap();
        lock.push("app.d.ts", content_id(b"declare const x: int;"))
            .unwrap();
        let mut loader = ModuleLoader::new();
        loader.load_lock(&lock, &Registry(bytes)).unwrap();
        assert!(loader.get("app").is_some());
        assert!(loader.get("app.d.ts").is_none());
    }

    #[test]
    fn directory_fetcher_loads_indexed_modules_and_rejects_escape_paths() {
        let root = std::env::temp_dir().join(format!("zun-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("scope")).unwrap();
        let bytes = Module::new().write();
        std::fs::write(root.join("scope").join("module.zm"), &bytes).unwrap();
        let mut index = ModuleIndex::new();
        index.push("scope/module", content_id(&bytes)).unwrap();
        let fetcher = DirectoryFetcher::new(&root);
        let mut loader = ModuleLoader::new();
        loader.load_index(&index, &fetcher).unwrap();
        assert!(loader.get("scope/module").is_some());
        assert!(fetcher.fetch("../outside").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn http_fetcher_validates_registry_urls_and_module_paths() {
        assert!(HttpFetcher::new("file:///tmp/registry").is_err());
        let fetcher = HttpFetcher::new("https://registry.example.test/root").unwrap();
        assert!(fetcher.fetch("../escape").is_err());
        assert!(fetcher.fetch("").is_err());
    }

    #[test]
    fn loads_and_preserves_scope_identity() {
        let module = Module::new();
        let scope = ModuleScope::from_bytes("demo", &module.write()).expect("load");
        assert_eq!(scope.name(), "demo");
    }

    #[test]
    fn indexes_module_exports_by_name() {
        let mut module = Module::new();
        let point = module.strings.intern("Point");
        let main = module.strings.intern("main");
        module.tables.add_type_def(zun_metadata::TypeDef {
            name: point,
            namespace: zun_metadata::StringIndex(0),
            base: Token::null(TableKind::TypeDef),
            field_start: 1,
            field_end: 1,
            method_start: 1,
            method_end: 1,
        });
        module.tables.add_method_def(zun_metadata::MethodDef {
            name: main,
            signature: zun_metadata::BlobIndex(0),
            body_offset: u32::MAX,
            is_static: false,
        });
        let scope = ModuleScope::from_bytes("demo", &module.write()).expect("load");
        assert!(matches!(scope.export("Point"), Some(Export::Type(_))));
        assert!(matches!(scope.export("main"), Some(Export::Method(_))));
        assert_eq!(scope.export("missing"), None);
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

    #[test]
    fn resolves_an_export_from_a_loaded_module() {
        let mut module = Module::new();
        let name = module.strings.intern("main");
        module.tables.add_method_def(zun_metadata::MethodDef {
            name,
            signature: zun_metadata::BlobIndex(0),
            body_offset: u32::MAX,
            is_static: false,
        });
        let mut loader = ModuleLoader::new();
        loader
            .insert(ModuleScope::from_bytes("app", &module.write()).expect("load"))
            .expect("register");
        let (scope, export) = loader.resolve_export("app", "main").expect("export");
        assert_eq!(scope.name(), "app");
        assert!(matches!(export, Export::Method(_)));
        assert!(loader.resolve_export("app", "missing").is_err());
    }

    #[test]
    fn rejects_duplicate_export_names() {
        let mut module = Module::new();
        let name = module.strings.intern("main");
        for _ in 0..2 {
            module.tables.add_method_def(zun_metadata::MethodDef {
                name,
                signature: zun_metadata::BlobIndex(0),
                body_offset: u32::MAX,
                is_static: false,
            });
        }
        assert!(matches!(
            ModuleScope::from_bytes("app", &module.write()),
            Err(LoadError::DuplicateExport(export)) if export == "main"
        ));
    }

    #[test]
    fn resolves_top_level_member_ref_through_module_ref() {
        let mut target = Module::new();
        let export_name = target.strings.intern("answer");
        target.tables.add_method_def(zun_metadata::MethodDef {
            name: export_name,
            signature: zun_metadata::BlobIndex(0),
            body_offset: u32::MAX,
            is_static: false,
        });

        let mut requester = Module::new();
        let module_name = requester.strings.intern("core");
        let member_name = requester.strings.intern("answer");
        let module_ref = requester
            .tables
            .add_module_ref(zun_metadata::ModuleRef { name: module_name });
        let member_ref = requester.tables.add_member_ref(zun_metadata::MemberRef {
            parent: module_ref,
            name: member_name,
            signature: zun_metadata::BlobIndex(0),
        });

        let target = ModuleScope::from_bytes("core", &target.write()).expect("target");
        let requester = ModuleScope::from_bytes("app", &requester.write()).expect("requester");
        let mut loader = ModuleLoader::new();
        loader.insert(target).expect("target registration");
        loader.insert(requester).expect("requester registration");
        let resolved = loader
            .resolve_member_ref(loader.get("app").unwrap(), member_ref)
            .expect("member ref");
        assert_eq!(resolved.scope.name(), "core");
        assert_eq!(resolved.kind, MemberKind::Method);
        assert_eq!(resolved.token.rid(), 1);
        let (scope, token) = loader
            .resolve_method_ref(loader.get("app").unwrap(), member_ref)
            .expect("method ref");
        assert_eq!(scope.name(), "core");
        assert_eq!(token.rid(), 1);
    }

    #[test]
    fn registry_rejects_duplicate_identities() {
        let bytes = Module::new().write();
        let mut loader = ModuleLoader::new();
        loader.load_bytes("core", &bytes).expect("first load");
        assert!(matches!(
            loader.load_bytes("core", &bytes),
            Err(LoadError::DuplicateModule(name)) if name == "core"
        ));
    }

    #[test]
    fn manifest_validates_entry_and_dependencies() {
        let bytes = Module::new().write();
        let mut loader = ModuleLoader::new();
        loader.load_bytes("core", &bytes).expect("core");
        loader.load_bytes("app", &bytes).expect("app");
        let manifest = ModuleManifest::new("app", vec!["core".into()]);
        assert!(manifest.validate(&loader).is_ok());
        let missing = ModuleManifest::new("app", vec!["missing".into()]);
        assert!(missing.validate(&loader).is_err());
        assert_eq!(
            manifest.load_order(&loader).expect("order"),
            vec!["core", "app"]
        );
        assert_eq!(
            ModuleManifest::read(&manifest.write()).expect("roundtrip"),
            manifest
        );
    }

    #[test]
    fn manifest_roundtrips_optional_native_object() {
        let manifest = ModuleManifest::new("app", vec!["core".into()])
            .with_native_object("app.o")
            .with_metadata_object("app.meta.o")
            .with_runtime_library("runtime.lib")
            .with_prelude_fingerprint(0x1234_5678_9abc_def0);
        let decoded = ModuleManifest::read(&manifest.write()).expect("roundtrip");
        assert_eq!(decoded, manifest);
        assert_eq!(decoded.prelude_fingerprint, Some(0x1234_5678_9abc_def0));
        decoded
            .validate_prelude_fingerprint(0x1234_5678_9abc_def0)
            .expect("matching prelude should validate");
        assert!(decoded.validate_prelude_fingerprint(0x42).is_err());
        assert_eq!(
            decoded.native_object_path("dist/app.zman"),
            Some(PathBuf::from("dist/app.o"))
        );
        assert_eq!(
            decoded.metadata_object_path("dist/app.zman"),
            Some(PathBuf::from("dist/app.meta.o"))
        );
        assert_eq!(
            decoded.runtime_library_path("dist/app.zman"),
            Some(PathBuf::from("dist/runtime.lib"))
        );
        assert_eq!(
            ModuleManifest::new("app", Vec::<String>::new()).native_object_path("app.zman"),
            None
        );
    }

    #[test]
    fn manifest_reads_version_zero_without_native_object() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(ModuleManifest::MAGIC);
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        write_name(&mut bytes, "app");
        let decoded = ModuleManifest::read(&bytes).expect("legacy manifest");
        assert_eq!(decoded, ModuleManifest::new("app", Vec::<String>::new()));
    }

    #[test]
    fn manifest_rejects_duplicate_or_self_dependencies() {
        let bytes = Module::new().write();
        let mut loader = ModuleLoader::new();
        loader.load_bytes("app", &bytes).expect("app");
        loader.load_bytes("x", &bytes).expect("x");
        assert!(matches!(
            ModuleManifest::new("app", vec!["app".into()]).validate(&loader),
            Err(LoadError::InvalidManifest(_))
        ));
        assert!(matches!(
            ModuleManifest::new("app", vec!["x".into(), "x".into()]).validate(&loader),
            Err(LoadError::InvalidManifest(_))
        ));
    }

    #[test]
    fn cache_aware_manifest_loading_rejects_a_stale_prelude_before_io() {
        let manifest = ModuleManifest::new("app", Vec::<String>::new()).with_prelude_fingerprint(7);
        let mut loader = ModuleLoader::new();
        let error = loader
            .load_manifest_with_prelude(&manifest, 8)
            .expect_err("stale prelude must be rejected");
        assert!(
            matches!(error, LoadError::InvalidManifest(detail) if detail.contains("fingerprint"))
        );
    }

    #[test]
    fn manifest_rejects_truncated_or_trailing_bytes() {
        let manifest = ModuleManifest::new("app", vec!["core".into()]);
        let bytes = manifest.write();
        assert!(ModuleManifest::read(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(ModuleManifest::read(&trailing).is_err());
    }

    #[test]
    fn manifest_file_roundtrip() {
        let manifest = ModuleManifest::new("app", vec!["core".into()]);
        let path = std::env::temp_dir().join(format!(
            "zun-manifest-{}-{}.zman",
            std::process::id(),
            manifest.dependencies.len()
        ));
        manifest.write_file(&path).expect("write manifest");
        let loaded = ModuleManifest::read_file(&path).expect("read manifest");
        std::fs::remove_file(&path).expect("remove manifest");
        assert_eq!(loaded, manifest);
    }

    #[test]
    fn loads_manifest_files_in_dependency_order() {
        let root = std::env::temp_dir().join(format!("zun-loader-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create temp directory");
        let dependency = root.join("core.zm");
        let entry = root.join("app.zm");
        std::fs::write(&dependency, Module::new().write()).expect("write dependency");
        std::fs::write(&entry, Module::new().write()).expect("write entry");
        let manifest = ModuleManifest::new(
            entry.to_string_lossy(),
            [dependency.to_string_lossy().into_owned()],
        );
        let mut loader = ModuleLoader::new();
        loader.load_manifest(&manifest).expect("load manifest");
        assert_eq!(loader.len(), 2);
        assert!(loader.get(&entry.to_string_lossy()).is_some());
        std::fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn rejects_manifest_with_unresolved_member_reference() {
        let mut module = Module::new();
        let missing = module.strings.intern("missing");
        let member = module.strings.intern("answer");
        let module_ref = module
            .tables
            .add_module_ref(zun_metadata::ModuleRef { name: missing });
        module.tables.add_member_ref(zun_metadata::MemberRef {
            parent: module_ref,
            name: member,
            signature: zun_metadata::BlobIndex(0),
        });
        let scope = ModuleScope::from_bytes("app", &module.write()).expect("load app");
        let mut loader = ModuleLoader::new();
        loader.insert(scope).expect("register app");
        let manifest = ModuleManifest::new("app", Vec::<String>::new());
        assert!(matches!(
            loader.validate_member_refs(&manifest),
            Err(LoadError::UnresolvedModule(name)) if name == "missing"
        ));
    }
}
