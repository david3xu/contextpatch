use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::*;
use crate::tools::git::support::*;

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
    if paths.len() > 100 {
        return Err("git_restore_exact refused: at most 100 paths may be restored".to_string());
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_restore_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = repo_root.canonicalize().map_err(|error| {
        format!("git_restore_exact refused: failed to resolve repo root: {error}")
    })?;
    let normalized_paths = normalize_git_paths(tools::git_restore_exact::NAME, &root, &paths)?;
    let restore_paths: BTreeSet<String> = normalized_paths.iter().cloned().collect();
    if restore_paths.len() != normalized_paths.len() {
        return Err("git_restore_exact refused: duplicate paths are not allowed".to_string());
    }

    let dirty_paths = git_status_paths_for_tool(tools::git_restore_exact::NAME, &root)?;
    let missing_dirty = restore_paths
        .difference(&dirty_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing_dirty.is_empty() {
        return Err(format!(
            "git_restore_exact refused: every requested path must currently be dirty\nnot_dirty_paths:\n{}",
            format_set(&missing_dirty)
        ));
    }
    ensure_restore_paths_are_tracked(&root, &normalized_paths)?;

    if dry_run {
        let remaining_dirty_paths = dirty_paths
            .difference(&restore_paths)
            .cloned()
            .collect::<BTreeSet<_>>();
        return serde_json::to_string_pretty(&json!({
            "tool": tools::git_restore_exact::NAME,
            "dry_run": true,
            "would_restore_paths": normalized_paths,
            "remaining_dirty_paths_after_restore": remaining_dirty_paths,
            "required_confirm_for_restore": CONFIRMATION
        }))
        .map_err(|error| format!("git_restore_exact refused: {error}"));
    }

    git_success_for_tool(
        tools::git_restore_exact::NAME,
        &root,
        git_restore_args(&normalized_paths),
    )?;
    let dirty_after = git_status_paths_for_tool(tools::git_restore_exact::NAME, &root)?;
    let still_dirty = restore_paths
        .intersection(&dirty_after)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !still_dirty.is_empty() {
        return Err(format!(
            "git_restore_exact refused after restore: requested paths are still dirty\n{}",
            format_set(&still_dirty)
        ));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_restore_exact::NAME,
        "dry_run": false,
        "restored": true,
        "restored_paths": normalized_paths,
        "remaining_dirty_paths": dirty_after
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
    if paths.len() > 100 {
        return Err("delete_untracked_exact refused: at most 100 paths may be deleted".to_string());
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "delete_untracked_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = canonical_repo_root(repo_root, tools::delete_untracked_exact::NAME)?;
    let normalized_paths = normalize_git_paths(tools::delete_untracked_exact::NAME, &root, &paths)?;
    let requested_paths: BTreeSet<String> = normalized_paths.iter().cloned().collect();
    if requested_paths.len() != normalized_paths.len() {
        return Err("delete_untracked_exact refused: duplicate paths are not allowed".to_string());
    }

    let untracked_paths = git_untracked_paths_for_tool(tools::delete_untracked_exact::NAME, &root)?;
    let not_untracked = requested_paths
        .difference(&untracked_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !not_untracked.is_empty() {
        return Err(format!(
            "delete_untracked_exact refused: every requested path must be an untracked file\nnot_untracked_paths:\n{}",
            format_set(&not_untracked)
        ));
    }
    for path in &normalized_paths {
        let target = root.join(path);
        if !target.is_file() {
            return Err(format!(
                "delete_untracked_exact refused: `{path}` is not a regular file"
            ));
        }
    }

    if dry_run {
        let remaining_untracked_paths = untracked_paths
            .difference(&requested_paths)
            .cloned()
            .collect::<BTreeSet<_>>();
        return serde_json::to_string_pretty(&json!({
            "tool": tools::delete_untracked_exact::NAME,
            "dry_run": true,
            "would_delete_paths": normalized_paths,
            "remaining_untracked_paths_after_delete": remaining_untracked_paths,
            "required_confirm_for_delete": CONFIRMATION
        }))
        .map_err(|error| format!("delete_untracked_exact refused: {error}"));
    }

    for path in &normalized_paths {
        fs::remove_file(root.join(path)).map_err(|error| {
            format!("delete_untracked_exact refused: failed to delete `{path}`: {error}")
        })?;
    }
    let untracked_after = git_untracked_paths_for_tool(tools::delete_untracked_exact::NAME, &root)?;
    let still_untracked = requested_paths
        .intersection(&untracked_after)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !still_untracked.is_empty() {
        return Err(format!(
            "delete_untracked_exact refused after delete: requested paths are still untracked\n{}",
            format_set(&still_untracked)
        ));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::delete_untracked_exact::NAME,
        "dry_run": false,
        "deleted": true,
        "deleted_paths": normalized_paths,
        "remaining_untracked_paths": untracked_after
    }))
    .map_err(|error| format!("delete_untracked_exact refused: {error}"))
}
