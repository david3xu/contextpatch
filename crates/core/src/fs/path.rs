use std::path::{Path, PathBuf};

use crate::error::ContextPatchError;

pub fn resolve_existing_file(repo_root: &Path, path: &Path) -> Result<PathBuf, ContextPatchError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };

    let resolved = candidate.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve target file {}: {error}",
            candidate.display()
        ))
    })?;

    if !resolved.starts_with(&root) {
        return Err(ContextPatchError::new(format!(
            "target path {} is outside repository root {}",
            resolved.display(),
            root.display()
        )));
    }

    if !resolved.is_file() {
        return Err(ContextPatchError::new(format!(
            "target path {} is not a file",
            resolved.display()
        )));
    }

    Ok(resolved)
}

pub fn resolve_new_file(repo_root: &Path, path: &Path) -> Result<PathBuf, ContextPatchError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };

    if candidate.exists() {
        return Err(ContextPatchError::new(format!(
            "target file {} already exists",
            candidate.display()
        )));
    }

    let parent = candidate.parent().ok_or_else(|| {
        ContextPatchError::new(format!("target path {} has no parent", candidate.display()))
    })?;
    let resolved_parent = parent.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve parent directory {}: {error}",
            parent.display()
        ))
    })?;

    if !resolved_parent.starts_with(&root) {
        return Err(ContextPatchError::new(format!(
            "target path {} is outside repository root {}",
            candidate.display(),
            root.display()
        )));
    }

    if !resolved_parent.is_dir() {
        return Err(ContextPatchError::new(format!(
            "target parent {} is not a directory",
            resolved_parent.display()
        )));
    }

    let file_name = candidate
        .file_name()
        .ok_or_else(|| ContextPatchError::new("target path has no file name"))?;

    Ok(resolved_parent.join(file_name))
}
