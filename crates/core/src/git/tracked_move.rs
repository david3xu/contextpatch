use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::error::ContextPatchError;
use crate::fs::hash::sha256_file;
use crate::git::guarded_path::{
    ensure_index_clean, ensure_not_tracked, ensure_path_clean, ensure_tracked, exact_worktree_root,
    move_with_git, path_is_tracked, resolve_absent_file, resolve_existing_regular_file,
};

pub const CONFIRMATION: &str = "move tracked file";

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct MoveTrackedResult {
    pub from: String,
    pub to: String,
    pub sha256: String,
    pub dry_run: bool,
    pub moved: bool,
}

pub fn move_tracked(
    repo_root: &Path,
    from: &Path,
    to: &Path,
    dry_run: bool,
    confirmation: Option<&str>,
) -> Result<MoveTrackedResult, ContextPatchError> {
    if !dry_run && confirmation != Some(CONFIRMATION) {
        return Err(ContextPatchError::new(format!(
            "dry_run=false requires confirm: {CONFIRMATION:?}"
        )));
    }

    let root = exact_worktree_root(repo_root)?;
    let (from, source) = resolve_existing_regular_file(&root, from)?;
    let (to, destination) = resolve_absent_file(&root, to)?;
    if from == to {
        return Err(ContextPatchError::new(
            "source and destination paths must differ",
        ));
    }
    ensure_tracked(&root, &from, "source file")?;
    ensure_not_tracked(&root, &to, "destination path")?;
    ensure_path_clean(&root, &from)?;
    ensure_index_clean(&root)?;
    let source_sha256 = sha256_file(&source)?;

    if dry_run {
        return Ok(MoveTrackedResult {
            from,
            to,
            sha256: source_sha256,
            dry_run: true,
            moved: false,
        });
    }

    move_with_git(&root, &from, &to)?;

    match fs::symlink_metadata(&source) {
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(ContextPatchError::new(format!(
                "move verification failed: source path `{from}` still exists"
            )))
        }
        Err(error) => {
            return Err(ContextPatchError::new(format!(
                "move verification failed while inspecting source `{from}`: {error}"
            )))
        }
    }
    let destination_metadata = fs::symlink_metadata(&destination).map_err(|error| {
        ContextPatchError::new(format!(
            "move verification failed while inspecting destination `{to}`: {error}"
        ))
    })?;
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_file() {
        return Err(ContextPatchError::new(format!(
            "move verification failed: destination `{to}` is not a regular file"
        )));
    }
    let destination_sha256 = sha256_file(&destination)?;
    if destination_sha256 != source_sha256 {
        return Err(ContextPatchError::new(format!(
            "move verification failed: content hash changed from {source_sha256} to {destination_sha256}"
        )));
    }
    if path_is_tracked(&root, &from)? {
        return Err(ContextPatchError::new(format!(
            "move verification failed: source `{from}` remains tracked"
        )));
    }
    if !path_is_tracked(&root, &to)? {
        return Err(ContextPatchError::new(format!(
            "move verification failed: destination `{to}` is not tracked"
        )));
    }

    Ok(MoveTrackedResult {
        from,
        to,
        sha256: destination_sha256,
        dry_run: false,
        moved: true,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{move_tracked, CONFIRMATION};

    #[test]
    fn previews_then_moves_one_clean_tracked_file() {
        let root = git_root("previews_then_moves_one_clean_tracked_file");
        fs::write(root.join("old.txt"), "tracked content\n").unwrap();
        fs::create_dir(root.join("renamed")).unwrap();
        commit_all(&root);

        let preview = move_tracked(
            &root,
            Path::new("old.txt"),
            Path::new("renamed/new.txt"),
            true,
            None,
        )
        .unwrap();
        assert!(preview.dry_run);
        assert!(!preview.moved);
        assert!(root.join("old.txt").exists());
        assert!(!root.join("renamed/new.txt").exists());

        let moved = move_tracked(
            &root,
            Path::new("old.txt"),
            Path::new("renamed/new.txt"),
            false,
            Some(CONFIRMATION),
        )
        .unwrap();

        assert!(moved.moved);
        assert_eq!(moved.sha256, preview.sha256);
        assert!(!root.join("old.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("renamed/new.txt")).unwrap(),
            "tracked content\n"
        );
        assert!(git_stdout(&root, &["status", "--short"]).contains("old.txt -> renamed/new.txt"));
    }

    #[test]
    fn refuses_dirty_source_existing_destination_and_dirty_index() {
        let root = git_root("refuses_dirty_source_existing_destination_and_dirty_index");
        fs::write(root.join("source.txt"), "base\n").unwrap();
        fs::write(root.join("other.txt"), "base\n").unwrap();
        commit_all(&root);

        fs::write(root.join("source.txt"), "changed\n").unwrap();
        let error = move_tracked(
            &root,
            Path::new("source.txt"),
            Path::new("moved.txt"),
            true,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("must be clean"));
        fs::write(root.join("source.txt"), "base\n").unwrap();

        fs::write(root.join("moved.txt"), "occupied\n").unwrap();
        let error = move_tracked(
            &root,
            Path::new("source.txt"),
            Path::new("moved.txt"),
            true,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        fs::remove_file(root.join("moved.txt")).unwrap();

        fs::write(root.join("other.txt"), "staged\n").unwrap();
        run_git(&root, &["add", "other.txt"]);
        let error = move_tracked(
            &root,
            Path::new("source.txt"),
            Path::new("moved.txt"),
            true,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("index must be clean"));
        assert!(root.join("source.txt").exists());
        assert!(!root.join("moved.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_tracked_symlink_source() {
        use std::os::unix::fs::symlink;

        let root = git_root("refuses_tracked_symlink_source");
        fs::write(root.join("target.txt"), "target\n").unwrap();
        symlink("target.txt", root.join("link.txt")).unwrap();
        commit_all(&root);

        let error = move_tracked(
            &root,
            Path::new("link.txt"),
            Path::new("moved.txt"),
            true,
            None,
        )
        .unwrap_err();

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
