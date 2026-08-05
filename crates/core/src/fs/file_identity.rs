//! Stable identity for an existing filesystem object.

use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::Path;

use same_file::Handle;

use crate::error::ContextPatchError;
use crate::fs::hash::sha256_bytes;

/// An open handle whose equality follows the underlying filesystem object.
///
/// Keeping handles open is important on platforms where a reusable file identifier is only reliable
/// while the corresponding handle remains alive.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct FileIdentity {
    handle: Handle,
}

impl FileIdentity {
    pub fn from_path(path: &Path) -> Result<Self, ContextPatchError> {
        let handle = Handle::from_path(path).map_err(|error| {
            ContextPatchError::new(format!(
                "failed to identify target file {}: {error}",
                path.display()
            ))
        })?;
        Ok(Self { handle })
    }

    pub fn from_file(file: &File) -> Result<Self, ContextPatchError> {
        let file = file.try_clone().map_err(|error| {
            ContextPatchError::new(format!("failed to clone target file handle: {error}"))
        })?;
        let handle = Handle::from_file(file).map_err(|error| {
            ContextPatchError::new(format!("failed to identify open target file: {error}"))
        })?;
        Ok(Self { handle })
    }

    /// A deterministic digest suitable for a cross-process lock filename.
    pub(crate) fn lock_key(&self) -> String {
        let mut hasher = IdentityBytes::new();
        self.handle.hash(&mut hasher);
        sha256_bytes(&hasher.bytes)
    }
}

struct IdentityBytes {
    bytes: Vec<u8>,
}

impl IdentityBytes {
    fn new() -> Self {
        Self {
            bytes: b"contextpatch-file-identity-v1".to_vec(),
        }
    }
}

impl Hasher for IdentityBytes {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.bytes
            .extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        self.bytes.extend_from_slice(bytes);
    }
}
