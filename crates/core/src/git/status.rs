use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::ContextPatchError;
use crate::process::runner::{run_bounded_command, BoundedProcessOutput};
use crate::process::GIT_SUBPROCESS_TIMEOUT;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StatusSnapshot {
    pub clean: bool,
    pub change_count: usize,
    pub sampled_changes: Vec<String>,
    pub sample_truncated: bool,
}

impl StatusSnapshot {
    pub fn summary(&self) -> String {
        if self.clean {
            return "clean: no Git changes".to_string();
        }
        if self.sample_truncated {
            return format!(
                "repository has {} uncommitted changes; showing {}",
                self.change_count,
                self.sampled_changes.len()
            );
        }
        format!("repository has {} uncommitted changes", self.change_count)
    }
}

pub fn status_summary(repo_root: &Path) -> Result<String, ContextPatchError> {
    status_summary_for_path(repo_root, None)
}

pub fn status_snapshot(
    repo_root: &Path,
    max_sampled_changes: usize,
    max_sample_bytes: usize,
) -> Result<StatusSnapshot, ContextPatchError> {
    let (_, changes) = status_changes_for_path(repo_root, None)?;
    let change_count = changes.len();
    let mut sampled_changes = Vec::new();
    let mut sampled_bytes = 0usize;

    for change in &changes {
        if sampled_changes.len() >= max_sampled_changes {
            break;
        }
        let separator_bytes = usize::from(!sampled_changes.is_empty());
        let required_bytes = separator_bytes.saturating_add(change.len());
        if required_bytes > max_sample_bytes.saturating_sub(sampled_bytes) {
            break;
        }
        sampled_bytes = sampled_bytes.saturating_add(required_bytes);
        sampled_changes.push(change.clone());
    }

    Ok(StatusSnapshot {
        clean: changes.is_empty(),
        change_count,
        sample_truncated: sampled_changes.len() < changes.len(),
        sampled_changes,
    })
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
    let (scope, changes) = status_changes_for_path(repo_root, path)?;

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

fn status_changes_for_path(
    repo_root: &Path,
    path: Option<&Path>,
) -> Result<(Option<PathBuf>, Vec<String>), ContextPatchError> {
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
    let changes = stdout.lines().map(str::to_string).collect();
    Ok((scope, changes))
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
) -> Result<BoundedProcessOutput, ContextPatchError> {
    let args = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    let output = run_bounded_command(root, "git", &args, GIT_SUBPROCESS_TIMEOUT, label)?;

    if output.timed_out {
        return Err(ContextPatchError::new(format!(
            "{label} timed out after {} seconds; the Git process was terminated",
            GIT_SUBPROCESS_TIMEOUT.as_secs()
        )));
    }
    if output.stdout_truncated || output.stderr_truncated {
        return Err(ContextPatchError::new(format!(
            "{label} refused: Git output exceeded the bounded capture limit"
        )));
    }
    if !output.success() {
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
    let entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    for entry in entries {
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

    use super::{
        dirty_paths, status_short, status_snapshot, status_summary, status_summary_for_path,
    };

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
    fn snapshots_dirty_status_with_entry_and_byte_bounds() {
        let root = git_root("snapshots_dirty_status_with_bounds");
        for index in 0..5 {
            fs::write(root.join(format!("sample-{index}.txt")), "content").unwrap();
        }

        let entry_bounded = status_snapshot(&root, 2, 1_000).unwrap();
        assert!(!entry_bounded.clean);
        assert_eq!(entry_bounded.change_count, 5);
        assert_eq!(entry_bounded.sampled_changes.len(), 2);
        assert!(entry_bounded.sample_truncated);
        assert_eq!(
            entry_bounded.summary(),
            "repository has 5 uncommitted changes; showing 2"
        );

        let byte_bounded = status_snapshot(&root, 5, 1).unwrap();
        assert_eq!(byte_bounded.change_count, 5);
        assert!(byte_bounded.sampled_changes.is_empty());
        assert!(byte_bounded.sample_truncated);
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
