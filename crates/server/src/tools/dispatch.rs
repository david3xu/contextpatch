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
/// stay open for as long as the call can still run. Only two accessors are exposed: the logical path, for
/// messages and for tools not yet migrated, and the typed target, which carries the descriptor when there
/// is one. Nothing here hands out an optional descriptor, so a caller cannot accidentally treat an
/// anchored repository as a path-backed one, and nothing here reopens a selected root.
pub(crate) enum EffectiveRepository {
    /// No selector was supplied, so the configured root is the target.
    Configured(std::path::PathBuf),
    /// A validated workspace selection, holding its descriptor open.
    #[cfg(unix)]
    Selected(contextpatch_core::git::SelectedRepository),
}

impl EffectiveRepository {
    /// The path for messages, receipts, and tools that still take one.
    pub(crate) fn logical_path(&self) -> &Path {
        match self {
            Self::Configured(path) => path,
            #[cfg(unix)]
            Self::Selected(selected) => selected.path(),
        }
    }

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
            // Still keyed by path. Descriptor-anchored locking is deferred to the lock slice, so this
            // takes the logical path rather than pretending to be anchored.
            contextpatch_core::fs::mutation_lock::try_repository_mutation_lock(
                repository.logical_path(),
            )
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
    // Tools not yet migrated still take a path, and receive the logical path until each one moves to the
    // typed target. That is what lets this proceed one tool at a time instead of all at once.
    let repo_root = repository.logical_path();
    match name {
        tools::capability_manifest::NAME => {
            tools::capability::call_capability_manifest(repo_root, arguments, surface)
        }
        tools::preflight_health::NAME => {
            tools::capability::call_preflight_health(repo_root, arguments)
        }
        tools::read_range::NAME => tools::files::call_read_range(repository.root(), arguments),
        tools::diff_preview::NAME => tools::files::call_diff_preview(repository.root(), arguments),
        tools::replace_exact::NAME => {
            tools::files::call_replace_exact(repository.root(), arguments)
        }
        tools::bulk_replace_exact::NAME => {
            tools::files::call_bulk_replace_exact(repository.root(), arguments)
        }
        tools::read_write_receipts::NAME => {
            tools::files::call_read_write_receipts(repository.root(), arguments)
        }
        tools::status_guard::NAME => tools::files::call_status_guard(repository.root(), arguments),
        tools::file_info::NAME => tools::files::call_file_info(repository.root(), arguments),
        tools::set_file_executable::NAME => {
            tools::files::call_set_file_executable(repository.root(), arguments)
        }
        tools::list_directory::NAME => {
            tools::files::call_list_directory(repository.root(), arguments)
        }
        tools::read_file_bytes::NAME => {
            tools::files::call_read_file_bytes(repository.root(), arguments)
        }
        tools::write_new_file::NAME => {
            tools::files::call_write_new_file(repository.root(), arguments)
        }
        tools::write_new_file_base64::NAME => {
            tools::files::call_write_new_file_base64(repository.root(), arguments)
        }
        tools::write_existing_file_exact_hash::NAME => {
            tools::files::call_write_existing_file_exact_hash(repository.root(), arguments)
        }
        tools::artifact_write_text::NAME => {
            tools::files::call_artifact_write_text(repository.root(), arguments)
        }
        tools::artifact_delete_exact::NAME => {
            tools::files::call_artifact_delete_exact(repository.root(), arguments)
        }
        tools::artifact_write_base64::NAME => {
            tools::files::call_artifact_write_base64(repository.root(), arguments)
        }
        tools::bulk_write_new_files_base64::NAME => {
            tools::files::call_bulk_write_new_files_base64(repository.root(), arguments)
        }
        tools::create_directory::NAME => {
            tools::files::call_create_directory(repository.root(), arguments)
        }
        tools::run_guarded_command::NAME => {
            tools::process::call_run_guarded_command(repository.root(), arguments)
        }
        tools::fixture_generator_run::NAME => {
            tools::fixtures::call_fixture_generator_run(repository.root(), arguments)
        }
        tools::base_image_check_run::NAME => {
            tools::fixtures::call_base_image_check_run(repository.root(), arguments)
        }
        tools::fixture_manifest_verify::NAME => {
            tools::fixtures::call_fixture_manifest_verify(repository.root(), arguments)
        }
        tools::fixture_manifest_refresh::NAME => {
            tools::fixtures::call_fixture_manifest_refresh(repository.root(), arguments)
        }
        tools::read_command_log::NAME => tools::process::call_read_command_log(arguments),
        tools::image_cleanliness_check_run::NAME => {
            tools::process::call_image_cleanliness_check_run(arguments)
        }
        tools::docker_image_inspect::NAME => tools::process::call_docker_image_inspect(arguments),
        tools::artifact_python_run::NAME => {
            tools::process::call_artifact_python_run(repository.root(), arguments)
        }
        tools::task_image_python_run::NAME => {
            tools::process::call_task_image_python_run(repository.root(), arguments)
        }
        tools::harbor_run_start::NAME => {
            tools::process::call_harbor_run_start(repo_root, arguments)
        }
        tools::validation_profile_run::NAME => {
            tools::process::call_validation_profile_run(repo_root, arguments)
        }
        tools::setup_profile_run::NAME => {
            tools::setup::call_setup_profile_run(repository.root(), arguments)
        }
        tools::native_build_run::NAME => tools::native::call_native_build_run(repo_root, arguments),
        tools::native_device_run::NAME => {
            tools::native::call_native_device_run(repo_root, arguments)
        }
        tools::git_commit_exact::NAME => {
            tools::git::handlers::call_git_commit_exact(repository.root(), arguments)
        }
        tools::git_commit_scoped::NAME => {
            tools::git::handlers::call_git_commit_scoped(repository.root(), arguments)
        }
        tools::git_commit_prefix::NAME => {
            tools::git::handlers::call_git_commit_prefix(repository.root(), arguments)
        }
        tools::git_stage_exact::NAME => {
            tools::git::handlers::call_git_stage_exact(repository.root(), arguments)
        }
        tools::git_staged_scope_check::NAME => {
            tools::git::handlers::call_git_staged_scope_check(repository.root(), arguments)
        }
        tools::git_restore_exact::NAME => {
            tools::git::handlers::call_git_restore_exact(repository.root(), arguments)
        }
        tools::move_tracked::NAME => {
            tools::git::handlers::call_move_tracked(repository.root(), arguments)
        }
        tools::delete_guarded::NAME => {
            tools::git::handlers::call_delete_guarded(repository.root(), arguments)
        }
        tools::delete_untracked_exact::NAME => {
            tools::git::handlers::call_delete_untracked_exact(repository.root(), arguments)
        }
        tools::delete_generated_prefix::NAME => {
            tools::git::handlers::call_delete_generated_prefix(repository.root(), arguments)
        }
        tools::git_remote_list::NAME => {
            tools::git::handlers::call_git_remote_list(repository.git_repository())
        }
        tools::git_remote_check::NAME => {
            tools::git::handlers::call_git_remote_check(repository.git_repository(), arguments)
        }
        tools::git_branch_prepare::NAME => {
            tools::git::handlers::call_git_branch_prepare(repository.root(), arguments)
        }
        tools::git_merge_readiness::NAME => {
            tools::git::handlers::call_git_merge_readiness(repository.git_repository(), arguments)
        }
        tools::git_push_exact::NAME => {
            tools::git::handlers::call_git_push_exact(repository.git_repository(), arguments)
        }
        tools::github_pr_run::NAME => tools::github::call_github_pr_run(repo_root, arguments),
        tools::github_fork_prepare::NAME => {
            tools::github::call_github_fork_prepare(repo_root, arguments)
        }
        unknown => Err(format!("unknown tool: {unknown}")),
    }
}

