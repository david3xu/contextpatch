//! One table describing every tool, replacing five parallel ones.
//!
//! A tool's identity used to be asserted in five independent places: its schema in a `schema/*.rs`
//! module, its handler in the `dispatch` match, its reply deadline in `deadline_for`, its
//! mutation-lock membership in `serializes_repository_mutation`, and its advertised authority in the
//! `authority` classifiers. Nothing checked those agreed, and adding a tool meant remembering all
//! five. The failure that produced was not a broken tool; it was a tool classified wrongly, or
//! described in one place and not another, while every test still passed.
//!
//! A descriptor states all of it once. This table is now the only answer: `dispatch` looks a tool up
//! here or refuses it as unknown, `deadline_for` and `serializes_repository_mutation` read a field,
//! and `schema::authority` classifies from `reach` and `read_only`. There is no fallback path left.
//!
//! `project_execute` is deliberately absent. It is the surface wrapper rather than an internal
//! action: resolved in `handle_tool_call` before the repository is determined, advertised only on
//! the project surface, and classified as the widest reach of everything it dispatches.
//!
//! # What guards this table
//!
//! Two recorded fixtures cover five of a descriptor's six facts — `tools-surface.json` pins name and
//! schema, `tool-matrix.tsv` pins deadline, lock, reach, and read-only. Both were captured before
//! the migration began and are byte-identical after it, which is the evidence that moving 54 tools
//! changed no advertised behaviour.
//!
//! The sixth fact, handler identity, is invisible to both: a descriptor pointing one tool at
//! another's handler leaves the fixtures unchanged. `each_descriptor_calls_the_handler_named_for_its_tool`
//! is what covers it, and the reasoning for why nothing cheaper works is recorded there.

use std::time::Duration;

use serde_json::Value;

use crate::tools::dispatch::EffectiveRepository;
use crate::tools::schema::RemoteReach;
use crate::tools::ToolSurface;

/// The uniform call shape every tool is adapted to.
///
/// Handlers themselves take three different argument shapes — most want the repository root and the
/// arguments, `capability_manifest` also needs the surface, and a few need neither — so descriptors
/// adapt with a non-capturing closure rather than forcing every handler to change signature. The
/// adaptation is visible in the table, which is where a reader looks anyway.
pub(crate) type ToolHandler = fn(
    &EffectiveRepository,
    ToolSurface,
    &serde_json::Map<String, Value>,
) -> Result<String, String>;

pub(crate) struct ToolDescriptor {
    pub(crate) name: &'static str,
    /// Whether the reply carries a log id to poll rather than a result. Recorded per tool because it
    /// is not derivable from anything else here: it describes what the handler returns, and
    /// `deadline: None` covers both this and the operation-specific-timeout case.
    pub(crate) starts_background_job: bool,
    /// The advertised schema, including its `inputSchema` and description. Annotations are added
    /// centrally from `reach` and `read_only`, so a descriptor cannot advertise an authority that
    /// disagrees with the one it is classified under.
    pub(crate) schema: fn() -> Value,
    pub(crate) handler: ToolHandler,
    /// Reply deadline, or `None` where no shared class bounds it. `None` alone does not mean the work
    /// is asynchronous: most tools carrying it hold their own operation-specific timeout and reply
    /// with a result. `starts_background_job` separates those from the pollable ones, and the absence
    /// of that separation is why the manifest's list of asynchronous tools had to be maintained by
    /// hand.
    pub(crate) deadline: Option<Duration>,
    pub(crate) reach: RemoteReach,
    pub(crate) read_only: bool,
    /// Whether this tool takes the cooperative per-repository mutation lock.
    pub(crate) serializes_mutation: bool,
}

