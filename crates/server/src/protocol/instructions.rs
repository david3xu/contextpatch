//! Concise orientation surfaced to clients during MCP initialization.

use crate::tools::ToolSurface;

const FULL_CLIENT_INSTRUCTIONS: &str = "\
Call capability_manifest first, preferably with names_only=true or one section. Before declaring a \
tool unavailable, compare build.git_sha with the checked-out commit; a stale installed binary may \
predate the capability. Follow each tool's exact anchors, hashes, dry-run, and confirmation gates; raw \
shell access is not available. Use artifact_write_text for non-repository scratch files. Read refusal \
messages for the safe alternative. After an interrupted or timed-out mutation, call \
read_write_receipts before retrying.";

const PROJECT_CLIENT_INSTRUCTIONS: &str = "\
Call project_execute first with action=\"describe\". Omit arguments.name to list actions, or provide \
one action name to retrieve its exact schema and annotations. Execute exactly one action per call by \
passing its original arguments unchanged. The wrapper preserves repository confinement, hashes, \
confirmations, dry runs, deadlines, mutation locks, receipts, and command allowlists. After an \
interrupted or timed-out mutation, inspect receipts and current state before retrying.";

pub(crate) const fn client_instructions(surface: ToolSurface) -> &'static str {
    match surface {
        ToolSurface::Full => FULL_CLIENT_INSTRUCTIONS,
        ToolSurface::Project => PROJECT_CLIENT_INSTRUCTIONS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_at_the_discovery_tool_first() {
        // Orientation is only useful if the first thing it says is where to look, so this is asserted
        // rather than left to reviewer discipline.
        assert!(
            client_instructions(ToolSurface::Full).starts_with("Call capability_manifest first")
        );
        assert!(client_instructions(ToolSurface::Project).starts_with("Call project_execute first"));
    }

    #[test]
    fn covers_the_failures_that_produce_false_negatives() {
        for expected in [
            "build.git_sha",
            "stale install",
            "dry-run",
            "read_write_receipts",
            "artifact_write_text",
        ] {
            assert!(
                client_instructions(ToolSurface::Full).contains(expected),
                "instructions omit {expected}"
            );
        }
        assert!(client_instructions(ToolSurface::Project).contains("action=\"describe\""));
        assert!(client_instructions(ToolSurface::Project).contains("exactly one action"));
    }

    #[test]
    fn stays_short_enough_to_prepend_every_session() {
        assert!(
            client_instructions(ToolSurface::Full).len() < 700,
            "instructions grew to {} bytes; move detail into capability_manifest instead",
            client_instructions(ToolSurface::Full).len()
        );
        assert!(client_instructions(ToolSurface::Project).len() < 700);
    }
}
