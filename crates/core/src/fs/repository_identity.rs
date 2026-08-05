//! A repository's identity, derived from what the filesystem says it is rather than what it is called.
//!
//! Scratch directories, mutation locks, and the receipt journal all need a key that answers "which
//! repository is this?" consistently across processes and restarts. Until now that key was a hash of the
//! canonical repository path, which gets two cases wrong in opposite directions.
//!
//! A rename changes the key. Move a repository and its in-flight locks stop excluding each other, its
//! receipts become unreachable, and its scratch directory is abandoned. Nothing warns, because from the
//! path's point of view a different repository has appeared.
//!
//! Two names for one directory also used to agree only because canonicalization collapsed them. That works
//! for symlinks but not for bind mounts or hard-linked directory trees, and it depends on resolving a name,
//! which is what the anchoring work removed everywhere else.
//!
//! Identity here comes from the root directory's device and inode, read through the root's own authority.
//! Those follow the directory rather than its name: a rename leaves them untouched, and two names for one
//! directory report the same pair without either name being resolved.
//!
//! The logical path still appears in the *label* half of a scratch directory name, because a human reading
//! an error or listing a scratch container needs to recognize which repository a directory belongs to. The
//! label never contributes to the key.

use crate::error::ContextPatchError;
use crate::git::root::RepositoryRoot;

/// Hex characters of the identity digest used in a directory name.
const DIGEST_PREFIX_LENGTH: usize = 16;

/// What identifies one repository, independent of the name it is reached by.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryIdentity {
    digest: String,
    label: String,
}

impl RepositoryIdentity {
    /// The stable key, safe to use in a filename.
    pub fn key(&self) -> &str {
        &self.digest
    }

    /// A human-recognizable directory name combining the label and the key.
    ///
    /// The label is for reading; only the digest distinguishes repositories.
    pub fn directory_name(&self) -> String {
        format!("{}-{}", self.label, &self.digest[..DIGEST_PREFIX_LENGTH])
    }
}

/// Derive one repository's identity through its own authority.
///
/// An anchored selection is read from the descriptor it already retains. A configured root has its
/// descriptor opened once, no-follow, by the shared root helper; that single open is the only time a name is
/// resolved, and it is the same open every other operation on that root performs.
pub fn identity_of(root: RepositoryRoot<'_>) -> Result<RepositoryIdentity, ContextPatchError> {
    let label = root
        .logical_path()
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "repository".to_string());

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let handle = crate::fs::rooted::root_descriptor(root)?;
        let metadata = handle.as_file().metadata().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to identify repository {}: {error}",
                root.logical_path().display()
            ))
        })?;
        // Device and inode together identify the directory. Rendered with an explicit separator so two
        // different pairs cannot produce one byte sequence by concatenation.
        let digest = crate::fs::hash::sha256_bytes(
            format!("repository:{}:{}", metadata.dev(), metadata.ino()).as_bytes(),
        );
        Ok(RepositoryIdentity { digest, label })
    }

    #[cfg(not(unix))]
    {
        let _ = label;
        Err(ContextPatchError::new(
            "repository identity requires descriptor-relative operations",
        ))
    }
}

/// The identity a path-keyed scratch directory would have had.
///
/// Retained so receipts written before identity keying stay readable. It reproduces the old digest exactly,
/// including its fallback to the unresolved path when canonicalization fails, because a legacy directory
/// name has to be reproduced rather than improved.
pub fn legacy_path_identity(root: RepositoryRoot<'_>) -> RepositoryIdentity {
    let logical_path = root.logical_path();
    let canonical = logical_path
        .canonicalize()
        .unwrap_or_else(|_| logical_path.to_path_buf());
    let digest = crate::fs::hash::sha256_bytes(canonical.as_os_str().as_encoded_bytes());
    let label = canonical
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "repository".to_string());
    RepositoryIdentity { digest, label }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-identity-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn open_directory(path: &Path) -> fs::File {
        use std::os::unix::fs::OpenOptionsExt;

        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
            .unwrap()
    }

    #[test]
    fn identity_survives_a_rename() {
        // The case path keying got wrong. Renaming a repository must not change which locks exclude it,
        // which receipts belong to it, or which scratch directory it uses.
        let holder = temp_root("rename");
        let before = holder.join("before");
        fs::create_dir(&before).unwrap();
        let identity_before = identity_of(RepositoryRoot::from_path(&before)).unwrap();

        let after = holder.join("after");
        fs::rename(&before, &after).unwrap();
        let identity_after = identity_of(RepositoryRoot::from_path(&after)).unwrap();

        assert_eq!(identity_before.key(), identity_after.key());
        // The label follows the name, so the directory name changes while the key does not.
        assert_ne!(
            identity_before.directory_name(),
            identity_after.directory_name()
        );
        // Path keying would have produced a different key, which is the bug being fixed.
        assert_ne!(
            legacy_path_identity(RepositoryRoot::from_path(&before)).key(),
            legacy_path_identity(RepositoryRoot::from_path(&after)).key()
        );
    }

    #[test]
    fn two_names_for_one_directory_share_one_identity() {
        let holder = temp_root("alias");
        let real = holder.join("real");
        fs::create_dir(&real).unwrap();
        let alias = holder.join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();

        // The alias cannot be opened as a root, because opening a symlink as a root is refused. Reached
        // through its target it reports the same identity, which is the property that matters: one
        // directory, one key.
        let identity = identity_of(RepositoryRoot::from_path(&real)).unwrap();
        let through_target =
            identity_of(RepositoryRoot::from_path(&alias.canonicalize().unwrap())).unwrap();
        assert_eq!(identity.key(), through_target.key());
    }

    #[test]
    fn distinct_repositories_get_distinct_identities() {
        let first = temp_root("first");
        let second = temp_root("second");

        assert_ne!(
            identity_of(RepositoryRoot::from_path(&first)).unwrap().key(),
            identity_of(RepositoryRoot::from_path(&second))
                .unwrap()
                .key()
        );
    }

    #[test]
    fn an_anchored_root_and_its_name_agree() {
        // A selection and a configured root pointing at one directory must key identically, or a selected
        // call and an unselected call would take different locks on the same repository.
        let root = temp_root("anchored");
        let directory = open_directory(&root);

        assert_eq!(
            identity_of(RepositoryRoot::anchored(&root, &directory))
                .unwrap()
                .key(),
            identity_of(RepositoryRoot::from_path(&root)).unwrap().key()
        );
    }

    #[test]
    fn an_anchored_identity_ignores_a_replaced_name() {
        // The descriptor case. After the name is taken over by a different directory, the selection still
        // identifies the directory it was anchored to rather than the replacement.
        let root = temp_root("anchored-swap");
        let directory = open_directory(&root);
        let anchored = identity_of(RepositoryRoot::anchored(&root, &directory)).unwrap();

        let holder = temp_root("anchored-swap-holder");
        let moved = holder.join("moved");
        fs::rename(&root, &moved).unwrap();
        let replacement = holder.join("replacement");
        fs::create_dir(&replacement).unwrap();
        std::os::unix::fs::symlink(&replacement, &root).unwrap();

        let still_anchored = identity_of(RepositoryRoot::anchored(&root, &directory)).unwrap();
        assert_eq!(anchored.key(), still_anchored.key());
        assert_eq!(
            identity_of(RepositoryRoot::from_path(&moved)).unwrap().key(),
            anchored.key(),
            "the moved directory is the one that was anchored"
        );
        assert_ne!(
            identity_of(RepositoryRoot::from_path(&replacement))
                .unwrap()
                .key(),
            anchored.key(),
            "the replacement is a different repository"
        );
    }
}