/// Every tool this server dispatches. A name absent from this table is refused as unknown.
static REGISTRY: &[ToolDescriptor] = &[
    ToolDescriptor {
        name: crate::tools::capability_manifest::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::capability_manifest_definition,
        handler: |repository, surface, arguments| {
            crate::tools::capability::call_capability_manifest(
                repository.root(),
                arguments,
                surface,
            )
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::preflight_health::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::preflight_health_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::capability::call_preflight_health(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::setup_profile_run::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::setup_profile_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::setup::call_setup_profile_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::native_build_run::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::native_build_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::native::call_native_build_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::native_device_run::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::native_device_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::native::call_native_device_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::fixture_generator_run::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::fixture_generator_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::fixtures::call_fixture_generator_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::base_image_check_run::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::base_image_check_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::fixtures::call_base_image_check_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::fixture_manifest_verify::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::fixture_manifest_verify_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::fixtures::call_fixture_manifest_verify(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::fixture_manifest_refresh::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::fixture_manifest_refresh_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::fixtures::call_fixture_manifest_refresh(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::github_pr_run::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::github_pr_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::github::call_github_pr_run(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::DirectRemote,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::github_fork_prepare::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::github_fork_prepare_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::github::call_github_fork_prepare(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::DirectRemote,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::move_tracked::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::move_tracked_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_move_tracked(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::delete_guarded::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::delete_guarded_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_delete_guarded(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::git_commit_exact::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_commit_exact_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_commit_exact(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::git_commit_scoped::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_commit_scoped_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_commit_scoped(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::git_commit_prefix::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_commit_prefix_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_commit_prefix(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::git_stage_exact::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_stage_exact_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_stage_exact(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::git_staged_scope_check::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_staged_scope_check_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_staged_scope_check(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::git_restore_exact::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_restore_exact_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_restore_exact(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::delete_untracked_exact::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::delete_untracked_exact_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_delete_untracked_exact(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::delete_generated_prefix::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::delete_generated_prefix_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_delete_generated_prefix(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::git_remote_list::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_remote_list_definition,
        handler: |repository, _surface, _arguments| {
            crate::tools::git::handlers::call_git_remote_list(repository.git_repository())
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::git_remote_check::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_remote_check_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_remote_check(
                repository.git_repository(),
                arguments,
            )
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::DirectRemote,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::git_branch_prepare::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_branch_prepare_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_branch_prepare(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::DirectRemote,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::git_merge_readiness::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_merge_readiness_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_merge_readiness(
                repository.git_repository(),
                arguments,
            )
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::DirectRemote,
        read_only: true,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::git_push_exact::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::git_push_exact_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::git::handlers::call_git_push_exact(repository.git_repository(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::GIT_DEADLINE),
        reach: RemoteReach::DirectRemote,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::read_range::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::read_range_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_read_range(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::artifact_delete_exact::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::artifact_delete_exact_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_artifact_delete_exact(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::read_write_receipts::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::read_write_receipts_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_read_write_receipts(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::diff_preview::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::diff_preview_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_diff_preview(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::replace_exact::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::replace_exact_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_replace_exact(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::status_guard::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::status_guard_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_status_guard(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::file_info::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::file_info_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_file_info(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::set_file_executable::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::set_file_executable_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_set_file_executable(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::list_directory::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::list_directory_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_list_directory(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::read_file_bytes::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::read_file_bytes_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_read_file_bytes(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::write_new_file::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::write_new_file_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_write_new_file(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::write_new_file_base64::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::write_new_file_base64_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_write_new_file_base64(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::write_existing_file_exact_hash::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::write_existing_file_exact_hash_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_write_existing_file_exact_hash(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::artifact_write_text::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::artifact_write_text_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_artifact_write_text(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::artifact_write_base64::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::artifact_write_base64_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_artifact_write_base64(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::bulk_replace_exact::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::bulk_replace_exact_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_bulk_replace_exact(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::bulk_write_new_files_base64::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::bulk_write_new_files_base64_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_bulk_write_new_files_base64(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::create_directory::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::create_directory_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::files::call_create_directory(repository.root(), arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::WRITE_DEADLINE),
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: true,
    },
    ToolDescriptor {
        name: crate::tools::run_guarded_command::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::run_guarded_command_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::process::call_run_guarded_command(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::read_command_log::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::read_command_log_definition,
        handler: |_repository, _surface, arguments| {
            crate::tools::process::call_read_command_log(arguments)
        },
        deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
        reach: RemoteReach::Local,
        read_only: true,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::artifact_python_run::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::artifact_python_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::process::call_artifact_python_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::task_image_python_run::NAME,
        starts_background_job: true,
        schema: crate::tools::schema::task_image_python_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::process::call_task_image_python_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::IsolatedExecution,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::artifact_build_check_run::NAME,
        starts_background_job: true,
        schema: crate::tools::schema::artifact_build_check_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::process::call_artifact_build_check_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::compose_stack_run::NAME,
        starts_background_job: true,
        schema: crate::tools::schema::compose_stack_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::process::call_compose_stack_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::harbor_run_start::NAME,
        starts_background_job: true,
        schema: crate::tools::schema::harbor_run_start_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::process::call_harbor_run_start(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::image_cleanliness_check_run::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::image_cleanliness_check_run_definition,
        handler: |_repository, _surface, arguments| {
            crate::tools::process::call_image_cleanliness_check_run(arguments)
        },
        deadline: None,
        reach: RemoteReach::IsolatedExecution,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::docker_image_inspect::NAME,
        starts_background_job: false,
        schema: crate::tools::schema::docker_image_inspect_definition,
        handler: |_repository, _surface, arguments| {
            crate::tools::process::call_docker_image_inspect(arguments)
        },
        deadline: None,
        reach: RemoteReach::Local,
        read_only: false,
        serializes_mutation: false,
    },
    ToolDescriptor {
        name: crate::tools::validation_profile_run::NAME,
        starts_background_job: true,
        schema: crate::tools::schema::validation_profile_run_definition,
        handler: |repository, _surface, arguments| {
            crate::tools::process::call_validation_profile_run(repository.root(), arguments)
        },
        deadline: None,
        reach: RemoteReach::InheritedByExecutedCode,
        read_only: false,
        serializes_mutation: false,
    },
];

pub(crate) fn descriptor(name: &str) -> Option<&'static ToolDescriptor> {
    REGISTRY.iter().find(|entry| entry.name == name)
}

pub(crate) fn descriptors() -> &'static [ToolDescriptor] {
    REGISTRY
}

/// Every tool whose reply carries a log id to poll, in table order.
///
/// The manifest advertises this so a client knows which calls to poll rather than await. It was an
/// array written by hand and it was wrong by two: `compose_stack_run` and `artifact_build_check_run`,
/// the two asynchronous tools added most recently. Both had already been missed once in
/// `typed_workflows` in the same file and fixed in `7de5b1d`, so the same omission survived in a
/// second list because the fix was applied to the instance rather than to the class.
pub(crate) fn background_job_tools() -> Vec<&'static str> {
    REGISTRY
        .iter()
        .filter(|entry| entry.starts_background_job)
        .map(|entry| entry.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A duplicate name would make dispatch depend on table order, and the first entry would silently
    /// win. Cheap to check, and impossible to see by reading a growing table.
    #[test]
    fn descriptor_names_are_unique() {
        let mut names: Vec<&str> = REGISTRY.iter().map(|entry| entry.name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            total,
            "registry contains a duplicate tool name"
        );
    }

    /// Record which registry names are proper prefixes of another, because one is.
    ///
    /// This documents an assumption rather than guarding anything. `dispatch::attribute` matches a
    /// name followed by the refusal marker, so it is correct whether or not prefix pairs exist, and
    /// this test must not be read as the thing that makes it safe: deleting the boundary because
    /// this passes would reintroduce the misattribution the boundary exists to prevent.
    ///
    /// What it does is keep the assumption measured instead of remembered. The set is asserted whole
    /// rather than counted, so a new pair fails here and is looked at deliberately, and a pair that
    /// disappears fails too rather than leaving a stale claim behind. The scope is registry names
    /// only: `project_execute` resolves before the repository is determined and never reaches
    /// `call_tool`, so it cannot be misattributed and asserting over it would claim something wider
    /// than the property being documented.
    #[test]
    fn the_registry_names_that_are_prefixes_of_another_are_the_known_ones() {
        const KNOWN_PREFIX_PAIRS: &[(&str, &str)] = &[("write_new_file", "write_new_file_base64")];

        let names: Vec<&str> = REGISTRY.iter().map(|entry| entry.name).collect();
        let mut observed: Vec<(&str, &str)> = names
            .iter()
            .flat_map(|shorter| {
                names
                    .iter()
                    .filter(move |longer| *longer != shorter && longer.starts_with(*shorter))
                    .map(move |longer| (*shorter, *longer))
            })
            .collect();
        observed.sort_unstable();

        let mut known = KNOWN_PREFIX_PAIRS.to_vec();
        known.sort_unstable();

        assert_eq!(
            observed, known,
            "the set of registry names that are proper prefixes of another has changed; confirm \
             that dispatch::attribute still matches on the refusal marker rather than the bare name, \
             then record the new set here"
        );
    }

    /// Two fields could disagree, so pin the one direction that would be a lie.
    ///
    /// A tool that returns a log id cannot also promise a bounded reply, because there is no result
    /// to return within it. The converse is deliberately not asserted: `None` with a result is the
    /// legitimate majority case, since those tools hold their own operation-specific timeout, and
    /// that is the distinction this field exists to make rather than one to forbid.
    #[test]
    fn a_background_job_never_also_advertises_a_reply_deadline() {
        for entry in REGISTRY {
            assert!(
                !(entry.starts_background_job && entry.deadline.is_some()),
                "{} starts a background job and also advertises a reply deadline; one of the two \
                 is wrong, because a pollable reply has no result to deliver within a deadline",
                entry.name
            );
        }
    }

    /// The derived list must be right, not merely self-consistent.
    ///
    /// Deriving `background_jobs.tools` from the table removes the chance of forgetting a new
    /// asynchronous tool, but it does not check that the flags themselves are set correctly: every
    /// flag could be false and the manifest would advertise an empty list, consistently and wrongly.
    /// The consumers cannot catch that either, since the instructions test iterates this same list and
    /// an empty one passes it trivially. So the set is asserted whole here, which is an assertion
    /// rather than a second source: changing which tools are asynchronous is meant to fail here and be
    /// re-recorded deliberately.
    #[test]
    fn the_tools_that_start_background_jobs_are_the_known_ones() {
        const KNOWN_BACKGROUND_JOB_TOOLS: &[&str] = &[
            "task_image_python_run",
            "artifact_build_check_run",
            "compose_stack_run",
            "harbor_run_start",
            "validation_profile_run",
        ];

        let mut observed = background_job_tools();
        observed.sort_unstable();
        let mut known = KNOWN_BACKGROUND_JOB_TOOLS.to_vec();
        known.sort_unstable();

        assert_eq!(
            observed, known,
            "the set of tools that return a pollable log id has changed; the manifest and the client \
             instructions both derive from it, so record the new set here deliberately"
        );
    }

    /// A descriptor whose schema advertises a different name than the descriptor claims would make
    /// the registry disagree with the surface it generates.
    #[test]
    fn each_descriptor_schema_advertises_its_own_name() {
        for entry in REGISTRY {
            let schema = (entry.schema)();
            assert_eq!(
                schema.get("name").and_then(Value::as_str),
                Some(entry.name),
                "descriptor {} generates a schema for a different tool",
                entry.name
            );
        }
    }

    /// Each descriptor must invoke the handler named for its own tool.
    ///
    /// This is the sixth per-tool fact, and the only one neither snapshot covers. `tool-matrix.tsv`
    /// pins the deadline, lock, reach, and read-only axes; `tools-surface.json` pins the name and
    /// schema. Handler identity is pinned by neither, so pointing `git_push_exact` at
    /// `git_remote_check`'s handler leaves both fixtures byte-identical.
    ///
    /// Behavioural tests do not close it either, which was measured rather than assumed: invoked
    /// with empty arguments, only 6 of 54 tools name themselves in the reply and only 13 of 54
    /// replies are distinct at all, because most refuse with the same generic missing-argument text.
    /// Two tools taking the same argument name are mutually indistinguishable that way.
    ///
    /// Comparing function pointers would not work either: every descriptor holds its own closure, so
    /// 54 closures are 54 distinct addresses no matter which function each one calls.
    ///
    /// What does discriminate is the naming convention, which every handler follows without
    /// exception. This enforces it, and in doing so pins the wiring: a swapped or duplicated handler
    /// names the wrong tool and fails here.
    #[test]
    fn each_descriptor_calls_the_handler_named_for_its_tool() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tools/registry.rs"),
        )
        .expect("the registry module must be readable");

        let table = &source[source
            .find("static REGISTRY")
            .expect("the registry table must exist")..];
        let table = &table[..table.find("\n];").expect("the table must terminate")];

        let mut offenders = Vec::new();
        let mut seen = 0;
        for block in table.split("\n    ToolDescriptor {").skip(1) {
            seen += 1;
            let name = block
                .split("name: crate::tools::")
                .nth(1)
                .and_then(|rest| rest.split("::NAME").next())
                .expect("every descriptor names a tool");
            let handler = block
                .split("handler:")
                .nth(1)
                .and_then(|rest| rest.split("\n        deadline:").next())
                .expect("every descriptor has a handler");
            if !handler.contains(&format!("call_{name}(")) {
                offenders.push(name.to_string());
            }
        }

        // Fail closed. This parses its own source with fixed indentation, so a reformatted descriptor
        // would drop out of the loop silently and the check would still pass on whatever remained.
        // Comparing against the table length makes the formatting coupling harmless instead of
        // load-bearing: if the parse stops matching, this fails rather than shrinking.
        assert_eq!(
            seen,
            REGISTRY.len(),
            "the source parse found {seen} descriptors but the table holds {}; the parse has \
             drifted from the source layout and is no longer checking every tool",
            REGISTRY.len()
        );
        assert!(
            offenders.is_empty(),
            "these descriptors invoke a handler named for a different tool, which no snapshot can \
             detect: {offenders:?}"
        );
    }

    /// Every migrated tool must be reachable by name, or dispatch would fall through to a path that
    /// no longer has an arm for it.
    #[test]
    fn every_descriptor_resolves_by_name() {
        for entry in REGISTRY {
            assert!(
                descriptor(entry.name).is_some(),
                "{} is in the table but does not resolve",
                entry.name
            );
        }
    }
}
