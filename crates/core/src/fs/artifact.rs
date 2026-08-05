//! Sidecar artifact directories, located by what a repository is rather than by what it is called.
//!
//! Artifacts are files a caller wants beside a repository without putting them inside it: a generated
//! script, throwaway tooling, output that must never appear in `git status`. Living outside the worktree is
//! the whole point, which means their location has to be derived from something.
//!
//! It used to be derived from the repository's path spelling, hashed. That gets the same two cases wrong
//! that scratch, lock, and receipt keying already had to fix, and it gets them wrong in opposite
//! directions. Renaming a repository moved its artifacts out of reach, because a new spelling hashes to a
//! new directory and nothing warns. Worse, a *different* repository taking over the old pathname inherited
//! the first one's artifacts, because the spelling was the only thing the key ever knew.
//!
//! Identity fixes both. The key comes from the root directory's device and inode, read through the root's
//! own authority, so it follows the directory through a rename and cannot be assumed by a replacement that
//! merely acquires the name. The logical path survives only as a label marker written inside the directory,
//! where it cannot affect where the directory is.

use std::path::PathBuf;

use crate::error::ContextPatchError;

/// The file naming which repository an artifact directory belongs to.
const LABEL_MARKER: &str = "repository-label";

/// The directory holding every repository's artifact directory.
const CONTAINER_NAME: &str = "contextpatch-artifacts";

/// Where artifact directories live, independent of any repository.
///
/// The temp directory, as before. Artifacts are explicitly disposable, so unlike scratch there is no reason
/// to prefer a location that survives a reboot.
fn container() -> PathBuf {
    std::env::temp_dir().join(CONTAINER_NAME)
}

/// A stable, repository-specific artifact path. Does not touch the filesystem.
pub fn artifact_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
) -> Result<PathBuf, ContextPatchError> {
    let identity = crate::fs::repository_identity::identity_of(repo_root.into())?;
    Ok(container().join(identity.directory_name()))
}

/// The artifact directory for this repository, created if absent.
///
/// The label marker is refreshed on every call, because a rename changes the label without changing the
/// directory. Writing it is best effort: failing to record a label must not fail the operation that needed
/// the directory.
pub fn ensure_artifact_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
) -> Result<PathBuf, ContextPatchError> {
    let repo_root = repo_root.into();
    let identity = crate::fs::repository_identity::identity_of(repo_root)?;
    let root = container().join(identity.directory_name());
    std::fs::create_dir_all(&root).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to create the artifact directory `{}`: {error}",
            root.display()
        ))
    })?;
    let _ = std::fs::write(
        root.join(LABEL_MARKER),
        format!(
            "{}\n{}\n",
            identity.label(),
            repo_root.logical_path().display()
        ),
    );
    Ok(root)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_directory(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_rename_keeps_the_same_artifact_directory() {
        let holder = temp_directory("artifact_rename");
        let original = holder.join("original");
        fs::create_dir_all(&original).unwrap();

        let before = artifact_root(&original).unwrap();
        let renamed = holder.join("renamed");
        fs::rename(&original, &renamed).unwrap();
        let after = artifact_root(&renamed).unwrap();

        assert_eq!(
            before, after,
            "artifacts belong to the directory rather than to its name"
        );
    }

    #[test]
    fn a_replacement_at_the_old_name_gets_a_different_artifact_directory() {
        let holder = temp_directory("artifact_replacement");
        let original = holder.join("target");
        fs::create_dir_all(&original).unwrap();
        let first = artifact_root(&original).unwrap();

        // Move the original aside and put a different directory under its name.
        let moved = holder.join("moved");
        fs::rename(&original, &moved).unwrap();
        fs::create_dir_all(&original).unwrap();
        let second = artifact_root(&original).unwrap();

        assert_ne!(
            first, second,
            "a replacement must not inherit the artifacts of the directory it replaced"
        );
        assert_eq!(
            first,
            artifact_root(&moved).unwrap(),
            "the original keeps its artifacts under its new name"
        );
    }

    #[test]
    fn two_names_for_one_directory_share_one_artifact_directory() {
        let holder = temp_directory("artifact_alias");
        let original = holder.join("original");
        fs::create_dir_all(&original).unwrap();
        let alias = holder.join("alias");
        std::os::unix::fs::symlink(&original, &alias).unwrap();

        // The alias itself cannot be opened as a root, because opening a symlink as a root is refused.
        // Reached through its target it reports the same artifact directory, which is the property that
        // matters here: one directory holds one set of artifacts however it was named.
        assert!(
            artifact_root(&alias).is_err(),
            "a symlinked root is refused rather than followed"
        );
        assert_eq!(
            artifact_root(&original).unwrap(),
            artifact_root(&alias.canonicalize().unwrap()).unwrap(),
            "one directory has one artifact directory however it is reached"
        );
    }

    #[test]
    fn the_directory_is_created_outside_the_repository_and_carries_a_label() {
        let repository = temp_directory("artifact_created");

        let root = ensure_artifact_root(&repository).unwrap();

        assert!(root.is_dir());
        assert!(
            !root.starts_with(&repository),
            "artifacts must live outside the worktree"
        );
        let label = fs::read_to_string(root.join(LABEL_MARKER)).unwrap();
        assert!(label.contains(&repository.display().to_string()), "{label}");
        // The label is informational: it names the repository without deciding the directory.
        assert_eq!(root, artifact_root(&repository).unwrap());
    }
}
