use std::collections::{HashMap, HashSet};
use std::path::Path;

use zun_metadata::{TableKind, Token};
use zun_module::Module;
use zun_types::{CoreTypes, TypeSystem};

use crate::LoadError;

/// A loaded module together with its loader-visible identity.
#[derive(Debug)]
pub struct ModuleScope {
    name: String,
    pub(crate) module: Module,
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
        // Collect every method RID owned by a type definition once, so the
        // per-method ownership test below is O(1) instead of scanning types.
        let mut type_methods = HashSet::new();
        for ty in &module.tables.type_defs {
            for rid in ty.method_start..ty.method_end {
                type_methods.insert(rid);
            }
        }
        for (rid, method) in module.tables.method_defs.iter().enumerate() {
            let method_rid = rid as u32 + 1;
            let is_type_method = type_methods.contains(&method_rid);
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

    /// Iterate every exported definition by its source-level name.
    pub fn exports(&self) -> impl Iterator<Item = (&str, Export)> + '_ {
        self.exports
            .iter()
            .map(|(name, export)| (name.as_str(), *export))
    }

    /// Consume the scope and return its decoded module.
    pub fn into_module(self) -> Module {
        self.module
    }
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
