use std::path::{Path, PathBuf};

use contextpatch_core::git::delete_guarded::{delete_guarded, CONFIRMATION as DELETE_CONFIRMATION};
use contextpatch_core::git::tracked_move::{move_tracked, CONFIRMATION as MOVE_CONFIRMATION};
use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::{optional_bool, optional_string, required_string};

pub(crate) fn call_move_tracked(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let from = required_string(arguments, "from")?;
    let to = required_string(arguments, "to")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    let result = move_tracked(
        repo_root,
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

pub(crate) fn call_delete_guarded(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let expected_sha256 = required_string(arguments, "expected_sha256")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    let result = delete_guarded(
        repo_root,
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
