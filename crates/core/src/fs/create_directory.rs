use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::ContextPatchError;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CreateDirectorySummary {
    pub path: PathBuf,
    pub directories_created: Vec<PathBuf>,
}

pub fn create_directory(
    path: &Path,
    parents: bool,
) -> Result<CreateDirectorySummary, ContextPatchError> {
    let repo_root = std::env::current_dir().map_err(|error| {
        ContextPatchError::new(format!("failed to read current directory: {error}"))
    })?;

    create_directory_in_root(&repo_root, path, parents)
}

/// Create one directory under the repository root.
///
/// The root is reached through the root's authority, so a selected repository is never re-resolved. The
/// per-component walk below is still path-based and canonicalizes each step to confine it; converting that
/// walk to `mkdirat` is tracked as a remaining C13 boundary because its refusals name absolute paths and
/// moving them is a separate, visible change.
pub fn create_directory_in_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    path: &Path,
    parents: bool,
) -> Result<CreateDirectorySummary, ContextPatchError> {
    let root = crate::fs::rooted::canonical_label(repo_root.into())?;
    let components = safe_relative_components(&root, path)?;
    let mut current = root.clone();
    let mut directories_created = Vec::new();

    for (index, component) in components.iter().enumerate() {
        let next = current.join(component);
        let is_target = index + 1 == components.len();

        if next.exists() {
            let resolved = next.canonicalize().map_err(|error| {
                ContextPatchError::new(format!(
                    "failed to resolve directory component {}: {error}",
                    next.display()
                ))
            })?;

            if !resolved.starts_with(&root) {
                return Err(ContextPatchError::new(format!(
                    "target path {} is outside repository root {}",
                    next.display(),
                    root.display()
                )));
            }

            if !resolved.is_dir() {
                return Err(ContextPatchError::new(format!(
                    "target path {} is not a directory",
                    next.display()
                )));
            }

            if is_target {
                return Err(ContextPatchError::new(format!(
                    "target directory {} already exists",
                    resolved.display()
                )));
            }

            current = resolved;
            continue;
        }

        if !parents && !is_target {
            return Err(ContextPatchError::new(format!(
                "failed to resolve parent directory {}: missing directory",
                next.display()
            )));
        }

        fs::create_dir(&next).map_err(|error| {
            ContextPatchError::new(format!(
                "failed to create directory {}: {error}",
                next.display()
            ))
        })?;
        let resolved = next.canonicalize().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to resolve created directory {}: {error}",
                next.display()
            ))
        })?;
        directories_created.push(resolved.clone());
        current = resolved;
    }

    Ok(CreateDirectorySummary {
        path: current,
        directories_created,
    })
}

fn safe_relative_components(root: &Path, path: &Path) -> Result<Vec<OsString>, ContextPatchError> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root).map_err(|_| {
            ContextPatchError::new(format!(
                "target path {} is outside repository root {}",
                path.display(),
                root.display()
            ))
        })?
    } else {
        path
    };

    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => components.push(name.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ContextPatchError::new(format!(
                    "target path {} is outside repository root {}",
                    path.display(),
                    root.display()
                )));
            }
        }
    }

    if components.is_empty() {
        return Err(ContextPatchError::new("target path has no directory name"));
    }

    Ok(components)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::create_directory_in_root;

    #[test]
    fn creates_leaf_directory() {
        let root = temp_root("creates_leaf_directory");

        let summary = create_directory_in_root(&root, Path::new("native"), false).unwrap();

        assert!(summary.path.is_dir());
        assert_eq!(summary.directories_created.len(), 1);
        assert!(root.join("native").is_dir());
    }

    #[test]
    fn creates_nested_directories_when_parents_enabled() {
        let root = temp_root("creates_nested_directories_when_parents_enabled");

        let summary =
            create_directory_in_root(&root, Path::new("native-plugins/background-audio"), true)
                .unwrap();

        assert!(summary.path.is_dir());
        assert_eq!(summary.directories_created.len(), 2);
        assert!(root.join("native-plugins/background-audio").is_dir());
    }

    #[test]
    fn refuses_missing_parent_when_parents_disabled() {
        let root = temp_root("refuses_missing_parent_when_parents_disabled");

        let error = create_directory_in_root(&root, Path::new("missing/child"), false).unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to resolve parent directory"));
        assert!(!root.join("missing").exists());
    }

    #[test]
    fn refuses_existing_directory() {
        let root = temp_root("refuses_existing_directory");
        fs::create_dir(root.join("existing")).unwrap();

        let error = create_directory_in_root(&root, Path::new("existing"), false).unwrap_err();

        assert!(error.to_string().contains("already exists"));
    }

    #[test]
    fn refuses_existing_file_component() {
        let root = temp_root("refuses_existing_file_component");
        fs::write(root.join("file"), "content").unwrap();

        let error = create_directory_in_root(&root, Path::new("file/child"), true).unwrap_err();

        assert!(error.to_string().contains("not a directory"));
    }

    #[test]
    fn refuses_paths_outside_root() {
        let root = temp_root("refuses_paths_outside_root");
        let outside = std::env::temp_dir().join(format!(
            "contextpatch-create-directory-outside-{}",
            std::process::id()
        ));

        let error = create_directory_in_root(&root, &outside, true).unwrap_err();

        assert!(error.to_string().contains("outside repository root"));
        assert!(!outside.exists());
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-create-directory-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
