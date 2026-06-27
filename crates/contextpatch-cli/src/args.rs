#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Command {
    Help,
    Version,
    Status,
    ReadRange,
    DiffPreview,
    ReplaceExact,
    ApplyPatch,
    Serve,
}
