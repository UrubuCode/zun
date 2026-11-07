use std::path::{Path, PathBuf};

use crate::{LoadError, ModuleIndex, SignedModuleIndex, TrustedKeySet};

/// Registry-facing byte fetch boundary. Network, authentication, and index
/// policy stay outside the loader and return canonical module bytes here.
pub trait ModuleFetcher {
    /// Fetch one module by registry identity.
    fn fetch(&self, name: &str) -> Result<Vec<u8>, LoadError>;
}

fn validate_registry_name(name: &str) -> Result<(), LoadError> {
    if name.is_empty()
        || name.contains(['\\', '?', '#', '%', ':'])
        || name.chars().any(char::is_control)
    {
        return Err(LoadError::InvalidManifest(format!(
            "unsafe registry module path `{name}`"
        )));
    }
    let path = Path::new(name);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(LoadError::InvalidManifest(format!(
            "unsafe registry module path `{name}`"
        )));
    }
    Ok(())
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
        validate_registry_name(name)?;
        let relative = Path::new(name);
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
        if !(base_url.starts_with("http://") || base_url.starts_with("https://"))
            || base_url.contains(['?', '#', '\\'])
            || base_url.chars().any(char::is_control)
        {
            return Err(LoadError::InvalidManifest(
                "HTTP registry base URL must use http:// or https:// without query, fragment, backslash, or control characters".into(),
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
    pub fn fetch_verified_index(&self, trusted_key: &[u8; 32]) -> Result<ModuleIndex, LoadError> {
        let signed = self.fetch_signed_index()?;
        signed.verify_against(trusted_key)?;
        Ok(signed.index)
    }

    /// Fetch the registry index and verify it against a rotated trusted set.
    pub fn fetch_verified_index_with_keys(
        &self,
        trusted: &TrustedKeySet,
    ) -> Result<ModuleIndex, LoadError> {
        let signed = self.fetch_signed_index()?;
        signed.verify_with_keys(trusted)?;
        Ok(signed.index)
    }
}

impl ModuleFetcher for HttpFetcher {
    fn fetch(&self, name: &str) -> Result<Vec<u8>, LoadError> {
        validate_registry_name(name)?;
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
