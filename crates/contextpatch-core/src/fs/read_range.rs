use std::path::Path;

use crate::error::ContextPatchError;

pub fn read_range(
    _path: &Path,
    _start_line: usize,
    _end_line: usize,
) -> Result<String, ContextPatchError> {
    Err(ContextPatchError::new("read range is not implemented yet"))
}
