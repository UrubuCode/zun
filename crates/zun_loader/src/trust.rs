use crate::LoadError;

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

    pub(crate) fn contains(&self, key: &[u8; 32]) -> bool {
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
