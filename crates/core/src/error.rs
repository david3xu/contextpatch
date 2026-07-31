use std::fmt;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ContextPatchError {
    message: String,
}

impl ContextPatchError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// An error caused by input the caller can correct, as opposed to an environment failure.
    ///
    /// Behaviourally identical to `new`. It exists because nearly every error this crate raises is a
    /// refusal of bad input, and naming that at the construction site makes the guard's intent legible
    /// without a comment. `new` remains for the cases that are genuinely not about input, so the
    /// distinction stays meaningful rather than becoming a synonym everyone ignores.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(message)
    }
}

impl fmt::Display for ContextPatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ContextPatchError {}
