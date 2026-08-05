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

    let root = repo_root_for_policy(
        repo_root,
        tools::delete_untracked_exact::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
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

    crate::tools::journal::recorded_deletions(
        repo_root,
        tools::delete_untracked_exact::NAME,
        &normalized_paths,
        || {
            for path in &normalized_paths {
                fs::remove_file(root.join(path)).map_err(|error| {
                    format!("delete_untracked_exact refused: failed to delete `{path}`: {error}")
                })?;
            }
            Ok(())
        },
    )?;
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

pub(crate) fn call_delete_generated_prefix(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "delete generated paths";
    const MAX_DELETE_PATHS: usize = 5000;

    let prefixes = required_string_array(arguments, "prefixes")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if prefixes.is_empty() {
        return Err("delete_generated_prefix refused: prefixes must not be empty".to_string());
    }
    if prefixes.len() > 20 {
        return Err(
            "delete_generated_prefix refused: at most 20 prefixes may be expanded".to_string(),
        );
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
    let prefix_set: BTreeSet<String> = normalized_prefixes.iter().cloned().collect();
    if prefix_set.len() != normalized_prefixes.len() {
        return Err(
            "delete_generated_prefix refused: duplicate prefixes are not allowed".to_string(),
        );
    }

    let tracked_paths = git_ls_files(&root)?;
    let candidates =
        git_untracked_and_ignored_paths_for_tool(tools::delete_generated_prefix::NAME, &root)?;
    let matched = paths_under_prefixes(&candidates, &normalized_prefixes);
    let mut files_to_delete = BTreeSet::new();
    let mut dirs_to_delete = BTreeSet::new();

    for path in &matched {
        if tracked_paths.contains(path) {
            return Err(format!(
                "delete_generated_prefix refused: matched tracked path `{path}`"
            ));
        }
        let target = root.join(path);
        if target.is_file() {
            files_to_delete.insert(path.clone());
        } else if target.is_dir() {
            dirs_to_delete.insert(path.clone());
            collect_untracked_files_in_dir(&root, path, &tracked_paths, &mut files_to_delete)?;
        }
    }
    collect_empty_dirs_under_prefixes(&root, &normalized_prefixes, &mut dirs_to_delete)?;

    if files_to_delete.len() + dirs_to_delete.len() > MAX_DELETE_PATHS {
        return Err(format!(
            "delete_generated_prefix refused: matched {} paths; maximum is {MAX_DELETE_PATHS}",
            files_to_delete.len() + dirs_to_delete.len()
        ));
    }
    if files_to_delete.is_empty() && dirs_to_delete.is_empty() {
        return Err(format!(
            "delete_generated_prefix refused: no generated files or empty directories matched prefixes\nprefixes:\n{}",
            format_set(&prefix_set)
        ));
    }

    let file_list = files_to_delete.iter().cloned().collect::<Vec<_>>();
    let dir_list = dirs_to_delete.iter().cloned().collect::<Vec<_>>();
    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::delete_generated_prefix::NAME,
            "dry_run": true,
            "prefixes": normalized_prefixes,
            "would_delete_files": file_list,
            "would_delete_empty_or_ignored_dirs": dir_list,
            "required_confirm_for_delete": CONFIRMATION
        }))
        .map_err(|error| format!("delete_generated_prefix refused: {error}"));
    }

    for path in &files_to_delete {
        let target = root.join(path);
        if target.exists() {
            fs::remove_file(&target).map_err(|error| {
                format!("delete_generated_prefix refused: failed to delete `{path}`: {error}")
            })?;
        }
    }
    for path in dirs_to_delete.iter().rev() {
        let target = root.join(path);
        if target.is_dir() && directory_is_empty(&target)? {
            fs::remove_dir(&target).map_err(|error| {
                format!(
                    "delete_generated_prefix refused: failed to delete directory `{path}`: {error}"
                )
            })?;
        } else if target.is_dir() && matched.contains(path) {
            fs::remove_dir_all(&target).map_err(|error| {
                format!("delete_generated_prefix refused: failed to delete ignored directory `{path}`: {error}")
            })?;
        }
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::delete_generated_prefix::NAME,
        "dry_run": false,
        "deleted": true,
        "deleted_files": file_list,
        "deleted_empty_or_ignored_dirs": dir_list
    }))
    .map_err(|error| format!("delete_generated_prefix refused: {error}"))
}

fn git_ls_files(root: &Path) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(
        tools::delete_generated_prefix::NAME,
        root,
        &["ls-files", "-z"],
    )?;
    parse_nul_paths_for_tool(
        tools::delete_generated_prefix::NAME,
        &output.stdout,
        "git ls-files",
    )
}

fn collect_untracked_files_in_dir(
    root: &Path,
    path: &str,
    tracked_paths: &BTreeSet<String>,
    files: &mut BTreeSet<String>,
) -> Result<(), String> {
    let dir = root.join(path);
    for entry in fs::read_dir(&dir).map_err(|error| {
        format!("delete_generated_prefix refused: failed to read `{path}`: {error}")
    })? {
        let entry = entry.map_err(|error| {
            format!("delete_generated_prefix refused: failed to read entry under `{path}`: {error}")
        })?;
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| format!("delete_generated_prefix refused: {error}"))?
            .to_string_lossy()
            .replace('\\', "/");
        if tracked_paths.contains(&relative) {
            return Err(format!(
                "delete_generated_prefix refused: ignored directory `{path}` contains tracked path `{relative}`"
            ));
        }
        let file_type = entry.file_type().map_err(|error| {
            format!("delete_generated_prefix refused: failed to inspect `{relative}`: {error}")
        })?;
        if file_type.is_file() {
            files.insert(relative);
        } else if file_type.is_dir() {
            collect_untracked_files_in_dir(root, &relative, tracked_paths, files)?;
        } else {
            return Err(format!(
                "delete_generated_prefix refused: `{relative}` is not a regular file or directory"
            ));
        }
    }
    Ok(())
}

fn collect_empty_dirs_under_prefixes(
    root: &Path,
    prefixes: &[String],
    dirs: &mut BTreeSet<String>,
) -> Result<(), String> {
    for prefix in prefixes {
        let target = root.join(prefix);
        if target.is_dir() {
            collect_empty_dirs(root, prefix, dirs)?;
        }
    }
    Ok(())
}

fn collect_empty_dirs(
    root: &Path,
    path: &str,
    dirs: &mut BTreeSet<String>,
) -> Result<bool, String> {
    let target = root.join(path);
    let mut empty = true;
    for entry in fs::read_dir(&target).map_err(|error| {
        format!("delete_generated_prefix refused: failed to read `{path}`: {error}")
    })? {
        let entry = entry.map_err(|error| {
            format!("delete_generated_prefix refused: failed to read entry under `{path}`: {error}")
        })?;
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| format!("delete_generated_prefix refused: {error}"))?
            .to_string_lossy()
            .replace('\\', "/");
        let file_type = entry.file_type().map_err(|error| {
            format!("delete_generated_prefix refused: failed to inspect `{relative}`: {error}")
        })?;
        if file_type.is_dir() {
            if !collect_empty_dirs(root, &relative, dirs)? {
                empty = false;
            }
        } else {
            empty = false;
        }
    }
    if empty {
        dirs.insert(path.to_string());
    }
    Ok(empty)
}

fn directory_is_empty(path: &Path) -> Result<bool, String> {
    Ok(fs::read_dir(path)
        .map_err(|error| {
            format!(
                "delete_generated_prefix refused: failed to inspect directory {}: {error}",
                path.display()
            )
        })?
        .next()
        .is_none())
}
