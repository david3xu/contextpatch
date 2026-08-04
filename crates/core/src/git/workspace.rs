use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::error::ContextPatchError;

use super::guarded_path::canonical_repo_root;

/// Resolve one exact Git worktree root beneath a fixed workspace boundary.
pub fn resolve_workspace_git_root(
    workspace_root: &Path,
    repository: &str,
) -> Result<PathBuf, ContextPatchError> {
    let workspace = workspace_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve workspace root {}: {error}",
            workspace_root.display()
        ))
    })?;
    if !workspace.is_dir() {
        return Err(ContextPatchError::new(format!(
            "workspace root {} is not a directory",
            workspace.display()
        )));
    }

    let relative = normalize_repository_selector(repository)?;
    ensure_directory_without_symlinks(&workspace, &relative)?;
    let candidate = workspace.join(&relative);
    let resolved = candidate.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve selected repository `{relative}`: {error}"
        ))
    })?;
    if resolved == workspace || !resolved.starts_with(&workspace) {
        return Err(ContextPatchError::new(format!(
            "selected repository `{relative}` resolves outside the workspace boundary"
        )));
    }

    canonical_repo_root(&resolved)
}

fn normalize_repository_selector(repository: &str) -> Result<String, ContextPatchError> {
    let path = Path::new(repository);
    let bytes = repository.as_bytes();
    let has_windows_drive_prefix =
        bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if repository.is_empty()
        || repository.starts_with('/')
        || repository.ends_with('/')
        || repository.contains("//")
        || repository.contains('\\')
        || repository.chars().any(char::is_control)
        || has_windows_drive_prefix
    {
        return Err(ContextPatchError::invalid(
            "repository must be a normalized workspace-relative path",
        ));
    }

    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(ContextPatchError::invalid(
                "repository must be a normalized workspace-relative path",
            ));
        };
        let value = part
            .to_str()
            .ok_or_else(|| ContextPatchError::invalid("repository must be valid UTF-8"))?;
        if value.eq_ignore_ascii_case(".git") {
            return Err(ContextPatchError::invalid(
                "repository must not include the Git administrative directory",
            ));
        }
        parts.push(value);
    }

    let normalized = parts.join("/");
    if normalized != repository {
        return Err(ContextPatchError::invalid(
            "repository must be a normalized workspace-relative path",
        ));
    }
    Ok(normalized)
}

fn ensure_directory_without_symlinks(
    workspace: &Path,
    relative: &str,
) -> Result<(), ContextPatchError> {
    let components = Path::new(relative).components().collect::<Vec<_>>();
    let mut current = workspace.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        let display = current.strip_prefix(workspace).unwrap_or(&current);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ContextPatchError::invalid(format!(
                    "repository `{relative}` contains symlink component `{}`",
                    display.display()
                )))
            }
            Ok(metadata) if !metadata.is_dir() => {
                let label = if index + 1 == components.len() {
                    "selected repository"
                } else {
                    "repository path component"
                };
                return Err(ContextPatchError::invalid(format!(
                    "{label} `{}` is not a directory",
                    display.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(ContextPatchError::invalid(format!(
                    "repository `{relative}` does not exist"
                )))
            }
            Err(error) => {
                return Err(ContextPatchError::new(format!(
                    "failed to inspect repository path component `{}`: {error}",
                    display.display()
                )))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn resolves_only_exact_descendant_git_roots() {
        let workspace = temp_root("workspace_git_root");
        let first = workspace.join("first");
        init_git_repo(&first);
        fs::create_dir_all(first.join("subdir")).unwrap();

        let parent = workspace.join("parent");
        init_git_repo(&parent);
        let nested = parent.join("nested");
        init_git_repo(&nested);

        assert_eq!(
            resolve_workspace_git_root(&workspace, "first").unwrap(),
            first.canonicalize().unwrap()
        );
        assert_eq!(
            resolve_workspace_git_root(&workspace, "parent/nested").unwrap(),
            nested.canonicalize().unwrap()
        );

        let error = resolve_workspace_git_root(&workspace, "first/subdir").unwrap_err();
        assert!(
            error.to_string().contains("not the Git worktree root"),
            "{error}"
        );

        let plain = workspace.join("plain");
        fs::create_dir(&plain).unwrap();
        let error = resolve_workspace_git_root(&workspace, "plain").unwrap_err();
        assert!(
            error.to_string().contains("resolve Git root failed"),
            "{error}"
        );

        fs::write(workspace.join("file"), "not a directory\n").unwrap();
        let error = resolve_workspace_git_root(&workspace, "file").unwrap_err();
        assert!(error.to_string().contains("not a directory"), "{error}");
    }

    #[test]
    fn refuses_non_normalized_repository_selectors() {
        let workspace = temp_root("workspace_git_root_invalid");
        for repository in [
            "",
            "/absolute",
            "../escape",
            "repo/",
            "repo//nested",
            "repo\\nested",
            "./repo",
            "repo/../other",
            "C:/repo",
            "repo\nnested",
        ] {
            let error = resolve_workspace_git_root(&workspace, repository).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("normalized workspace-relative path"),
                "{repository:?}: {error}"
            );
        }

        for repository in [".git", "repo/.GIT/objects"] {
            let error = resolve_workspace_git_root(&workspace, repository).unwrap_err();
            assert!(
                error.to_string().contains("Git administrative directory"),
                "{repository:?}: {error}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_components_even_when_the_target_is_a_git_root() {
        use std::os::unix::fs::symlink;

        let workspace = temp_root("workspace_git_root_symlink");
        let outside = temp_root("workspace_git_root_outside");
        init_git_repo(&outside);
        symlink(&outside, workspace.join("linked")).unwrap();

        let error = resolve_workspace_git_root(&workspace, "linked").unwrap_err();
        assert!(error.to_string().contains("symlink component"), "{error}");
    }

    fn init_git_repo(root: &Path) {
        fs::create_dir_all(root).unwrap();
        let status = Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(root)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
