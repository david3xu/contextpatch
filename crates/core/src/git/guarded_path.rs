use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use crate::error::ContextPatchError;

pub(crate) fn canonical_repo_root(repo_root: &Path) -> Result<PathBuf, ContextPatchError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    if !root.is_dir() {
        return Err(ContextPatchError::new(format!(
            "repository root {} is not a directory",
            root.display()
        )));
    }

    let output = git_output(&root, &["rev-parse", "--show-toplevel"], "resolve Git root")?;
    let git_root = std::str::from_utf8(&output.stdout)
        .map_err(|error| ContextPatchError::new(format!("Git root is not UTF-8: {error}")))?
        .trim();
    let git_root = Path::new(git_root).canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to canonicalize Git root {git_root}: {error}"
        ))
    })?;
    if git_root != root {
        return Err(ContextPatchError::new(format!(
            "configured repository root {} is not the Git worktree root {}",
            root.display(),
            git_root.display()
        )));
    }

    Ok(root)
}

pub(crate) fn resolve_existing_regular_file(
    root: &Path,
    path: &Path,
) -> Result<(String, PathBuf), ContextPatchError> {
    let relative = normalize_relative_path(path)?;
    ensure_no_symlink_components(root, &relative, false)?;
    let target = root.join(&relative);
    let metadata = fs::symlink_metadata(&target).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            ContextPatchError::new(format!("source file `{relative}` does not exist"))
        } else {
            ContextPatchError::new(format!(
                "failed to inspect source file `{relative}`: {error}"
            ))
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ContextPatchError::new(format!(
            "source file `{relative}` must not be a symlink"
        )));
    }
    if !metadata.is_file() {
        return Err(ContextPatchError::new(format!(
            "source path `{relative}` is not a regular file"
        )));
    }
    let resolved = target.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve source file `{relative}`: {error}"
        ))
    })?;
    if !resolved.starts_with(root) {
        return Err(ContextPatchError::new(format!(
            "source file `{relative}` resolves outside repository root"
        )));
    }
    Ok((relative, target))
}

pub(crate) fn resolve_absent_file(
    root: &Path,
    path: &Path,
) -> Result<(String, PathBuf), ContextPatchError> {
    let relative = normalize_relative_path(path)?;
    ensure_no_symlink_components(root, &relative, true)?;
    let target = root.join(&relative);
    match fs::symlink_metadata(&target) {
        Ok(_) => {
            return Err(ContextPatchError::new(format!(
                "destination path `{relative}` already exists"
            )))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(ContextPatchError::new(format!(
                "failed to inspect destination path `{relative}`: {error}"
            )))
        }
    }

    let parent = target.parent().ok_or_else(|| {
        ContextPatchError::new(format!("destination path `{relative}` has no parent"))
    })?;
    let resolved_parent = parent.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve destination parent for `{relative}`: {error}"
        ))
    })?;
    if !resolved_parent.starts_with(root) {
        return Err(ContextPatchError::new(format!(
            "destination path `{relative}` resolves outside repository root"
        )));
    }
    if !resolved_parent.is_dir() {
        return Err(ContextPatchError::new(format!(
            "destination parent for `{relative}` is not a directory"
        )));
    }
    Ok((relative, target))
}

pub(crate) fn ensure_tracked(
    root: &Path,
    relative: &str,
    label: &str,
) -> Result<(), ContextPatchError> {
    if !path_is_tracked(root, relative)? {
        return Err(ContextPatchError::new(format!(
            "{label} `{relative}` is not tracked by Git"
        )));
    }
    Ok(())
}

pub(crate) fn ensure_not_tracked(
    root: &Path,
    relative: &str,
    label: &str,
) -> Result<(), ContextPatchError> {
    if path_is_tracked(root, relative)? {
        return Err(ContextPatchError::new(format!(
            "{label} `{relative}` is already tracked in the Git index"
        )));
    }
    Ok(())
}

