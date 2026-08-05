use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::error::ContextPatchError;
use crate::fs::hash::{sha256_file, validate_sha256};
use crate::git::guarded_path::{
    ensure_path_clean, ensure_tracked, exact_worktree_root, path_has_unstaged_deletion,
    path_is_tracked, resolve_existing_regular_file,
};

pub const CONFIRMATION: &str = "delete tracked file";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DeleteGuardedResult {
    pub path: String,
    pub sha256: String,
    pub dry_run: bool,
    pub deleted: bool,
}

pub fn delete_guarded(
    repo_root: &Path,
    path: &Path,
    expected_sha256: &str,
    dry_run: bool,
    confirmation: Option<&str>,
) -> Result<DeleteGuardedResult, ContextPatchError> {
    validate_sha256(expected_sha256)?;
    if !dry_run && confirmation != Some(CONFIRMATION) {
        return Err(ContextPatchError::new(format!(
            "dry_run=false requires confirm: {CONFIRMATION:?}"
        )));
    }

    let root = exact_worktree_root(repo_root)?;
    let (path, target) = resolve_existing_regular_file(&root, path)?;
    ensure_tracked(&root, &path, "file")?;
    ensure_path_clean(&root, &path)?;
    let current_sha256 = sha256_file(&target)?;
    if current_sha256 != expected_sha256 {
        return Err(ContextPatchError::new(format!(
            "hash mismatch for `{path}`; current_sha256={current_sha256}, expected_sha256={expected_sha256}"
        )));
    }

    if dry_run {
        return Ok(DeleteGuardedResult {
            path,
            sha256: current_sha256,
            dry_run: true,
            deleted: false,
        });
    }

    let verified_sha256 = sha256_file(&target)?;
    if verified_sha256 != expected_sha256 {
        return Err(ContextPatchError::new(format!(
            "file `{path}` changed during validation; current_sha256={verified_sha256}, expected_sha256={expected_sha256}"
        )));
    }
    fs::remove_file(&target).map_err(|error| {
        ContextPatchError::new(format!("failed to delete tracked file `{path}`: {error}"))
    })?;

    match fs::symlink_metadata(&target) {
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(ContextPatchError::new(format!(
                "delete verification failed: path `{path}` still exists"
            )))
        }
        Err(error) => {
            return Err(ContextPatchError::new(format!(
                "delete verification failed while inspecting `{path}`: {error}"
            )))
        }
    }
    if !path_is_tracked(&root, &path)? {
        return Err(ContextPatchError::new(format!(
            "delete verification failed: `{path}` is no longer tracked in the index"
        )));
    }
    if !path_has_unstaged_deletion(&root, &path)? {
        return Err(ContextPatchError::new(format!(
            "delete verification failed: `{path}` is not an unstaged tracked deletion"
        )));
    }

    Ok(DeleteGuardedResult {
        path,
        sha256: verified_sha256,
        dry_run: false,
        deleted: true,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{delete_guarded, CONFIRMATION};
    use crate::fs::hash::sha256_file;

    #[test]
    fn previews_then_deletes_clean_tracked_file_as_unstaged_change() {
        let root = git_root("previews_then_deletes_clean_tracked_file_as_unstaged_change");
        fs::write(root.join("obsolete.txt"), "obsolete\n").unwrap();
        commit_all(&root);
        let expected_sha256 = sha256_file(&root.join("obsolete.txt")).unwrap();

        let preview = delete_guarded(
            &root,
            Path::new("obsolete.txt"),
            &expected_sha256,
            true,
            None,
        )
        .unwrap();
        assert!(preview.dry_run);
        assert!(!preview.deleted);
        assert!(root.join("obsolete.txt").exists());

        let deleted = delete_guarded(
            &root,
            Path::new("obsolete.txt"),
            &expected_sha256,
            false,
            Some(CONFIRMATION),
        )
        .unwrap();

        assert!(deleted.deleted);
        assert!(!root.join("obsolete.txt").exists());
        assert_eq!(
            git_stdout(&root, &["status", "--short"]),
            " D obsolete.txt\n"
        );
        assert_eq!(git_stdout(&root, &["diff", "--cached", "--name-only"]), "");
    }

    #[test]
    fn refuses_hash_mismatch_dirty_file_untracked_file_and_missing_confirmation() {
        let root =
            git_root("refuses_hash_mismatch_dirty_file_untracked_file_and_missing_confirmation");
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        commit_all(&root);
        let expected_sha256 = sha256_file(&root.join("tracked.txt")).unwrap();

        let error = delete_guarded(&root, Path::new("tracked.txt"), &"0".repeat(64), true, None)
            .unwrap_err();
        assert!(error.to_string().contains("hash mismatch"));

        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        let error = delete_guarded(
            &root,
            Path::new("tracked.txt"),
            &expected_sha256,
            true,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("must be clean"));
        fs::write(root.join("tracked.txt"), "base\n").unwrap();

        fs::write(root.join("untracked.txt"), "new\n").unwrap();
        let untracked_sha256 = sha256_file(&root.join("untracked.txt")).unwrap();
        let error = delete_guarded(
            &root,
            Path::new("untracked.txt"),
            &untracked_sha256,
            true,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("not tracked"));

        let error = delete_guarded(
            &root,
            Path::new("tracked.txt"),
            &expected_sha256,
            false,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("requires confirm"));
        assert_eq!(
            fs::read_to_string(root.join("tracked.txt")).unwrap(),
            "base\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_tracked_symlink() {
        use std::os::unix::fs::symlink;

        let root = git_root("refuses_delete_tracked_symlink");
        fs::write(root.join("target.txt"), "target\n").unwrap();
        symlink("target.txt", root.join("link.txt")).unwrap();
        commit_all(&root);
        let target_sha256 = sha256_file(&root.join("target.txt")).unwrap();

        let error =
            delete_guarded(&root, Path::new("link.txt"), &target_sha256, true, None).unwrap_err();

        assert!(error.to_string().contains("symlink"));
        assert!(root.join("link.txt").exists());
    }

    fn git_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        run_git(&root, &["init", "--quiet"]);
        root
    }

    fn commit_all(root: &Path) {
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "--quiet", "-m", "initial"]);
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn git_stdout(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    }
}
