use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::ContextPatchError;

pub fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> Result<String, ContextPatchError> {
    let mut file = File::open(path).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to open {} for hashing: {error}",
            path.display()
        ))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            ContextPatchError::new(format!("failed to hash {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn validate_sha256(value: &str) -> Result<(), ContextPatchError> {
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(ContextPatchError::new(
            "expected_sha256 must be 64 hexadecimal characters",
        ));
    }
    if value.chars().any(|ch| ch.is_ascii_uppercase()) {
        return Err(ContextPatchError::new(
            "expected_sha256 must use lowercase hexadecimal characters",
        ));
    }
    Ok(())
}
