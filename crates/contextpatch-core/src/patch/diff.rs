use std::path::Path;

use crate::error::ContextPatchError;

pub fn preview_diff(_path: &Path, _new_contents: &str) -> Result<String, ContextPatchError> {
    Err(ContextPatchError::new(
        "diff preview is not implemented yet",
    ))
}
