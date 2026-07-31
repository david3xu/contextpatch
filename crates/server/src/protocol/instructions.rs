//! Concise orientation surfaced to clients during MCP initialization.

/// Orientation text returned from `initialize`.
pub const CLIENT_INSTRUCTIONS: &str = "\
Call capability_manifest first, preferably with names_only=true or one section. Before declaring a \
tool unavailable, compare build.git_sha with the checked-out commit; a stale installed binary may \
predate the capability. Follow each tool's exact anchors, hashes, dry-run, and confirmation gates; raw \
shell access is not available. Use artifact_write_text for non-repository scratch files. Read refusal \
messages for the safe alternative. After an interrupted or timed-out mutation, call \
read_write_receipts before retrying.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_at_the_discovery_tool_first() {
        // Orientation is only useful if the first thing it says is where to look, so this is asserted
        // rather than left to reviewer discipline.
        assert!(CLIENT_INSTRUCTIONS.starts_with("Call capability_manifest first"));
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
                CLIENT_INSTRUCTIONS.contains(expected),
                "instructions omit {expected}"
            );
        }
    }

    #[test]
    fn stays_short_enough_to_prepend_every_session() {
        assert!(
            CLIENT_INSTRUCTIONS.len() < 700,
            "instructions grew to {} bytes; move detail into capability_manifest instead",
            CLIENT_INSTRUCTIONS.len()
        );
    }
}
