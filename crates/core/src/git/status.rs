use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::ContextPatchError;

pub fn status_summary(repo_root: &Path) -> Result<String, ContextPatchError> {
    status_summary_for_path(repo_root, None)
}

pub fn status_short(repo_root: &Path) -> Result<String, ContextPatchError> {
    let root = canonical_repo_root(repo_root)?;
    let output = git_output(
        &root,
        &["status", "--short", "--untracked-files=all"],
        "git status --short",
    )?;
    String::from_utf8(output.stdout).map_err(|error| {
        ContextPatchError::new(format!("git status output was not valid UTF-8: {error}"))
    })
}

pub fn dirty_paths(repo_root: &Path) -> Result<BTreeSet<String>, ContextPatchError> {
    let root = canonical_repo_root(repo_root)?;
    let output = git_output(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        "git status",
    )?;
    parse_porcelain_paths(&output.stdout, "git status")
}

pub fn status_summary_for_path(
    repo_root: &Path,
    path: Option<&Path>,
) -> Result<String, ContextPatchError> {
    let root = canonical_repo_root(repo_root)?;
    let scope = path
        .map(|path| guarded_relative_path(&root, path))
        .transpose()?;

    let mut args = vec!["status", "--porcelain=v1", "--untracked-files=all"];
    let scope_string;
    if let Some(scope) = &scope {
        scope_string = scope.to_string_lossy().into_owned();
        args.push("--");
        args.push(&scope_string);
    }
    let output = git_output(&root, &args, "git status")?;

    let stdout = String::from_utf8(output.stdout).map_err(|error| {
        ContextPatchError::new(format!("git status output was not valid UTF-8: {error}"))
    })?;
    let changes: Vec<&str> = stdout.lines().collect();

    if changes.is_empty() {
        return Ok(match scope {
            Some(scope) => format!("clean: no Git changes under {}", scope.display()),
            None => "clean: no Git changes".to_string(),
        });
    }

    let scope_label = scope
        .as_ref()
        .map(|scope| format!(" under {}", scope.display()))
        .unwrap_or_default();
    Err(ContextPatchError::new(format!(
        "repository has uncommitted changes{scope_label}:\n{}",
        changes.join("\n")
    )))
}

fn canonical_repo_root(repo_root: &Path) -> Result<PathBuf, ContextPatchError> {
    repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })
}

fn git_output(
    root: &Path,
    args: &[&str],
    label: &str,
) -> Result<std::process::Output, ContextPatchError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| {
            ContextPatchError::new(format!(
                "failed to run {label} for {}: {error}",
                root.display()
            ))
        })?;

    if !output.status.success() {
        return Err(ContextPatchError::new(format!(
            "{label} failed for {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(output)
}

fn parse_porcelain_paths(bytes: &[u8], label: &str) -> Result<BTreeSet<String>, ContextPatchError> {
    let mut paths = BTreeSet::new();
    let mut entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(ContextPatchError::new(format!(
                "unexpected {label} porcelain entry"
            )));
        }
        if matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C') {
            return Err(ContextPatchError::new(
                "rename/copy status entries require a dedicated tracked-move workflow",
            ));
        }
        let path = std::str::from_utf8(&entry[3..]).map_err(|error| {
            ContextPatchError::new(format!("{label} path is not UTF-8: {error}"))
        })?;
        paths.insert(path.to_string());
    }
    Ok(paths)
}

fn guarded_relative_path(root: &Path, path: &Path) -> Result<PathBuf, ContextPatchError> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };

    let resolved = if candidate.exists() {
        candidate.canonicalize().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to resolve status path {}: {error}",
                candidate.display()
            ))
        })?
    } else {
        let parent = candidate.parent().ok_or_else(|| {
            ContextPatchError::new(format!("status path {} has no parent", candidate.display()))
        })?;
        let resolved_parent = parent.canonicalize().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to resolve status path parent {}: {error}",
                parent.display()
            ))
        })?;
        let file_name = candidate
            .file_name()
            .ok_or_else(|| ContextPatchError::new("status path has no file name"))?;
        resolved_parent.join(file_name)
    };

    if !resolved.starts_with(root) {
        return Err(ContextPatchError::new(format!(
            "status path {} is outside repository root {}",
            resolved.display(),
            root.display()
        )));
    }

    let relative = resolved.strip_prefix(root).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to make status path {} relative to {}: {error}",
            resolved.display(),
            root.display()
        ))
    })?;

    if relative.as_os_str().is_empty() {
        Ok(PathBuf::from("."))
    } else {
        Ok(relative.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{dirty_paths, status_short, status_summary, status_summary_for_path};

    #[test]
    fn returns_clean_summary_for_clean_repository() {
        let root = git_root("returns_clean_summary_for_clean_repository");

        let summary = status_summary(&root).unwrap();

        assert_eq!(summary, "clean: no Git changes");
    }

    #[test]
    fn refuses_dirty_repository() {
        let root = git_root("refuses_dirty_repository");
        fs::write(root.join("sample.txt"), "content").unwrap();

        let error = status_summary(&root).unwrap_err();

        assert!(error
            .to_string()
            .contains("repository has uncommitted changes"));
        assert!(error.to_string().contains("?? sample.txt"));
    }

    #[test]
    fn scopes_status_to_requested_path() {
        let root = git_root("scopes_status_to_requested_path");
        fs::write(root.join("dirty.txt"), "content").unwrap();

        let summary = status_summary_for_path(&root, Some(Path::new("clean.txt"))).unwrap();

        assert_eq!(summary, "clean: no Git changes under clean.txt");
    }

    #[test]
    fn refuses_dirty_requested_path() {
        let root = git_root("refuses_dirty_requested_path");
        fs::write(root.join("dirty.txt"), "content").unwrap();

        let error = status_summary_for_path(&root, Some(Path::new("dirty.txt"))).unwrap_err();

        assert!(error.to_string().contains("under dirty.txt"));
        assert!(error.to_string().contains("?? dirty.txt"));
    }

    #[test]
    fn refuses_paths_outside_root() {
        let root = git_root("refuses_paths_outside_root");
        let outside = temp_root("status-outside").join("outside.txt");

        let error = status_summary_for_path(&root, Some(&outside)).unwrap_err();

        assert!(error.to_string().contains("outside repository root"));
    }

    #[test]
    fn returns_short_status_and_dirty_paths() {
        let root = git_root("returns_short_status_and_dirty_paths");
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        run_git(&root, &["add", "tracked.txt"]);
        run_git(&root, &["commit", "--quiet", "-m", "initial"]);

        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("new.txt"), "new\n").unwrap();

        let status = status_short(&root).unwrap();
        let paths = dirty_paths(&root).unwrap();

        assert!(status.contains(" M tracked.txt"));
        assert!(status.contains("?? new.txt"));
        assert_eq!(
            paths,
            ["new.txt".to_string(), "tracked.txt".to_string()]
                .into_iter()
                .collect()
        );
    }

    fn git_root(name: &str) -> std::path::PathBuf {
        let root = temp_root(name);
        run_git(&root, &["init", "--quiet"]);
        root
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
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
}
