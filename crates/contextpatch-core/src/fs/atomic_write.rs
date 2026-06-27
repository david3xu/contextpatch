use std::path::Path;

use crate::error::ContextPatchError;

pub fn write_atomic(_path: &Path, _contents: &[u8]) -> Result<(), ContextPatchError> {
    Err(ContextPatchError::new(
        "atomic write is not implemented yet",
    ))
}
