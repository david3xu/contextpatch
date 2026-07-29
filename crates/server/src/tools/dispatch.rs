use std::path::Path;

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

    let result = match name {
        tools::capability_manifest::NAME => tools::capability::call_capability_manifest(repo_root),
        tools::preflight_health::NAME => tools::capability::call_preflight_health(repo_root),
        tools::read_range::NAME => tools::files::call_read_range(repo_root, &arguments),
        tools::diff_preview::NAME => tools::files::call_diff_preview(repo_root, &arguments),
        tools::replace_exact::NAME => tools::files::call_replace_exact(repo_root, &arguments),
        tools::status_guard::NAME => tools::files::call_status_guard(repo_root, &arguments),
        tools::write_new_file::NAME => tools::files::call_write_new_file(repo_root, &arguments),
        tools::write_new_file_base64::NAME => {
            tools::files::call_write_new_file_base64(repo_root, &arguments)
        }
        tools::write_existing_file_exact_hash::NAME => {
            tools::files::call_write_existing_file_exact_hash(repo_root, &arguments)
        }
        tools::artifact_write_text::NAME => {
            tools::files::call_artifact_write_text(repo_root, &arguments)
        }
        tools::artifact_write_base64::NAME => {
            tools::files::call_artifact_write_base64(repo_root, &arguments)
        }
        tools::bulk_write_new_files_base64::NAME => {
            tools::files::call_bulk_write_new_files_base64(repo_root, &arguments)
        }
        tools::create_directory::NAME => tools::files::call_create_directory(repo_root, &arguments),
        tools::run_guarded_command::NAME => {
            tools::process::call_run_guarded_command(repo_root, &arguments)
        }
        tools::fixture_generator_run::NAME => {
            tools::fixtures::call_fixture_generator_run(repo_root, &arguments)
        }
        tools::base_image_check_run::NAME => {
            tools::fixtures::call_base_image_check_run(repo_root, &arguments)
        }
        tools::fixture_manifest_verify::NAME => {
            tools::fixtures::call_fixture_manifest_verify(repo_root, &arguments)
        }
        tools::fixture_manifest_refresh::NAME => {
            tools::fixtures::call_fixture_manifest_refresh(repo_root, &arguments)
        }
        tools::read_command_log::NAME => tools::process::call_read_command_log(&arguments),
        tools::validation_profile_run::NAME => {
            tools::process::call_validation_profile_run(repo_root, &arguments)
        }
        tools::setup_profile_run::NAME => {
            tools::setup::call_setup_profile_run(repo_root, &arguments)
        }
        tools::native_build_run::NAME => {
            tools::native::call_native_build_run(repo_root, &arguments)
        }
        tools::native_device_run::NAME => {
            tools::native::call_native_device_run(repo_root, &arguments)
        }
        tools::git_commit_exact::NAME => {
            tools::git::handlers::call_git_commit_exact(repo_root, &arguments)
        }
        tools::git_commit_scoped::NAME => {
            tools::git::handlers::call_git_commit_scoped(repo_root, &arguments)
        }
        tools::git_restore_exact::NAME => {
            tools::git::handlers::call_git_restore_exact(repo_root, &arguments)
        }
        tools::delete_untracked_exact::NAME => {
            tools::git::handlers::call_delete_untracked_exact(repo_root, &arguments)
        }
        tools::git_remote_list::NAME => tools::git::handlers::call_git_remote_list(repo_root),
        tools::git_remote_check::NAME => {
            tools::git::handlers::call_git_remote_check(repo_root, &arguments)
        }
        tools::git_branch_prepare::NAME => {
            tools::git::handlers::call_git_branch_prepare(repo_root, &arguments)
        }
        tools::git_merge_readiness::NAME => {
            tools::git::handlers::call_git_merge_readiness(repo_root, &arguments)
        }
        tools::git_push_exact::NAME => {
            tools::git::handlers::call_git_push_exact(repo_root, &arguments)
        }
        tools::github_pr_run::NAME => tools::github::call_github_pr_run(repo_root, &arguments),
        tools::github_fork_prepare::NAME => {
            tools::github::call_github_fork_prepare(repo_root, &arguments)
        }
        unknown => Err(format!("unknown tool: {unknown}")),
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
