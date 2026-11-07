use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::bytes::{read_name, read_u16, read_u32, take, write_name};
use crate::{ContentId, LoadError, TrustedKeySet};

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
    /// Keys authorized by this authenticated index for the next release.
    pub key_additions: Vec<[u8; 32]>,
    /// Keys revoked by this authenticated index for the next release.
    pub key_revocations: Vec<[u8; 32]>,
}

impl SignedModuleIndex {
    const MAGIC: &'static [u8; 4] = b"ZISG";
    const VERSION: u16 = 2;

    pub fn new(
        index: ModuleIndex,
        public_key: [u8; 32],
        signature: [u8; 64],
    ) -> Result<Self, LoadError> {
        let signed = Self {
            index,
            public_key,
            signature,
            key_additions: Vec::new(),
            key_revocations: Vec::new(),
        };
        signed.verify_signature()?;
        Ok(signed)
    }

    pub fn new_with_rotation(
        index: ModuleIndex,
        public_key: [u8; 32],
        key_additions: impl IntoIterator<Item = [u8; 32]>,
        key_revocations: impl IntoIterator<Item = [u8; 32]>,
        signature: [u8; 64],
    ) -> Result<Self, LoadError> {
        let signed = Self {
            index,
            public_key,
            signature,
            key_additions: key_additions.into_iter().collect(),
            key_revocations: key_revocations.into_iter().collect(),
        };
        signed.verify_signature()?;
        Ok(signed)
    }

    /// Return the canonical bytes an index publisher must sign.
    pub fn signing_bytes(
        index: &ModuleIndex,
        key_additions: &[[u8; 32]],
        key_revocations: &[[u8; 32]],
    ) -> Vec<u8> {
        let index_payload = index.write();
        if key_additions.is_empty() && key_revocations.is_empty() {
            return index_payload;
        }
        let mut payload = Vec::with_capacity(4 + index_payload.len() + 12);
        payload.extend_from_slice(&(index_payload.len() as u32).to_le_bytes());
        payload.extend_from_slice(&index_payload);
        payload.extend_from_slice(b"ZROT");
        payload.extend_from_slice(&(key_additions.len() as u32).to_le_bytes());
        for key in key_additions {
            payload.extend_from_slice(key);
        }
        payload.extend_from_slice(&(key_revocations.len() as u32).to_le_bytes());
        for key in key_revocations {
            payload.extend_from_slice(key);
        }
        payload
    }

    fn signed_payload(&self) -> Vec<u8> {
        Self::signing_bytes(&self.index, &self.key_additions, &self.key_revocations)
    }

    fn validate_rotation(&self) -> Result<(), LoadError> {
        if self
            .key_additions
            .iter()
            .enumerate()
            .any(|(index, key)| self.key_additions[..index].contains(key))
            || self
                .key_revocations
                .iter()
                .enumerate()
                .any(|(index, key)| self.key_revocations[..index].contains(key))
            || self
                .key_additions
                .iter()
                .any(|key| self.key_revocations.contains(key))
        {
            return Err(LoadError::InvalidManifest(
                "signed index contains a contradictory or duplicate key rotation".into(),
            ));
        }
        Ok(())
    }

    fn verify_signature(&self) -> Result<(), LoadError> {
        self.validate_rotation()?;
        let key = VerifyingKey::from_bytes(&self.public_key).map_err(|error| {
            LoadError::InvalidManifest(format!("invalid index public key: {error}"))
        })?;
        let signature = Signature::from_bytes(&self.signature);
        key.verify(&self.signed_payload(), &signature)
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

    /// Authenticate this index and derive the next trusted key set.
    pub fn rotated_trusted_keys(
        &self,
        trusted: &TrustedKeySet,
    ) -> Result<TrustedKeySet, LoadError> {
        self.verify_with_keys(trusted)?;
        trusted.rotate(self.key_additions.clone(), self.key_revocations.clone())
    }

    pub fn write(&self) -> Vec<u8> {
        let payload = self.signed_payload();
        let mut bytes = Vec::with_capacity(106 + payload.len());
        bytes.extend_from_slice(Self::MAGIC);
        let version = if self.key_additions.is_empty() && self.key_revocations.is_empty() {
            1
        } else {
            Self::VERSION
        };
        bytes.extend_from_slice(&version.to_le_bytes());
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
        if version > Self::VERSION || version == 0 {
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
        if version == 1 {
            return Self::new(ModuleIndex::read(&bytes[106..end])?, public_key, signature);
        }
        let payload = &bytes[106..end];
        let mut prefix_cursor = 0;
        let index_len = read_u32(payload, &mut prefix_cursor)
            .ok_or_else(|| LoadError::InvalidManifest("truncated rotated index".into()))?
            as usize;
        let index_bytes = take(payload, &mut prefix_cursor, index_len)
            .ok_or_else(|| LoadError::InvalidManifest("truncated rotated index".into()))?;
        let index = ModuleIndex::read(index_bytes)?;
        if take(payload, &mut prefix_cursor, 4) != Some(b"ZROT") {
            return Err(LoadError::InvalidManifest(
                "missing signed key rotation".into(),
            ));
        }
        let mut cursor = prefix_cursor;
        let additions = read_u32(payload, &mut cursor)
            .ok_or_else(|| LoadError::InvalidManifest("truncated key additions".into()))?
            as usize;
        let mut key_additions = Vec::with_capacity(additions);
        for _ in 0..additions {
            let raw = take(payload, &mut cursor, 32)
                .ok_or_else(|| LoadError::InvalidManifest("truncated key addition".into()))?;
            key_additions.push(raw.try_into().unwrap());
        }
        let revocations = read_u32(payload, &mut cursor)
            .ok_or_else(|| LoadError::InvalidManifest("truncated key revocations".into()))?
            as usize;
        let mut key_revocations = Vec::with_capacity(revocations);
        for _ in 0..revocations {
            let raw = take(payload, &mut cursor, 32)
                .ok_or_else(|| LoadError::InvalidManifest("truncated key revocation".into()))?;
            key_revocations.push(raw.try_into().unwrap());
        }
        if cursor != payload.len() {
            return Err(LoadError::InvalidManifest(
                "trailing key rotation bytes".into(),
            ));
        }
        let signed = Self {
            index,
            public_key,
            signature,
            key_additions,
            key_revocations,
        };
        signed.verify_signature()?;
        Ok(signed)
    }
}
