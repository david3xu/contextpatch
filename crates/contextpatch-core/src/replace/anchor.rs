use std::path::Path;

use crate::error::ContextPatchError;

pub fn insert_at_anchor(
    _path: &Path,
    _anchor: &str,
    _contents: &str,
) -> Result<(), ContextPatchError> {
    Err(ContextPatchError::new(
        "anchor insert is not implemented yet",
    ))
}
