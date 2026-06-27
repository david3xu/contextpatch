use std::path::Path;

use crate::error::ContextPatchError;

pub fn replace_exact(_path: &Path, _old: &str, _new: &str) -> Result<(), ContextPatchError> {
    Err(ContextPatchError::new(
        "exact replace is not implemented yet",
    ))
}
