use std::path::Path;

use crate::error::ContextPatchError;

pub fn apply_patch(_repo_root: &Path, _patch: &str) -> Result<(), ContextPatchError> {
    Err(ContextPatchError::new("patch apply is not implemented yet"))
}
