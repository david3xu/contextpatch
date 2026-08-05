use std::path::PathBuf;

use contextpatch_core::git::delete_guarded::{delete_guarded, CONFIRMATION as DELETE_CONFIRMATION};
use contextpatch_core::git::tracked_move::{move_tracked, CONFIRMATION as MOVE_CONFIRMATION};
use contextpatch_core::git::RepositoryRoot;
use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::{optional_bool, optional_string, required_string};
use crate::tools::git::support::{repository_under_policy, WorktreeRootPolicy};

pub(crate) fn call_move_tracked<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let from = required_string(arguments, "from")?;
    let to = required_string(arguments, "to")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    // The worktree-root requirement is applied here so an anchored selection is not re-resolved, and the
    // same authority then carries both the Git move and the filesystem checks around it.
    let authority = repository_root.into();
    let policy = repository_under_policy(
        authority.git(),
        tools::move_tracked::NAME,
        WorktreeRootPolicy::ExactWorktreeRoot,
    )?;
    let confined = policy.bind_root(authority);

    let result = move_tracked(
        confined,
        &PathBuf::from(from),
        &PathBuf::from(to),
        dry_run,
        confirm,
    )
    .map_err(|error| format!("move_tracked refused: {error}"))?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::move_tracked::NAME,
        "dry_run": result.dry_run,
        "moved": result.moved,
        "from": result.from,
        "to": result.to,
        "sha256": result.sha256,
        "required_confirm_for_move": MOVE_CONFIRMATION
    }))
    .map_err(|error| format!("move_tracked refused: {error}"))
}

pub(crate) fn call_delete_guarded<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let expected_sha256 = required_string(arguments, "expected_sha256")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    let authority = repository_root.into();
    let policy = repository_under_policy(
        authority.git(),
        tools::delete_guarded::NAME,
        WorktreeRootPolicy::ExactWorktreeRoot,
    )?;
    let confined = policy.bind_root(authority);

    let result = delete_guarded(
        confined,
        &PathBuf::from(path),
        expected_sha256,
        dry_run,
        confirm,
    )
    .map_err(|error| format!("delete_guarded refused: {error}"))?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::delete_guarded::NAME,
        "dry_run": result.dry_run,
        "deleted": result.deleted,
        "path": result.path,
        "sha256": result.sha256,
        "required_confirm_for_delete": DELETE_CONFIRMATION
    }))
    .map_err(|error| format!("delete_guarded refused: {error}"))
}
