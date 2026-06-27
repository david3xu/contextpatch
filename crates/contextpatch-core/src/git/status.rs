use std::path::Path;

use crate::error::ContextPatchError;

pub fn status_summary(_repo_root: &Path) -> Result<String, ContextPatchError> {
    Err(ContextPatchError::new(
        "git status guard is not implemented yet",
    ))
}