pub(crate) fn path_is_tracked(root: &Path, relative: &str) -> Result<bool, ContextPatchError> {
    let output = git_output(
        root,
        &["ls-files", "-z", "--", relative],
        "inspect tracked path",
    )?;
    for entry in output.stdout.split(|byte| *byte == 0) {
        if entry == relative.as_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn ensure_path_clean(root: &Path, relative: &str) -> Result<(), ContextPatchError> {
    let output = git_output(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            relative,
        ],
        "inspect source status",
    )?;
    if !output.stdout.is_empty() {
        return Err(ContextPatchError::new(format!(
            "tracked source `{relative}` must be clean before mutation; status: {}",
            display_nul_output(&output.stdout)
        )));
    }
    Ok(())
}

pub(crate) fn ensure_index_clean(root: &Path) -> Result<(), ContextPatchError> {
    let output = git_output_allow_failure(
        root,
        &["diff", "--cached", "--quiet", "--exit-code"],
        "inspect Git index",
    )?;
    match output.status.code() {
        Some(0) => Ok(()),
        Some(1) => Err(ContextPatchError::new(
            "Git index must be clean before moving a tracked file",
        )),
        _ => Err(ContextPatchError::new(format!(
            "failed to inspect Git index: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

pub(crate) fn move_with_git(root: &Path, from: &str, to: &str) -> Result<(), ContextPatchError> {
    git_output(root, &["mv", "--", from, to], "move tracked file")?;
    Ok(())
}

pub(crate) fn path_has_unstaged_deletion(
    root: &Path,
    relative: &str,
) -> Result<bool, ContextPatchError> {
    let output = git_output(
        root,
        &["status", "--porcelain=v1", "-z", "--", relative],
        "verify tracked deletion",
    )?;
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            entry.len() >= 4
                && entry[0] == b' '
                && entry[1] == b'D'
                && entry[2] == b' '
                && &entry[3..] == relative.as_bytes()
        }))
}

fn normalize_relative_path(path: &Path) -> Result<String, ContextPatchError> {
    let raw = path
        .to_str()
        .ok_or_else(|| ContextPatchError::new("path must be valid UTF-8"))?;
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.ends_with('/')
        || raw.contains("//")
        || raw.contains('\\')
        || raw.chars().any(char::is_control)
    {
        return Err(ContextPatchError::new(
            "path must be a normalized repository-relative path",
        ));
    }
    if raw.starts_with(':') || raw.chars().any(|ch| matches!(ch, '*' | '?' | '[')) {
        return Err(ContextPatchError::new(format!(
            "path `{raw}` contains Git pathspec metacharacters"
        )));
    }

    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(ContextPatchError::new(
                "path must be a normalized repository-relative path",
            ));
        };
        if part
            .to_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(".git"))
        {
            return Err(ContextPatchError::new(
                "paths under the Git administrative directory are refused",
            ));
        }
        parts.push(part);
    }
    let normalized = parts
        .iter()
        .map(|part| part.to_str().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("/");
    if normalized != raw {
        return Err(ContextPatchError::new(
            "path must be a normalized repository-relative path",
        ));
    }
    Ok(normalized)
}

fn ensure_no_symlink_components(
    root: &Path,
    relative: &str,
    allow_missing_leaf: bool,
) -> Result<(), ContextPatchError> {
    let components = Path::new(relative).components().collect::<Vec<_>>();
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ContextPatchError::new(format!(
                    "path `{relative}` contains symlink component `{}`",
                    current.strip_prefix(root).unwrap_or(&current).display()
                )))
            }
            Ok(_) => {}
            Err(error)
                if allow_missing_leaf
                    && index + 1 == components.len()
                    && error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ContextPatchError::new(format!(
                    "failed to inspect path component `{}`: {error}",
                    current.strip_prefix(root).unwrap_or(&current).display()
                )))
            }
        }
    }
    Ok(())
}

fn git_output(root: &Path, args: &[&str], label: &str) -> Result<Output, ContextPatchError> {
    let output = git_output_allow_failure(root, args, label)?;
    if !output.status.success() {
        return Err(ContextPatchError::new(format!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output)
}

fn git_output_allow_failure(
    root: &Path,
    args: &[&str],
    label: &str,
) -> Result<Output, ContextPatchError> {
    Command::new("git")
        .arg("--no-pager")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| ContextPatchError::new(format!("failed to {label}: {error}")))
}

fn display_nul_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace('\0', "\n")
        .trim()
        .to_string()
}
