use std::path::Path;

use serde_json::{json, Value};

use contextpatch_core::git::restore as core_restore;

use crate::tools;
use crate::tools::common::*;
use crate::tools::git::support::*;

/// Address a moved-policy refusal to the calling tool.
///
/// `core::git::restore` returns the detail only, so every public refusal string is assembled here.
fn refused(tool_name: &str, error: contextpatch_core::error::ContextPatchError) -> String {
    format!("{tool_name} refused: {error}")
}

pub(crate) fn call_git_restore_exact(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "restore exact paths";
    let paths = required_string_array(arguments, "paths")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("git_restore_exact refused: paths must not be empty".to_string());
    }
    if paths.len() > core_restore::MAX_RESTORE_PATHS {
        return Err(format!(
            "git_restore_exact refused: at most {} paths may be restored",
            core_restore::MAX_RESTORE_PATHS
        ));
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_restore_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = resolved_repo_root(repo_root, tools::git_restore_exact::NAME)?;
    let normalized_paths = normalize_git_paths(tools::git_restore_exact::NAME, &root, &paths)?;
    let plan = core_restore::plan_restore_exact(&root, &normalized_paths)
        .map_err(|error| refused(tools::git_restore_exact::NAME, error))?;

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::git_restore_exact::NAME,
            "dry_run": true,
            "would_restore_paths": plan.paths,
            "remaining_dirty_paths_after_restore": plan.remaining_dirty_after,
            "required_confirm_for_restore": CONFIRMATION
        }))
        .map_err(|error| format!("git_restore_exact refused: {error}"));
    }

    let outcome = core_restore::apply_restore_exact(&root, &plan)
        .map_err(|error| refused(tools::git_restore_exact::NAME, error))?;
    // A failed postcondition is addressed differently from a refusal that happened before anything
    // changed, so the offending set comes back as data and is worded here.
    if !outcome.still_dirty.is_empty() {
        return Err(format!(
            "git_restore_exact refused after restore: requested paths are still dirty\n{}",
            format_set(&outcome.still_dirty)
        ));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_restore_exact::NAME,
        "dry_run": false,
        "restored": true,
        "restored_paths": plan.paths,
        "remaining_dirty_paths": outcome.dirty_after
    }))
    .map_err(|error| format!("git_restore_exact refused: {error}"))
}

pub(crate) fn call_delete_untracked_exact(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "delete untracked files";
    let paths = required_string_array(arguments, "paths")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("delete_untracked_exact refused: paths must not be empty".to_string());
    }
    if paths.len() > core_restore::MAX_UNTRACKED_DELETE_PATHS {
        return Err(format!(
            "delete_untracked_exact refused: at most {} paths may be deleted",
            core_restore::MAX_UNTRACKED_DELETE_PATHS
        ));
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "delete_untracked_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = repo_root_for_policy(
        repo_root,
        tools::delete_untracked_exact::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
    let normalized_paths = normalize_git_paths(tools::delete_untracked_exact::NAME, &root, &paths)?;
    let plan = core_restore::plan_delete_untracked_exact(&root, &normalized_paths)
        .map_err(|error| refused(tools::delete_untracked_exact::NAME, error))?;

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::delete_untracked_exact::NAME,
            "dry_run": true,
            "would_delete_paths": plan.paths,
            "remaining_untracked_paths_after_delete": plan.remaining_untracked_after,
            "required_confirm_for_delete": CONFIRMATION
        }))
        .map_err(|error| format!("delete_untracked_exact refused: {error}"));
    }

    // The receipt wraps the removal alone. Planning already happened and verification happens after, so
    // the journalled window is exactly the mutation.
    crate::tools::journal::recorded_deletions(
        repo_root,
        tools::delete_untracked_exact::NAME,
        &plan.paths,
        || {
            core_restore::delete_untracked_files(&root, &plan)
                .map_err(|error| refused(tools::delete_untracked_exact::NAME, error))
        },
    )?;

    let outcome = core_restore::verify_untracked_deleted(&root, &plan)
        .map_err(|error| refused(tools::delete_untracked_exact::NAME, error))?;
    if !outcome.still_untracked.is_empty() {
        return Err(format!(
            "delete_untracked_exact refused after delete: requested paths are still untracked\n{}",
            format_set(&outcome.still_untracked)
        ));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::delete_untracked_exact::NAME,
        "dry_run": false,
        "deleted": true,
        "deleted_paths": plan.paths,
        "remaining_untracked_paths": outcome.untracked_after
    }))
    .map_err(|error| format!("delete_untracked_exact refused: {error}"))
}

