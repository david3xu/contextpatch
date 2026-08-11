use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use crate::protocol::response::{error_response, success_response};
use crate::tools;
use crate::tools::project::ProjectCall;
use crate::tools::ToolSurface;

const MAX_SERIALIZED_TOOL_RESPONSE_BYTES: usize = 900 * 1024;
const MAX_FALLBACK_TOOL_NAME_BYTES: usize = 256;

pub(crate) fn handle_tool_call(
    repo_root: &Path,
    surface: ToolSurface,
    id: Value,
    request: &Value,
) -> String {
    let Some(params) = request.get("params") else {
        return error_response(id, -32602, "tools/call missing params");
    };
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_response(id, -32602, "tools/call missing tool name");
    };
    let arguments = params
        .get("arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let resolved = match surface {
        ToolSurface::Full => Ok(ProjectCall::Execute {
            name: name.to_string(),
            arguments,
            repository: None,
        }),
        ToolSurface::Project if name == tools::project_execute::NAME => {
            tools::project::resolve(&arguments)
        }
        ToolSurface::Project => Err(format!(
            "unknown tool for project surface: {name}; use project_execute"
        )),
    };

    let result = match resolved {
        Err(message) => Err(message),
        Ok(ProjectCall::Describe { text, repository }) => {
            effective_repository(repo_root, repository.as_deref()).map(|_| text)
        }
        Ok(ProjectCall::Execute {
            name,
            arguments,
            repository,
        }) => effective_repository(repo_root, repository.as_deref())
            .and_then(|effective| execute_tool(effective, surface, &name, &arguments)),
    };

    bounded_tool_result_response(id, name, result)
}

/// The repository one call operates on.
///
/// Owning, because a selection holds the directory descriptor that anchors it and that descriptor has to
/// stay open for as long as the call can still run. Both exposed accessors are typed: [`Self::root`]
/// carries whichever authority the call has, and [`Self::git_repository`] narrows it to the Git
/// projection. Nothing here hands out an optional descriptor or a bare path, so a caller cannot
/// accidentally treat an anchored repository as a path-backed one, and nothing here reopens a
/// selected root.
pub(crate) enum EffectiveRepository {
    /// No selector was supplied, so the configured root is the target.
    Configured(std::path::PathBuf),
    /// A validated workspace selection, holding its descriptor open.
    #[cfg(unix)]
    Selected(contextpatch_core::git::SelectedRepository),
}

impl EffectiveRepository {
    /// The repository root this call operates on, carrying whichever authority it has.
    ///
    /// The general form. Projections for Git and, later, for the filesystem boundaries are reached through
    /// it, which is what will let those boundaries stop resolving names without another plumbing change.
    pub(crate) fn root(&self) -> contextpatch_core::git::RepositoryRoot<'_> {
        match self {
            Self::Configured(path) => contextpatch_core::git::RepositoryRoot::from_path(path),
            #[cfg(unix)]
            Self::Selected(selected) => selected.root(),
        }
    }

    /// The typed target for migrated Git policy, descriptor-backed when this is a selection.
    pub(crate) fn git_repository(&self) -> contextpatch_core::git::GitRepository<'_> {
        self.root().git()
    }
}

/// Decide which repository a call targets, revalidating a selection before it is handed over.
fn effective_repository(
    configured_root: &Path,
    repository: Option<&str>,
) -> Result<EffectiveRepository, String> {
    let Some(repository) = repository else {
        return Ok(EffectiveRepository::Configured(
            configured_root.to_path_buf(),
        ));
    };

    let selected = contextpatch_core::git::select_workspace_repository(configured_root, repository)
        .map_err(|error| format!("project_execute refused: {error}"))?;
    // Reproved immediately before use, so a directory replaced after validation is refused rather than
    // operated on.
    selected
        .revalidate()
        .map_err(|error| format!("project_execute refused: {error}"))?;

    #[cfg(unix)]
    {
        Ok(EffectiveRepository::Selected(selected))
    }
    #[cfg(not(unix))]
    {
        let _ = selected;
        Err("project_execute refused: repository selection requires descriptor-relative directory access, which is unavailable on this platform".to_string())
    }
}

