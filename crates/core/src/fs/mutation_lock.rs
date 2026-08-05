//! Cooperative cross-process locks for ContextPatch mutations.

use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::ContextPatchError;
use crate::fs::file_identity::FileIdentity;
use crate::fs::scratch::{ensure_scratch_root, scratch_root};

const LOCK_DIRECTORY: &str = "mutation-locks";
const REPOSITORY_LOCK: &str = "repository.lock";

#[derive(Debug)]
pub struct MutationLockGuard {
    _file: File,
    _identity: Option<FileIdentity>,
}

impl Drop for MutationLockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._file);
    }
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

fn file_lock_directory(repo_root: &Path) -> Result<PathBuf, ContextPatchError> {
    let repository_scratch = scratch_root(repo_root)?;
    let scratch_container = repository_scratch.parent().ok_or_else(|| {
        ContextPatchError::new(format!(
            "could not determine shared lock directory from `{}`",
            repository_scratch.display()
        ))
    })?;
    let directory = scratch_container.join("file-mutation-locks");
    fs::create_dir_all(&directory).map_err(|error| {
        ContextPatchError::new(format!(
            "could not create shared file mutation lock directory `{}`: {error}",
            directory.display()
        ))
    })?;
    Ok(directory)
}

fn try_lock(
    path: &Path,
    contention_message: &str,
    identity: Option<FileIdentity>,
) -> Result<MutationLockGuard, ContextPatchError> {
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
        Ok(()) => Ok(MutationLockGuard {
            _file: file,
            _identity: identity,
        }),
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
        None,
    )
}

pub fn try_file_mutation_lock(
    repo_root: &Path,
    target: &Path,
) -> Result<MutationLockGuard, ContextPatchError> {
    let identity = FileIdentity::from_path(target)?;
    try_file_mutation_lock_for_identity(repo_root, target, identity)
}

pub fn try_file_mutation_lock_for_open_file(
    repo_root: &Path,
    target: &Path,
    file: &File,
) -> Result<MutationLockGuard, ContextPatchError> {
    let identity = FileIdentity::from_file(file)?;
    try_file_mutation_lock_for_identity(repo_root, target, identity)
}

fn try_file_mutation_lock_for_identity(
    repo_root: &Path,
    target: &Path,
    identity: FileIdentity,
) -> Result<MutationLockGuard, ContextPatchError> {
    let key = identity.lock_key();
    let path = file_lock_directory(repo_root)?.join(format!("{key}.lock"));
    try_lock(
        &path,
        &format!(
            "another ContextPatch mutation is still active for `{}`; this edit was not started",
            target.display()
        ),
        Some(identity),
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
        fs::write(&first_path, "first").unwrap();
        fs::write(&second_path, "second").unwrap();
        let first = try_file_mutation_lock(&root, &first_path).unwrap();
        assert!(try_file_mutation_lock(&root, &first_path).is_err());
        assert!(try_file_mutation_lock(&root, &second_path).is_ok());
        drop(first);
        try_file_mutation_lock(&root, &first_path).unwrap();
    }

    #[test]
    fn hard_link_aliases_share_one_file_lock() {
        let root = temp_repo("hard-link");
        let original = root.join("original.txt");
        let alias = root.join("alias.txt");
        fs::write(&original, "same file").unwrap();
        fs::hard_link(&original, &alias).unwrap();

        let first = try_file_mutation_lock(&root, &original).unwrap();
        let error = try_file_mutation_lock(&root, &alias).unwrap_err();

        assert!(error.to_string().contains("still active"));
        drop(first);
        try_file_mutation_lock(&root, &alias).unwrap();
    }

    #[test]
    fn parent_and_descendant_repository_views_share_one_file_lock() {
        let root = temp_repo("nested-repository");
        let descendant = root.join("project");
        fs::create_dir(&descendant).unwrap();
        let target = descendant.join("shared.txt");
        fs::write(&target, "same file").unwrap();

        let first = try_file_mutation_lock(&root, &target).unwrap();
        let error = try_file_mutation_lock(&descendant, &target).unwrap_err();

        assert!(error.to_string().contains("still active"));
        drop(first);
        assert!(try_file_mutation_lock(&descendant, &target).is_ok());
    }

    #[test]
    fn case_aliases_share_one_file_lock_when_supported() {
        let root = temp_repo("case-alias");
        let mixed_case = root.join("CaseAlias.txt");
        let lower_case = root.join("casealias.txt");
        fs::write(&mixed_case, "same file").unwrap();
        if !lower_case.exists() {
            return;
        }

        let _first = try_file_mutation_lock(&root, &mixed_case).unwrap();
        let error = try_file_mutation_lock(&root, &lower_case).unwrap_err();

        assert!(error.to_string().contains("still active"));
    }
}