pub(crate) fn call_delete_generated_prefix(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "delete generated paths";

    let prefixes = required_string_array(arguments, "prefixes")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if prefixes.is_empty() {
        return Err("delete_generated_prefix refused: prefixes must not be empty".to_string());
    }
    if prefixes.len() > core_restore::MAX_GENERATED_PREFIXES {
        return Err(format!(
            "delete_generated_prefix refused: at most {} prefixes may be expanded",
            core_restore::MAX_GENERATED_PREFIXES
        ));
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "delete_generated_prefix refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = repo_root_for_policy(
        repo_root,
        tools::delete_generated_prefix::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
    let normalized_prefixes =
        normalize_git_prefixes(tools::delete_generated_prefix::NAME, &root, &prefixes)?;
    let plan = core_restore::plan_delete_generated_prefix(&root, &normalized_prefixes)
        .map_err(|error| refused(tools::delete_generated_prefix::NAME, error))?;

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::delete_generated_prefix::NAME,
            "dry_run": true,
            "prefixes": plan.prefixes,
            "would_delete_files": plan.files,
            "would_delete_empty_or_ignored_dirs": plan.directories,
            "required_confirm_for_delete": CONFIRMATION
        }))
        .map_err(|error| format!("delete_generated_prefix refused: {error}"));
    }

    core_restore::apply_delete_generated_prefix(&root, &plan)
        .map_err(|error| refused(tools::delete_generated_prefix::NAME, error))?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::delete_generated_prefix::NAME,
        "dry_run": false,
        "deleted": true,
        "deleted_files": plan.files,
        "deleted_empty_or_ignored_dirs": plan.directories
    }))
    .map_err(|error| format!("delete_generated_prefix refused: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn moved_restore_policy_keeps_its_refusal_prefixes() {
        // Planning now lives in core, which returns the detail only. A duplicate entry is the cheapest
        // core-originated refusal to reach for all three actions, so it pins the seam without needing a
        // repository. If a detail is reworded in core, these fail rather than the public text changing.
        let root = std::env::temp_dir();

        assert_eq!(
            call_git_restore_exact(&root, &arguments(json!({"paths": ["a.txt", "a.txt"]})))
                .unwrap_err(),
            "git_restore_exact refused: duplicate paths are not allowed"
        );
        assert_eq!(
            call_delete_untracked_exact(&root, &arguments(json!({"paths": ["a.txt", "a.txt"]})))
                .unwrap_err(),
            "delete_untracked_exact refused: duplicate paths are not allowed"
        );
        assert_eq!(
            call_delete_generated_prefix(
                &root,
                &arguments(json!({"prefixes": ["build", "build"]}))
            )
            .unwrap_err(),
            "delete_generated_prefix refused: duplicate prefixes are not allowed"
        );
    }

    #[test]
    fn limits_and_confirmations_are_still_checked_before_any_repository_work() {
        // These stay in the adapter, ahead of normalization and planning, so the order in which a caller
        // learns about a malformed request is unchanged.
        let root = std::env::temp_dir();
        let too_many: Vec<String> = (0..=core_restore::MAX_RESTORE_PATHS)
            .map(|index| format!("file{index}.txt"))
            .collect();

        assert_eq!(
            call_git_restore_exact(&root, &arguments(json!({"paths": too_many}))).unwrap_err(),
            "git_restore_exact refused: at most 100 paths may be restored"
        );
        assert_eq!(
            call_git_restore_exact(&root, &arguments(json!({"paths": []}))).unwrap_err(),
            "git_restore_exact refused: paths must not be empty"
        );
        assert_eq!(
            call_git_restore_exact(
                &root,
                &arguments(json!({"paths": ["a.txt"], "dry_run": false}))
            )
            .unwrap_err(),
            "git_restore_exact refused: dry_run=false requires confirm: \"restore exact paths\""
        );
        assert_eq!(
            call_delete_generated_prefix(
                &root,
                &arguments(json!({"prefixes": ["build"], "dry_run": false}))
            )
            .unwrap_err(),
            "delete_generated_prefix refused: dry_run=false requires confirm: \"delete generated paths\""
        );
    }
}
