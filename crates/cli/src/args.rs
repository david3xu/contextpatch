#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Command {
    Help,
    Version,
    Status,
    ReadRange,
    DiffPreview,
    ReplaceExact,
    WriteNewFile,
    CreateDirectory,
    ApplyPatch,
    ConfigureClaudeDesktop,
    Serve,
}
