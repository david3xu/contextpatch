//! Cooperative cross-process locks for ContextPatch mutations.

use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::ContextPatchError;
use crate::fs::hash::sha256_bytes;
use crate::fs::scratch::ensure_scratch_root;

const LOCK_DIRECTORY: &str = "mutation-locks";
const REPOSITORY_LOCK: &str = "repository.lock";

#[derive(Debug)]
pub struct MutationLockGuard {
    _file: File,
}

fn lock_directory(repo_root: &Path) -> Result<PathBuf, ContextPatchError> {
    let directory = ensure_scratch_root(repo_root)?.join(LOCK_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|error| {
        ContextPatchError::new(format!(
            "could not create mutation lock directory `{}`: {error}",
            directory.display()
        ))
    })?;
    Ok(directory)
}

fn try_lock(path: &Path, contention_message: &str) -> Result<MutationLockGuard, ContextPatchError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            ContextPatchError::new(format!(
                "could not open mutation lock `{}`: {error}",
                path.display()
            ))
        })?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(MutationLockGuard { _file: file }),
        Err(error) if error.kind() == ErrorKind::WouldBlock => {
            Err(ContextPatchError::new(contention_message))
        }
        Err(error) => Err(ContextPatchError::new(format!(
            "could not acquire mutation lock `{}`: {error}",
            path.display()
        ))),
    }
}

pub fn try_repository_mutation_lock(
    repo_root: &Path,
) -> Result<MutationLockGuard, ContextPatchError> {
    try_lock(
        &lock_directory(repo_root)?.join(REPOSITORY_LOCK),
        "another ContextPatch mutation is still active for this repository; this operation was \
         not started. Wait for it to finish, then inspect read_write_receipts and status_guard \
         before retrying",
    )
}

pub fn try_file_mutation_lock(
    repo_root: &Path,
    target: &Path,
) -> Result<MutationLockGuard, ContextPatchError> {
    let key = sha256_bytes(target.as_os_str().as_encoded_bytes());
    let path = lock_directory(repo_root)?.join(format!("{key}.lock"));
    try_lock(
        &path,
        &format!(
            "another ContextPatch mutation is still active for `{}`; this edit was not started",
            target.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-mutation-lock-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn repository_lock_refuses_overlapping_mutations_without_waiting() {
        let root = temp_repo("repository");
        let first = try_repository_mutation_lock(&root).unwrap();
        let error = try_repository_mutation_lock(&root).unwrap_err();
        assert!(error.to_string().contains("still active"));
        drop(first);
        assert!(try_repository_mutation_lock(&root).is_ok());
    }

    #[test]
    fn file_locks_are_scoped_to_the_target() {
        let root = temp_repo("file");
        let first_path = root.join("first.txt");
        let second_path = root.join("second.txt");
        let first = try_file_mutation_lock(&root, &first_path).unwrap();
        assert!(try_file_mutation_lock(&root, &first_path).is_err());
        assert!(try_file_mutation_lock(&root, &second_path).is_ok());
        drop(first);
        assert!(try_file_mutation_lock(&root, &first_path).is_ok());
    }
}