fn bounded_tool_result_response(
    id: Value,
    tool_name: &str,
    result: Result<String, String>,
) -> String {
    let (is_error, text) = match result {
        Ok(text) => (false, text),
        Err(message) => (true, message),
    };
    let result = tool_result(text, is_error);
    let response = success_response(id.clone(), result);
    if response.len() <= MAX_SERIALIZED_TOOL_RESPONSE_BYTES {
        return response;
    }

    let measured_response_bytes = response.len();
    let (fallback_tool_name, tool_name_truncated) =
        bounded_utf8_prefix(tool_name, MAX_FALLBACK_TOOL_NAME_BYTES);
    let fallback_text = if is_error {
        json!({
            "tool": fallback_tool_name,
            "tool_name_truncated": tool_name_truncated,
            "handler_result": "error",
            "diagnostic_omitted": true,
            "reason": "the serialized diagnostic exceeded ContextPatch's MCP response limit",
            "measured_response_bytes": measured_response_bytes,
            "max_response_bytes": MAX_SERIALIZED_TOOL_RESPONSE_BYTES,
            "next_step": "Use narrower, paged, compact, or minimal arguments. If the action can mutate state, inspect current state before retrying."
        })
        .to_string()
    } else {
        json!({
            "tool": fallback_tool_name,
            "tool_name_truncated": tool_name_truncated,
            "handler_result": "success",
            "output_omitted": true,
            "reason": "the serialized output exceeded ContextPatch's MCP response limit",
            "measured_response_bytes": measured_response_bytes,
            "max_response_bytes": MAX_SERIALIZED_TOOL_RESPONSE_BYTES,
            "next_step": "Use narrower, paged, compact, or minimal arguments.",
            "retry_warning": "The action completed successfully; do not retry a mutation solely because its output was omitted."
        })
        .to_string()
    };
    success_response(id, tool_result(fallback_text, is_error))
}

