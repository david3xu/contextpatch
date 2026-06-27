use crate::error::ContextPatchError;

pub fn require_clean_guard(_enabled: bool) -> Result<(), ContextPatchError> {
    Err(ContextPatchError::new(
        "policy guard is not implemented yet",
    ))
}