fn deadline_for(name: &str) -> Option<Duration> {
    use contextpatch_core::process::deadline::{GIT_DEADLINE, READ_DEADLINE, WRITE_DEADLINE};

    match name {
        tools::capability_manifest::NAME
        | tools::preflight_health::NAME
        | tools::read_range::NAME
        | tools::read_write_receipts::NAME
        | tools::diff_preview::NAME
        | tools::status_guard::NAME
        | tools::file_info::NAME
        | tools::list_directory::NAME
        | tools::read_file_bytes::NAME
        | tools::fixture_manifest_verify::NAME
        | tools::read_command_log::NAME => Some(READ_DEADLINE),

        tools::replace_exact::NAME
        | tools::bulk_replace_exact::NAME
        | tools::write_new_file::NAME
        | tools::write_new_file_base64::NAME
        | tools::write_existing_file_exact_hash::NAME
        | tools::set_file_executable::NAME
        | tools::artifact_write_text::NAME
        | tools::artifact_delete_exact::NAME
        | tools::artifact_write_base64::NAME
        | tools::bulk_write_new_files_base64::NAME
        | tools::create_directory::NAME
        | tools::fixture_manifest_refresh::NAME => Some(WRITE_DEADLINE),

        tools::git_commit_exact::NAME
        | tools::git_commit_scoped::NAME
        | tools::git_commit_prefix::NAME
        | tools::git_stage_exact::NAME
        | tools::git_staged_scope_check::NAME
        | tools::git_restore_exact::NAME
        | tools::move_tracked::NAME
        | tools::delete_guarded::NAME
        | tools::delete_untracked_exact::NAME
        | tools::delete_generated_prefix::NAME
        | tools::git_remote_list::NAME
        | tools::git_remote_check::NAME
        | tools::git_branch_prepare::NAME
        | tools::git_merge_readiness::NAME
        | tools::git_push_exact::NAME
        | tools::github_pr_run::NAME
        | tools::github_fork_prepare::NAME => Some(GIT_DEADLINE),

        _ => None,
    }
}

fn serializes_repository_mutation(name: &str) -> bool {
    matches!(
        name,
        tools::replace_exact::NAME
            | tools::bulk_replace_exact::NAME
            | tools::write_new_file::NAME
            | tools::write_new_file_base64::NAME
            | tools::write_existing_file_exact_hash::NAME
            | tools::set_file_executable::NAME
            | tools::artifact_write_text::NAME
            | tools::artifact_delete_exact::NAME
            | tools::artifact_write_base64::NAME
            | tools::bulk_write_new_files_base64::NAME
            | tools::create_directory::NAME
            | tools::fixture_generator_run::NAME
            | tools::fixture_manifest_refresh::NAME
            | tools::setup_profile_run::NAME
            | tools::native_build_run::NAME
            | tools::native_device_run::NAME
            | tools::git_commit_exact::NAME
            | tools::git_commit_scoped::NAME
            | tools::git_commit_prefix::NAME
            | tools::git_stage_exact::NAME
            | tools::git_restore_exact::NAME
            | tools::move_tracked::NAME
            | tools::delete_guarded::NAME
            | tools::delete_untracked_exact::NAME
            | tools::delete_generated_prefix::NAME
            | tools::git_remote_check::NAME
            | tools::git_branch_prepare::NAME
            | tools::git_merge_readiness::NAME
            | tools::git_push_exact::NAME
            | tools::github_fork_prepare::NAME
    )
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
}
