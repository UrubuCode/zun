use std::collections::HashMap;
use std::path::Path;

use zun_metadata::{TableKind, Token};
use zun_module::Module;

use crate::{
    content_id, ContentId, Export, LoadError, MemberKind, ModuleCache, ModuleFetcher,
    ModuleIndex, ModuleLock, ModuleManifest, ModuleScope, ResolvedMember, SignedModuleIndex,
    SignedModuleLock, TrustedKeySet, Zmlib,
};

/// Registry of loaded module scopes, keyed by their manifest identity.
#[derive(Debug, Default)]
pub struct ModuleLoader {
    modules: HashMap<String, ModuleScope>,
    trusted_keys: Option<TrustedKeySet>,
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

    /// Fetch, verify, persist, and register one module selected by a lockfile
    /// entry. The cache write happens only after the fetched bytes match the
    /// immutable content identity.
    pub fn load_fetched_cached(
        &mut self,
        cache_root: impl AsRef<Path>,
        name: &str,
        expected: ContentId,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let bytes = fetcher.fetch(name)?;
        if content_id(&bytes) != expected {
            return Err(LoadError::InvalidManifest(format!(
                "fetched module `{name}` has content identity mismatch"
            )));
        }
        let mut cache = ModuleCache::default();
        cache.store(cache_root, &bytes).map_err(|error| {
            LoadError::InvalidManifest(format!("could not cache module `{name}`: {error}"))
        })?;
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

    /// Fetch, cache, and register every executable module in lockfile order.
    pub fn load_lock_cached(
        &mut self,
        cache_root: impl AsRef<Path>,
        lock: &ModuleLock,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let cache_root = cache_root.as_ref();
        for (name, id) in &lock.entries {
            if name.ends_with(".d.ts") {
                continue;
            }
            self.load_fetched_cached(cache_root, name, *id, fetcher)?;
        }
        Ok(())
    }

    /// Verify a signed lock against a release-pinned key set before fetching
    /// any of its selected modules.
    pub fn load_signed_lock_with_keys(
        &mut self,
        signed: &SignedModuleLock,
        trusted: &TrustedKeySet,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        signed.verify_with_keys(trusted)?;
        self.load_lock(&signed.lock, fetcher)
    }

    /// Verify a signed lock against a release-pinned key set, then cache and
    /// register its modules only after the envelope has been authenticated.
    pub fn load_signed_lock_with_keys_cached(
        &mut self,
        cache_root: impl AsRef<Path>,
        signed: &SignedModuleLock,
        trusted: &TrustedKeySet,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        signed.verify_with_keys(trusted)?;
        self.load_lock_cached(cache_root, &signed.lock, fetcher)
    }

    /// Verify a signed lock using this loader's retained key set before
    /// fetching and registering its selected modules.
    pub fn load_signed_lock_trusted(
        &mut self,
        signed: &SignedModuleLock,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let trusted = self.trusted_keys.clone().ok_or_else(|| {
            LoadError::InvalidManifest("loader has no trusted registry key set".into())
        })?;
        self.load_signed_lock_with_keys(signed, &trusted, fetcher)
    }

    /// Verify a signed lock using this loader's retained key set, then cache
    /// and register its selected modules.
    pub fn load_signed_lock_trusted_cached(
        &mut self,
        cache_root: impl AsRef<Path>,
        signed: &SignedModuleLock,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let trusted = self.trusted_keys.clone().ok_or_else(|| {
            LoadError::InvalidManifest("loader has no trusted registry key set".into())
        })?;
        self.load_signed_lock_with_keys_cached(cache_root, signed, &trusted, fetcher)
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

    /// Fetch, cache, and register every module in deterministic index order.
    pub fn load_index_cached(
        &mut self,
        cache_root: impl AsRef<Path>,
        index: &ModuleIndex,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let cache_root = cache_root.as_ref();
        for (name, id) in &index.entries {
            self.load_fetched_cached(cache_root, name, *id, fetcher)?;
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

    /// Verify a signed index against the release root, then cache and load its
    /// selected modules only after the envelope has been authenticated.
    pub fn load_signed_index_cached(
        &mut self,
        cache_root: impl AsRef<Path>,
        signed: &SignedModuleIndex,
        trusted_key: &[u8; 32],
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        signed.verify_against(trusted_key)?;
        self.load_index_cached(cache_root, &signed.index, fetcher)
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

    /// Verify a signed index against rotated release keys, then cache and load
    /// its selected modules only after the envelope has been authenticated.
    pub fn load_signed_index_with_keys_cached(
        &mut self,
        cache_root: impl AsRef<Path>,
        signed: &SignedModuleIndex,
        trusted: &TrustedKeySet,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        signed.verify_with_keys(trusted)?;
        self.load_index_cached(cache_root, &signed.index, fetcher)
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

    /// Verify a signed index using this loader's retained key set, then cache
    /// and load its selected modules.
    pub fn load_signed_index_trusted_cached(
        &mut self,
        cache_root: impl AsRef<Path>,
        signed: &SignedModuleIndex,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let trusted = self.trusted_keys.as_ref().ok_or_else(|| {
            LoadError::InvalidManifest("loader has no trusted registry key set".into())
        })?;
        signed.verify_with_keys(trusted)?;
        self.load_index_cached(cache_root, &signed.index, fetcher)
    }

    /// Authenticate, load, and then install a signed index's successor key set.
    pub fn load_signed_index_trusted_with_rotation(
        &mut self,
        signed: &SignedModuleIndex,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let trusted = self.trusted_keys.clone().ok_or_else(|| {
            LoadError::InvalidManifest("loader has no trusted registry key set".into())
        })?;
        let next = signed.rotated_trusted_keys(&trusted)?;
        self.load_index(&signed.index, fetcher)?;
        self.trusted_keys = Some(next);
        Ok(())
    }

    /// Cached counterpart of [`Self::load_signed_index_trusted_with_rotation`].
    pub fn load_signed_index_trusted_cached_with_rotation(
        &mut self,
        cache_root: impl AsRef<Path>,
        signed: &SignedModuleIndex,
        fetcher: &impl ModuleFetcher,
    ) -> Result<(), LoadError> {
        let trusted = self.trusted_keys.clone().ok_or_else(|| {
            LoadError::InvalidManifest("loader has no trusted registry key set".into())
        })?;
        let next = signed.rotated_trusted_keys(&trusted)?;
        self.load_index_cached(cache_root, &signed.index, fetcher)?;
        self.trusted_keys = Some(next);
        Ok(())
    }

    /// Authenticate and install the successor key set carried by a signed
    /// index. The retained set is changed only after the transition verifies.
    pub fn apply_signed_index_rotation(
        &mut self,
        signed: &SignedModuleIndex,
    ) -> Result<(), LoadError> {
        let trusted = self.trusted_keys.as_ref().ok_or_else(|| {
            LoadError::InvalidManifest("loader has no trusted registry key set".into())
        })?;
        let next = signed.rotated_trusted_keys(trusted)?;
        self.trusted_keys = Some(next);
        Ok(())
    }

    /// Verify and register every module carried by a `.zmlib` package.
    pub fn load_zmlib(&mut self, package: &Zmlib) -> Result<(), LoadError> {
        let trusted = self.trusted_keys.as_ref().ok_or_else(|| {
            LoadError::InvalidManifest("loader has no trusted registry key set".into())
        })?;
        package.check_core_contract(zun_module::CORE_CONTRACT_VERSION)?;
        package.lock.verify_with_keys(trusted)?;
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
        package.check_core_contract(zun_module::CORE_CONTRACT_VERSION)?;
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
