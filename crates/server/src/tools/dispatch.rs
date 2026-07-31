use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use crate::protocol::response::{error_response, success_response};
use crate::tools;

pub(crate) fn handle_tool_call(repo_root: &Path, id: Value, request: &Value) -> String {
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

    let result = match deadline_for(name) {
        Some(limit) => {
            let owned_root = repo_root.to_path_buf();
            let owned_name = name.to_string();
            let owned_arguments = arguments.clone();
            contextpatch_core::process::deadline::with_deadline(name, limit, move || {
                call_tool_with_mutation_lock(&owned_root, &owned_name, &owned_arguments)
            })
            .into_result()
            .map_err(|error| format!("{name} refused: {error}"))
            .and_then(|result| result)
        }
        None => call_tool_with_mutation_lock(repo_root, name, &arguments),
    };

    match result {
        Ok(text) => success_response(
            id,
            json!({
                "content": [
                    {
                        "type": "text",
                        "text": text
                    }
                ]
            }),
        ),
        Err(message) => success_response(
            id,
            json!({
                "isError": true,
                "content": [
                    {
                        "type": "text",
                        "text": message
                    }
                ]
            }),
        ),
    }
}

fn call_tool_with_mutation_lock(
    repo_root: &Path,
    name: &str,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let _mutation_lock = if serializes_repository_mutation(name) {
        Some(
            contextpatch_core::fs::mutation_lock::try_repository_mutation_lock(repo_root)
                .map_err(|error| format!("{name} refused: {error}"))?,
        )
    } else {
        None
    };
    call_tool(repo_root, name, arguments)
}

fn call_tool(
    repo_root: &Path,
    name: &str,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    match name {
        tools::capability_manifest::NAME => {
            tools::capability::call_capability_manifest(repo_root, arguments)
        }
        tools::preflight_health::NAME => tools::capability::call_preflight_health(repo_root),
        tools::read_range::NAME => tools::files::call_read_range(repo_root, arguments),
        tools::diff_preview::NAME => tools::files::call_diff_preview(repo_root, arguments),
        tools::replace_exact::NAME => tools::files::call_replace_exact(repo_root, arguments),
        tools::bulk_replace_exact::NAME => {
            tools::files::call_bulk_replace_exact(repo_root, arguments)
        }
        tools::read_write_receipts::NAME => {
            tools::files::call_read_write_receipts(repo_root, arguments)
        }
        tools::status_guard::NAME => tools::files::call_status_guard(repo_root, arguments),
        tools::file_info::NAME => tools::files::call_file_info(repo_root, arguments),
        tools::list_directory::NAME => tools::files::call_list_directory(repo_root, arguments),
        tools::read_file_bytes::NAME => tools::files::call_read_file_bytes(repo_root, arguments),
        tools::write_new_file::NAME => tools::files::call_write_new_file(repo_root, arguments),
        tools::write_new_file_base64::NAME => {
            tools::files::call_write_new_file_base64(repo_root, arguments)
        }
        tools::write_existing_file_exact_hash::NAME => {
            tools::files::call_write_existing_file_exact_hash(repo_root, arguments)
        }
        tools::artifact_write_text::NAME => {
            tools::files::call_artifact_write_text(repo_root, arguments)
        }
        tools::artifact_delete_exact::NAME => {
            tools::files::call_artifact_delete_exact(repo_root, arguments)
        }
        tools::artifact_write_base64::NAME => {
            tools::files::call_artifact_write_base64(repo_root, arguments)
        }
        tools::bulk_write_new_files_base64::NAME => {
            tools::files::call_bulk_write_new_files_base64(repo_root, arguments)
        }
        tools::create_directory::NAME => tools::files::call_create_directory(repo_root, arguments),
        tools::run_guarded_command::NAME => {
            tools::process::call_run_guarded_command(repo_root, arguments)
        }
        tools::fixture_generator_run::NAME => {
            tools::fixtures::call_fixture_generator_run(repo_root, arguments)
        }
        tools::base_image_check_run::NAME => {
            tools::fixtures::call_base_image_check_run(repo_root, arguments)
        }
        tools::fixture_manifest_verify::NAME => {
            tools::fixtures::call_fixture_manifest_verify(repo_root, arguments)
        }
        tools::fixture_manifest_refresh::NAME => {
            tools::fixtures::call_fixture_manifest_refresh(repo_root, arguments)
        }
        tools::read_command_log::NAME => tools::process::call_read_command_log(arguments),
        tools::image_cleanliness_check_run::NAME => {
            tools::process::call_image_cleanliness_check_run(arguments)
        }
        tools::docker_image_inspect::NAME => tools::process::call_docker_image_inspect(arguments),
        tools::artifact_python_run::NAME => {
            tools::process::call_artifact_python_run(repo_root, arguments)
        }
        tools::validation_profile_run::NAME => {
            tools::process::call_validation_profile_run(repo_root, arguments)
        }
        tools::setup_profile_run::NAME => {
            tools::setup::call_setup_profile_run(repo_root, arguments)
        }
        tools::native_build_run::NAME => tools::native::call_native_build_run(repo_root, arguments),
        tools::native_device_run::NAME => {
            tools::native::call_native_device_run(repo_root, arguments)
        }
        tools::git_commit_exact::NAME => {
            tools::git::handlers::call_git_commit_exact(repo_root, arguments)
        }
        tools::git_commit_scoped::NAME => {
            tools::git::handlers::call_git_commit_scoped(repo_root, arguments)
        }
        tools::git_commit_prefix::NAME => {
            tools::git::handlers::call_git_commit_prefix(repo_root, arguments)
        }
        tools::git_stage_exact::NAME => {
            tools::git::handlers::call_git_stage_exact(repo_root, arguments)
        }
        tools::git_staged_scope_check::NAME => {
            tools::git::handlers::call_git_staged_scope_check(repo_root, arguments)
        }
        tools::git_restore_exact::NAME => {
            tools::git::handlers::call_git_restore_exact(repo_root, arguments)
        }
        tools::move_tracked::NAME => tools::git::handlers::call_move_tracked(repo_root, arguments),
        tools::delete_guarded::NAME => {
            tools::git::handlers::call_delete_guarded(repo_root, arguments)
        }
        tools::delete_untracked_exact::NAME => {
            tools::git::handlers::call_delete_untracked_exact(repo_root, arguments)
        }
        tools::delete_generated_prefix::NAME => {
            tools::git::handlers::call_delete_generated_prefix(repo_root, arguments)
        }
        tools::git_remote_list::NAME => tools::git::handlers::call_git_remote_list(repo_root),
        tools::git_remote_check::NAME => {
            tools::git::handlers::call_git_remote_check(repo_root, arguments)
        }
        tools::git_branch_prepare::NAME => {
            tools::git::handlers::call_git_branch_prepare(repo_root, arguments)
        }
        tools::git_merge_readiness::NAME => {
            tools::git::handlers::call_git_merge_readiness(repo_root, arguments)
        }
        tools::git_push_exact::NAME => {
            tools::git::handlers::call_git_push_exact(repo_root, arguments)
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
