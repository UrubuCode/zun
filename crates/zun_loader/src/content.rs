use std::collections::HashMap;
use std::path::Path;

use sha2::{Digest, Sha256};

/// A deterministic content identifier for a serialized module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentId(pub [u8; 32]);
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
        let needs_write = match std::fs::read(&path) {
            Ok(existing) => content_id(&existing) != id,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(CacheError::Io(error)),
        };
        if needs_write {
            let temporary = root.join(format!(".{}.zm.tmp", id.hex()));
            use std::io::Write;
            let mut file = match std::fs::File::create(&temporary) {
                Ok(file) => file,
                Err(error) => {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(CacheError::Io(error));
                }
            };
            if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                let _ = std::fs::remove_file(&temporary);
                return Err(CacheError::Io(error));
            }
            match std::fs::rename(&temporary, &path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    std::fs::remove_file(&path)?;
                    if let Err(error) = std::fs::rename(&temporary, &path) {
                        let _ = std::fs::remove_file(&temporary);
                        return Err(CacheError::Io(error));
                    }
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(CacheError::Io(error));
                }
            }
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
