use std::path::Path;

use crate::error::ContextPatchError;

pub fn move_tracked(_from: &Path, _to: &Path) -> Result<(), ContextPatchError> {
    Err(ContextPatchError::new(
        "tracked move is not implemented yet",
    ))
}
