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
