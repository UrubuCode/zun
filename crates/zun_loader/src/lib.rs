//! Module loading and scope identity for Zun binaries.
//!
//! Loading is deliberately separate from verification and execution. A
//! [`ModuleScope`] owns the bytes decoded from one `.zm` module and gives the
//! future resolver a stable scope boundary for type and member identity.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use zun_metadata::{TableKind, Token};
use zun_module::{Module, ReadError};
use zun_types::{CoreTypes, TypeError, TypeSystem};

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
    /// Fingerprint of the prelude embedded or linked by the compiled modules.
    /// A missing value denotes a legacy or manually assembled manifest.
    pub prelude_fingerprint: Option<u64>,
}

impl ModuleManifest {
    const MAGIC: &'static [u8; 4] = b"ZMAN";
    const VERSION: u16 = 2;

    /// Construct a manifest for an entry module and its dependencies.
    pub fn new(entry: impl Into<String>, dependencies: impl IntoIterator<Item = String>) -> Self {
        Self {
            entry: entry.into(),
            dependencies: dependencies.into_iter().collect(),
            native_object: None,
            prelude_fingerprint: None,
        }
    }

    /// Associate an optional R2R/AOT object path with this manifest.
    pub fn with_native_object(mut self, path: impl Into<String>) -> Self {
        self.native_object = Some(path.into());
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
                _ => return Err(LoadError::InvalidManifest("invalid native object flag".into())),
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
                _ => return Err(LoadError::InvalidManifest("invalid prelude fingerprint flag".into())),
            }
        };
        if cursor != bytes.len() {
            return Err(LoadError::InvalidManifest("trailing manifest bytes".into()));
        }
        Ok(Self {
            entry,
            dependencies,
            native_object,
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

    /// Load and register a module under `name`.
    pub fn load_bytes(&mut self, name: impl Into<String>, bytes: &[u8]) -> Result<(), LoadError> {
        let name = name.into();
        let scope = ModuleScope::from_bytes(name.clone(), bytes)?;
        self.insert(scope)
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
        let manifest = ModuleManifest::new("app", Vec::<String>::new())
            .with_prelude_fingerprint(7);
        let mut loader = ModuleLoader::new();
        let error = loader
            .load_manifest_with_prelude(&manifest, 8)
            .expect_err("stale prelude must be rejected");
        assert!(matches!(error, LoadError::InvalidManifest(detail) if detail.contains("fingerprint")));
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