fn bounded_utf8_prefix(value: &str, max_bytes: usize) -> (&str, bool) {
    if value.len() <= max_bytes {
        return (value, false);
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (&value[..end], true)
}

fn tool_result(text: String, is_error: bool) -> Value {
    let mut result = json!({
        "content": [
            {
                "type": "text",
                "text": text
            }
        ]
    });
    if is_error {
        result["isError"] = Value::Bool(true);
    }
    result
}

fn execute_tool(
    repository: EffectiveRepository,
    surface: ToolSurface,
    name: &str,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    tools::schema::validate_internal_action_arguments(name, arguments)?;
    match deadline_for(name) {
        Some(limit) => {
            // The repository moves into the deadline thread, which is what keeps a selection's descriptor
            // open for as long as the call can still be running.
            let owned_name = name.to_string();
            let owned_arguments = arguments.clone();
            contextpatch_core::process::deadline::with_deadline(name, limit, move || {
                call_tool_with_mutation_lock(&repository, surface, &owned_name, &owned_arguments)
            })
            .into_result()
            .map_err(|error| format!("{name} refused: {error}"))
            .and_then(|result| result)
        }
        None => call_tool_with_mutation_lock(&repository, surface, name, arguments),
    }
}

fn call_tool_with_mutation_lock(
    repository: &EffectiveRepository,
    surface: ToolSurface,
    name: &str,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let _mutation_lock = if serializes_repository_mutation(name) {
        Some(
            // Keyed by repository identity through the selected root's own authority, so a rename keeps a
            // repository excluding itself and a replacement at the old name does not inherit its lock.
            contextpatch_core::fs::mutation_lock::try_repository_mutation_lock(repository.root())
                .map_err(|error| format!("{name} refused: {error}"))?,
        )
    } else {
        None
    };
    call_tool(repository, surface, name, arguments)
}

fn call_tool(
    repository: &EffectiveRepository,
    surface: ToolSurface,
    name: &str,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let Some(entry) = crate::tools::registry::descriptor(name) else {
        return Err(format!("unknown tool: {name}"));
    };
    (entry.handler)(repository, surface, arguments).map_err(|error| attribute(entry.name, error))
}

/// The exact text every handler writes between its own name and its reason. Matching on it is what
/// makes the check below a boundary rather than a bare prefix.
const REFUSAL_MARKER: &str = " refused:";

/// Ensure a refusal names the tool that produced it.
///
/// The shared argument helpers cannot do this themselves: `required_string` is handed a key and no
/// tool name, so it can only say `missing or invalid string argument: path`. Without attribution a
/// caller that mis-invoked most of the server learned neither which tool refused nor, in a batch,
/// which call the refusal belonged to. Measured by calling every registered tool through the server
/// with empty arguments and searching each reply for the tool's own name: of the twenty tools that
/// refuse empty arguments, all twenty name themselves and all twenty replies are distinct.
///
/// Attributing here rather than at each call site fixes every tool at once and cannot be forgotten
/// by a new one. Handlers that already name themselves are left exactly as they are, so no existing
/// refusal text changes.
///
/// The match is on the name followed by [`REFUSAL_MARKER`], never on the name alone. A bare prefix
/// lets one tool's reply pass through wearing another tool's name wherever one name is a prefix of
/// another, which `write_new_file` and `write_new_file_base64` already are. The marker makes this
/// correct whether or not such a pair exists, so it is load-bearing rather than a second guard on
/// the same hole, and removing it as redundant would reopen the misattribution.
fn attribute(name: &str, error: String) -> String {
    if error.starts_with(&format!("{name}{REFUSAL_MARKER}")) {
        return error;
    }
    format!("{name}{REFUSAL_MARKER} {error}")
}

/// The reply deadline for one tool, or `None` for work that returns a pollable log id.
fn deadline_for(name: &str) -> Option<Duration> {
    crate::tools::registry::descriptor(name).and_then(|entry| entry.deadline)
}

fn serializes_repository_mutation(name: &str) -> bool {
    crate::tools::registry::descriptor(name).is_some_and(|entry| entry.serializes_mutation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use contextpatch_core::process::deadline::{GIT_DEADLINE, READ_DEADLINE, WRITE_DEADLINE};

    #[test]
    fn direct_operations_have_bounded_reply_deadlines() {
        assert_eq!(deadline_for(tools::read_range::NAME), Some(READ_DEADLINE));
        assert_eq!(
            deadline_for(tools::replace_exact::NAME),
            Some(WRITE_DEADLINE)
        );
        assert_eq!(
            deadline_for(tools::git_commit_exact::NAME),
            Some(GIT_DEADLINE)
        );
        assert_eq!(deadline_for(tools::github_pr_run::NAME), Some(GIT_DEADLINE));
        assert_eq!(
            deadline_for(tools::github_fork_prepare::NAME),
            Some(GIT_DEADLINE)
        );
        assert!(serializes_repository_mutation(
            tools::git_commit_exact::NAME
        ));
        assert!(!serializes_repository_mutation(
            tools::read_write_receipts::NAME
        ));
    }

    #[test]
    fn process_tools_keep_their_operation_specific_timeouts() {
        assert_eq!(deadline_for(tools::run_guarded_command::NAME), None);
        assert_eq!(deadline_for(tools::validation_profile_run::NAME), None);
        assert_eq!(deadline_for(tools::native_build_run::NAME), None);
    }

    /// Every tool against every behavioural axis, recorded before the registry migration.
    ///
    /// These four facts are decided in four different functions across two files today, and the
    /// registry collapses them into fields on one descriptor. Nothing else checks that they survive
    /// that move: a tool that silently loses its mutation lock or its deadline still passes every
    /// behavioural test, because the tests exercise tools one at a time and none of them asserts the
    /// classification itself.
    ///
    /// File locations are deliberately excluded. Which module owns a handler is organisation, not
    /// contract, and it changes on purpose when the oversized modules are split.
    #[test]
    fn the_per_tool_classification_matrix_matches_its_recorded_snapshot() {
        use contextpatch_core::process::deadline::{GIT_DEADLINE, READ_DEADLINE, WRITE_DEADLINE};

        let mut names = crate::tools::schema::internal_action_names();
        names.push(tools::project_execute::NAME.to_string());
        names.sort();
        names.dedup();

        let mut rendered = String::from("tool\tdeadline\tlock\treach\tread_only\n");
        for name in &names {
            let deadline = match deadline_for(name) {
                Some(limit) if limit == READ_DEADLINE => "read",
                Some(limit) if limit == WRITE_DEADLINE => "write",
                Some(limit) if limit == GIT_DEADLINE => "git",
                Some(_) => "other",
                None => "none",
            };
            rendered.push_str(&format!(
                "{name}\t{deadline}\t{}\t{:?}\t{}\n",
                if serializes_repository_mutation(name) {
                    "yes"
                } else {
                    "no"
                },
                crate::tools::schema::remote_reach(name),
                if crate::tools::schema::is_read_only(name) {
                    "yes"
                } else {
                    "no"
                }
            ));
        }

        crate::tools::snapshot_fixture::assert_matches("tool-matrix.tsv", &rendered);
    }

    /// Pin which files read `logical_path()`, and how many times each does.
    ///
    /// Operations derive authority from a retained directory descriptor, not from a pathname. Three
    /// sites still read the logical path, all deliberately and none to reach a file: `github.rs`
    /// reports `cwd` back to a caller twice, because a caller asking which directory was used cannot
    /// be handed a descriptor, and `restore.rs` uses it once as receipt identity, which is
    /// path-derived by design and is an acknowledged deferred boundary.
    ///
    /// Those three were previously recorded as line numbers in two documents, which is the staleness
    /// this whole exercise removed elsewhere: the numbers are wrong after any edit above them. A map
    /// of file to count survives edits within a file, which is the point, and is compared whole
    /// rather than summed so that relocating a reader between files fails here and has to be argued
    /// for rather than absorbed into an unchanged total.
    #[test]
    fn the_files_that_read_a_logical_path_are_the_known_ones() {
        // Assembled rather than written whole so this file does not match its own scan. Writing the
        // literal here would add `dispatch.rs` to the observed set and quietly make the recorded map
        // describe the test instead of the code it is pinning.
        const READER: &str = concat!(".logical_", "path()");
        const KNOWN_READERS: &[(&str, usize)] = &[
            ("tools/git/handlers/restore.rs", 1),
            ("tools/github.rs", 2),
        ];

        // A unit test in this binary runs with `CARGO_MANIFEST_DIR` at `crates/server`, so the scan
        // root is this crate's own sources. `canonical_label` reads the logical path inside `core`,
        // which is a different crate and is not in scope here.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut observed: Vec<(String, usize)> = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(directory) = pending.pop() {
            let entries = std::fs::read_dir(&directory).expect("the source tree must be readable");
            for entry in entries {
                let path = entry.expect("a source entry must be readable").path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("a source file must be readable");
                let count = source.matches(READER).count();
                if count > 0 {
                    let relative = path
                        .strip_prefix(&root)
                        .expect("every scanned file is under the scan root")
                        .to_string_lossy()
                        .replace('\\', "/");
                    observed.push((relative, count));
                }
            }
        }
        observed.sort();

        let known: Vec<(String, usize)> = KNOWN_READERS
            .iter()
            .map(|(file, count)| ((*file).to_string(), *count))
            .collect();

        assert_eq!(
            observed, known,
            "the set of files reading a logical path has changed; a new reader for reporting or \
             receipt identity needs the same justification the existing three carry, and a reader \
             added to gain access to a file is a regression"
        );
    }
}
