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
}

impl fmt::Display for ContextPatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ContextPatchError {}
