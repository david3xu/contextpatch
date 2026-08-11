//! Concise orientation surfaced to clients during MCP initialization.
//!
//! These strings name tools only where the client has to *call* one. They deliberately do not
//! enumerate which tools behave asynchronously: that list was written when there were three, fell
//! behind when there were five, and would fall behind again. A client does not need to know which
//! actions are asynchronous, it needs to know what to do when a reply says so, and phrasing the rule
//! against the reply shape rather than against a tool list cannot go stale.

use crate::tools::ToolSurface;

const FULL_CLIENT_INSTRUCTIONS: &str = "\
Call capability_manifest first. Before declaring a tool unavailable, compare build.git_sha with the \
checkout; a stale install may predate it. Follow exact anchors, hashes, dry-run, and confirmation; raw \
shell access is unavailable. Tool replies can arrive out of order: correlate by JSON-RPC id. Any reply \
with status=\"running\" carries a log_id; poll it on the same server with read_command_log. Polling \
never restarts work; a restart makes active logs unknown. Use artifact_write_text for non-repo \
scratch. After interrupted or timed-out mutations, call read_write_receipts before retrying.";

const PROJECT_CLIENT_INSTRUCTIONS: &str = "\
Call project_execute first with action=\"describe\". Omit arguments.name to list actions, or provide \
one action name for its exact schema. Execute one action per call with its original arguments. \
repository may select a normalized exact child Git worktree. All guards remain. Tool replies can \
arrive out of order: correlate by JSON-RPC id. Any reply with status=\"running\" carries a log_id; \
poll it on the same server through read_command_log. Polling never restarts work; a restart makes \
active logs unknown. After interrupted or timed-out mutations, inspect receipts and current state \
before retrying.";

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
            "JSON-RPC id",
            "status=\"running\"",
            "read_command_log",
        ] {
            assert!(
                client_instructions(ToolSurface::Full).contains(expected),
                "instructions omit {expected}"
            );
        }
        assert!(client_instructions(ToolSurface::Project).contains("action=\"describe\""));
        assert!(client_instructions(ToolSurface::Project).contains("one action per call"));
        assert!(client_instructions(ToolSurface::Project).contains("JSON-RPC id"));
        assert!(client_instructions(ToolSurface::Project).contains("read_command_log"));
    }

    /// The asynchronous rule must be stated against the reply shape, never as a tool list.
    ///
    /// The previous wording named three of the five tools that return a `log_id`, and stayed wrong
    /// through two additions because nothing pointed at it. Any subset is wrong the moment the set
    /// changes, so naming an asynchronous producer here is the defect, not an omission.
    #[test]
    fn the_asynchronous_rule_names_no_tools_that_produce_log_ids() {
        for surface in [ToolSurface::Full, ToolSurface::Project] {
            let text = client_instructions(surface);
            for producer in crate::tools::registry::background_job_tools() {
                assert!(
                    !text.contains(producer),
                    "instructions enumerate {producer}; state the rule against status=\"running\" \
                     instead, or the list goes stale on the next asynchronous tool"
                );
            }
            assert!(text.contains("status=\"running\""));
            assert!(text.contains(crate::tools::read_command_log::NAME));
        }
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
