pub mod base_image_check_run {
    pub const NAME: &str = "base_image_check_run";
}

pub mod fixture_generator_run {
    pub const NAME: &str = "fixture_generator_run";
}

pub mod fixture_manifest_refresh {
    pub const NAME: &str = "fixture_manifest_refresh";
}

pub mod fixture_manifest_verify {
    pub const NAME: &str = "fixture_manifest_verify";
}

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use contextpatch_core::process::guarded_command::run_guarded_command;
use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::*;
use crate::tools::git::support::{
    format_set, git_status_paths_for_tool, normalize_git_path, resolved_repo_root,
    normalize_git_paths,
};

pub(crate) fn call_fixture_generator_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "run fixture generator";

    let script_path = required_string(arguments, "script_path")?;
    if !script_path.ends_with(".py") {
        return Err("fixture_generator_run refused: script_path must end with .py".to_string());
    }
    let args = optional_string_array(arguments, "args")?;
    let exact_outputs = optional_string_array(arguments, "expected_output_paths")?;
    let output_prefixes = optional_string_array(arguments, "expected_output_prefixes")?;
    if exact_outputs.is_empty() && output_prefixes.is_empty() {
        return Err("fixture_generator_run refused: at least one expected output path or prefix is required".to_string());
    }
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(120);
    let cwd = optional_string(arguments, "cwd")?;
    let root = resolved_repo_root(repo_root, tools::fixture_generator_run::NAME)?;
    let script = normalize_git_path(tools::fixture_generator_run::NAME, &root, script_path)?;
    if !root.join(&script).is_file() {
        return Err(format!(
            "fixture_generator_run refused: script_path `{script}` is not a file"
        ));
    }
    let exact_outputs =
        normalize_repo_relative_paths(tools::fixture_generator_run::NAME, &exact_outputs)?
            .into_iter()
            .collect::<BTreeSet<_>>();
    let output_prefixes =
        normalize_repo_relative_paths(tools::fixture_generator_run::NAME, &output_prefixes)?;
    let allowed_existing_dirty = if arguments.contains_key("allowed_existing_dirty_paths") {
        normalize_git_paths(
            tools::fixture_generator_run::NAME,
            &root,
            &required_string_array(arguments, "allowed_existing_dirty_paths")?,
        )?
    } else {
        vec![script.clone()]
    }
    .into_iter()
    .collect::<BTreeSet<_>>();

    let before = git_status_paths_for_tool(tools::fixture_generator_run::NAME, &root)?;
    let unexpected_before = before
        .difference(&allowed_existing_dirty)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !unexpected_before.is_empty() {
        return Err(format!(
            "fixture_generator_run refused: worktree has dirty paths outside allowed_existing_dirty_paths before generator run\n{}",
            format_set(&unexpected_before)
        ));
    }

    let mut command_args = Vec::with_capacity(args.len() + 1);
    command_args.push(script.clone());
    command_args.extend(args);

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::fixture_generator_run::NAME,
            "dry_run": true,
            "would_run": {
                "program": "python3",
                "args": command_args,
                "cwd": cwd.unwrap_or("."),
                "timeout_secs": timeout_secs
            },
            "allowed_existing_dirty_paths": allowed_existing_dirty,
            "expected_output_paths": exact_outputs,
            "expected_output_prefixes": output_prefixes,
            "confirm_required": CONFIRMATION
        }))
        .map_err(|error| format!("fixture_generator_run refused: {error}"));
    }
    let confirm = optional_string(arguments, "confirm")?;
    let Some(confirm) = confirm else {
        return Err(format!(
            "fixture_generator_run refused: confirm must be {CONFIRMATION:?}"
        ));
    };
    if confirm != CONFIRMATION {
        return Err(format!(
            "fixture_generator_run refused: confirm must be {CONFIRMATION:?}"
        ));
    }

    let output = run_guarded_command(
        &root,
        cwd.map(Path::new),
        "python3",
        &command_args,
        Some(timeout_secs),
    )
    .map_err(|error| format!("fixture_generator_run refused: {error}"))?;
    if !guarded_output_succeeded(&output) {
        return Err(format!(
            "fixture_generator_run refused: generator command failed\n{output}"
        ));
    }

    let after = git_status_paths_for_tool(tools::fixture_generator_run::NAME, &root)?;
    let unauthorized = after
        .iter()
        .filter(|path| {
            !allowed_existing_dirty.contains(*path)
                && !exact_outputs.contains(*path)
                && !matches_any_prefix(path, &output_prefixes)
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    if !unauthorized.is_empty() {
        return Err(format!(
            "fixture_generator_run refused: generator changed paths outside declared outputs\n{}",
            format_set(&unauthorized)
        ));
    }

    let output_changes = after
        .iter()
        .filter(|path| exact_outputs.contains(*path) || matches_any_prefix(path, &output_prefixes))
        .cloned()
        .collect::<BTreeSet<_>>();

    serde_json::to_string_pretty(&json!({
        "tool": tools::fixture_generator_run::NAME,
        "ran": true,
        "program": "python3",
        "script_path": script,
        "changed_output_paths": output_changes,
        "status_paths_after": after,
        "command_output": output
    }))
    .map_err(|error| format!("fixture_generator_run refused: {error}"))
}

pub(crate) fn call_base_image_check_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "run base image check";
    const SCRIPT: &str = "references/check-base-image.sh";

    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(120);
    let project_path = optional_string(arguments, "project_path")?;
    let command_args = base_image_check_args(project_path)?;
    let root = resolved_repo_root(repo_root, tools::base_image_check_run::NAME)?;
    if !root.join(SCRIPT).is_file() {
        return Err(format!(
            "base_image_check_run refused: required script `{SCRIPT}` is not present"
        ));
    }
    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::base_image_check_run::NAME,
            "dry_run": true,
            "would_run": {
                "program": "bash",
                "args": command_args,
                "cwd": ".",
                "timeout_secs": timeout_secs
            },
            "scope": "exact base-image check script only, optionally with the exact task project argument; arbitrary shell remains unsupported",
            "confirm_required": CONFIRMATION
        }))
        .map_err(|error| format!("base_image_check_run refused: {error}"));
    }
    let confirm = optional_string(arguments, "confirm")?;
    let Some(confirm) = confirm else {
        return Err(format!(
            "base_image_check_run refused: confirm must be {CONFIRMATION:?}"
        ));
    };
    if confirm != CONFIRMATION {
        return Err(format!(
            "base_image_check_run refused: confirm must be {CONFIRMATION:?}"
        ));
    }

    let output = run_guarded_command(&root, None, "bash", &command_args, Some(timeout_secs))
        .map_err(|error| format!("base_image_check_run refused: {error}"))?;
    if !guarded_output_succeeded(&output) {
        return Err(format!(
            "base_image_check_run refused: check command failed\n{output}"
        ));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::base_image_check_run::NAME,
        "ran": true,
        "script": SCRIPT,
        "args": command_args,
        "command_output": output
    }))
    .map_err(|error| format!("base_image_check_run refused: {error}"))
}

fn base_image_check_args(project_path: Option<&str>) -> Result<Vec<String>, String> {
    const SCRIPT: &str = "references/check-base-image.sh";
    match project_path {
        Some("task") => Ok(vec![SCRIPT.to_string(), "task".to_string()]),
        Some(other) => Err(format!(
            "base_image_check_run refused: project_path must be exactly `task` when provided, got `{other}`"
        )),
        None => Ok(vec![SCRIPT.to_string()]),
    }
}

fn parse_fixture_manifest(
    tool_name: &str,
    value: &Value,
) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    match value {
        Value::Object(object) => {
            if let Some(files) = object.get("files") {
                let files = files.as_array().ok_or_else(|| {
                    format!("{tool_name} refused: manifest files must be an array")
                })?;
                for (index, entry) in files.iter().enumerate() {
                    let entry = entry.as_object().ok_or_else(|| {
                        format!("{tool_name} refused: manifest files[{index}] must be an object")
                    })?;
                    let path = entry.get("path").and_then(Value::as_str).ok_or_else(|| {
                        format!("{tool_name} refused: manifest files[{index}].path is required")
                    })?;
                    let sha = entry
                        .get("sha256")
                        .or_else(|| entry.get("digest"))
                        .or_else(|| entry.get("sha256_digest"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            format!(
                                "{tool_name} refused: manifest files[{index}] needs sha256 or digest"
                            )
                        })?;
                    let normalized = normalize_repo_relative_path(tool_name, path)?;
                    let sha = validate_sha256_hex(tool_name, sha)?;
                    if entries.insert(normalized.clone(), sha).is_some() {
                        return Err(format!(
                            "{tool_name} refused: manifest has duplicate path `{normalized}`"
                        ));
                    }
                }
            } else {
                for (path, digest) in object {
                    let digest = digest.as_str().ok_or_else(|| {
                        format!(
                            "{tool_name} refused: manifest object values must be SHA-256 strings"
                        )
                    })?;
                    let normalized = normalize_repo_relative_path(tool_name, path)?;
                    let digest = validate_sha256_hex(tool_name, digest)?;
                    if entries.insert(normalized.clone(), digest).is_some() {
                        return Err(format!(
                            "{tool_name} refused: manifest has duplicate path `{normalized}`"
                        ));
                    }
                }
            }
        }
        _ => {
            return Err(format!(
                "{tool_name} refused: manifest root must be an object"
            ))
        }
    }
    if entries.is_empty() {
        return Err(format!(
            "{tool_name} refused: manifest must contain at least one file"
        ));
    }
    Ok(entries)
}

fn collect_fixture_files_from_args(
    tool_name: &str,
    root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<String, String>, String> {
    let fixture_paths = optional_string_array(arguments, "fixture_paths")?;
    let fixture_prefixes = optional_string_array(arguments, "fixture_prefixes")?;
    if fixture_paths.is_empty() && fixture_prefixes.is_empty() {
        return Err(format!(
            "{tool_name} refused: at least one fixture_paths or fixture_prefixes entry is required"
        ));
    }

    let mut paths = BTreeSet::new();
    for path in normalize_repo_relative_paths(tool_name, &fixture_paths)? {
        let target = root.join(&path);
        if !target.is_file() {
            return Err(format!(
                "{tool_name} refused: fixture path `{path}` is not a regular file"
            ));
        }
        paths.insert(path);
    }
    for prefix in normalize_repo_relative_paths(tool_name, &fixture_prefixes)? {
        let target = root.join(&prefix);
        if target.is_file() {
            paths.insert(prefix);
        } else if target.is_dir() {
            collect_regular_files(tool_name, root, &target, &mut paths)?;
        } else {
            return Err(format!(
                "{tool_name} refused: fixture prefix `{prefix}` is not a file or directory"
            ));
        }
    }

    let mut result = BTreeMap::new();
    for path in paths {
        let target = root.join(&path);
        let bytes = fs::read(&target)
            .map_err(|error| format!("{tool_name} refused: failed to read `{path}`: {error}"))?;
        result.insert(path, sha256_hex(&bytes));
    }
    Ok(result)
}

fn collect_regular_files(
    tool_name: &str,
    root: &Path,
    directory: &Path,
    paths: &mut BTreeSet<String>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("{tool_name} refused: failed to read directory: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("{tool_name} refused: failed to read directory entry: {error}"))?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "{tool_name} refused: failed to inspect `{}`: {error}",
                path.display()
            )
        })?;
        if file_type.is_symlink() {
            return Err(format!(
                "{tool_name} refused: fixture path `{}` is a symlink",
                path.display()
            ));
        }
        if file_type.is_dir() {
            collect_regular_files(tool_name, root, &path, paths)?;
        } else if file_type.is_file() {
            paths.insert(repo_relative_string(tool_name, root, &path)?);
        }
    }
    Ok(())
}

fn repo_relative_string(tool_name: &str, root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(root).map_err(|error| {
        format!(
            "{tool_name} refused: path `{}` is outside repository root: {error}",
            path.display()
        )
    })?;
    let parts = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(value) => Ok(value.to_string_lossy().into_owned()),
            _ => Err(format!(
                "{tool_name} refused: collected path must be normalized relative"
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

pub(crate) fn call_fixture_manifest_verify(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let manifest_path = required_string(arguments, "manifest_path")?;
    let root = resolved_repo_root(repo_root, tools::fixture_manifest_verify::NAME)?;
    let manifest_normalized =
        normalize_repo_relative_path(tools::fixture_manifest_verify::NAME, manifest_path)?;
    let manifest_target = root.join(&manifest_normalized);
    if !manifest_target.is_file() {
        return Err(format!(
            "fixture_manifest_verify refused: manifest `{manifest_normalized}` is not an existing file"
        ));
    }
    let manifest_bytes = fs::read(&manifest_target).map_err(|error| {
        format!("fixture_manifest_verify refused: failed to read `{manifest_normalized}`: {error}")
    })?;
    let manifest_value: Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        format!("fixture_manifest_verify refused: manifest JSON is invalid: {error}")
    })?;
    let expected = parse_fixture_manifest(tools::fixture_manifest_verify::NAME, &manifest_value)?;
    let actual =
        collect_fixture_files_from_args(tools::fixture_manifest_verify::NAME, &root, arguments)?;
    let expected_paths = expected.keys().cloned().collect::<BTreeSet<_>>();
    let actual_paths = actual.keys().cloned().collect::<BTreeSet<_>>();
    let missing_files = expected_paths
        .difference(&actual_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    let unlisted_files = actual_paths
        .difference(&expected_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut modified_files = Vec::new();
    for path in expected_paths.intersection(&actual_paths) {
        let expected_sha = expected.get(path).expect("path from expected set");
        let actual_sha = actual.get(path).expect("path from actual set");
        if expected_sha != actual_sha {
            modified_files.push(json!({
                "path": path,
                "expected_sha256": expected_sha,
                "actual_sha256": actual_sha
            }));
        }
    }
    let verified =
        missing_files.is_empty() && unlisted_files.is_empty() && modified_files.is_empty();

    let report = serde_json::to_string_pretty(&json!({
        "tool": tools::fixture_manifest_verify::NAME,
        "verified": verified,
        "manifest_path": manifest_normalized,
        "manifest_sha256": sha256_hex(&manifest_bytes),
        "file_count": expected.len(),
        "actual_file_count": actual.len(),
        "missing_files": missing_files,
        "unlisted_files": unlisted_files,
        "modified_files": modified_files
    }))
    .map_err(|error| format!("fixture_manifest_verify refused: {error}"))?;
    if !verified {
        return Err(format!(
            "fixture_manifest_verify refused: fixture file set or digest mismatch\n{report}"
        ));
    }
    Ok(report)
}

pub(crate) fn call_fixture_manifest_refresh(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "refresh fixture manifest";

    let manifest_path = required_string(arguments, "manifest_path")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "fixture_manifest_refresh refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }
    let root = resolved_repo_root(repo_root, tools::fixture_manifest_refresh::NAME)?;
    let manifest_normalized =
        normalize_repo_relative_path(tools::fixture_manifest_refresh::NAME, manifest_path)?;
    let manifest_target = root.join(&manifest_normalized);
    let current_manifest = if manifest_target.exists() {
        if !manifest_target.is_file() {
            return Err(format!(
                "fixture_manifest_refresh refused: manifest path `{manifest_normalized}` is not a regular file"
            ));
        }
        let bytes = fs::read(&manifest_target).map_err(|error| {
            format!(
                "fixture_manifest_refresh refused: failed to read `{manifest_normalized}`: {error}"
            )
        })?;
        Some((sha256_hex(&bytes), bytes))
    } else {
        None
    };
    if !dry_run {
        if let Some((current_sha, _)) = &current_manifest {
            let expected = optional_string(arguments, "expected_manifest_sha256")?.ok_or_else(|| {
                "fixture_manifest_refresh refused: expected_manifest_sha256 is required when overwriting an existing manifest".to_string()
            })?;
            let expected = validate_sha256_hex(tools::fixture_manifest_refresh::NAME, expected)?;
            if &expected != current_sha {
                return Err(format!(
                    "fixture_manifest_refresh refused: manifest hash mismatch; current_sha256={current_sha}, expected_manifest_sha256={expected}"
                ));
            }
        }
    }

    let actual =
        collect_fixture_files_from_args(tools::fixture_manifest_refresh::NAME, &root, arguments)?;
    let files = actual
        .iter()
        .map(|(path, sha256)| {
            let bytes = fs::metadata(root.join(path))
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            json!({
                "path": path,
                "sha256": sha256,
                "bytes": bytes
            })
        })
        .collect::<Vec<_>>();
    let manifest = json!({
        "version": 1,
        "algorithm": "sha256",
        "files": files
    });
    let manifest_text = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("fixture_manifest_refresh refused: {error}"))?
        + "\n";
    let new_sha256 = sha256_hex(manifest_text.as_bytes());

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::fixture_manifest_refresh::NAME,
            "dry_run": true,
            "would_write": true,
            "manifest_path": manifest_normalized,
            "file_count": actual.len(),
            "current_manifest_sha256": current_manifest.as_ref().map(|(sha, _)| sha),
            "new_manifest_sha256": new_sha256,
            "confirm_required": CONFIRMATION
        }))
        .map_err(|error| format!("fixture_manifest_refresh refused: {error}"));
    }

    if let Some(parent) = manifest_target.parent() {
        if !parent.is_dir() {
            return Err(format!(
                "fixture_manifest_refresh refused: parent directory for `{manifest_normalized}` does not exist"
            ));
        }
    }
    let temporary = manifest_target.with_extension(format!(
        "contextpatch-tmp-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("fixture_manifest_refresh refused: {error}"))?
            .as_nanos()
    ));
    fs::write(&temporary, manifest_text).map_err(|error| {
        format!(
            "fixture_manifest_refresh refused: failed to write temporary manifest for `{manifest_normalized}`: {error}"
        )
    })?;
    fs::rename(&temporary, &manifest_target).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!(
            "fixture_manifest_refresh refused: failed to replace `{manifest_normalized}`: {error}"
        )
    })?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::fixture_manifest_refresh::NAME,
        "dry_run": false,
        "refreshed": true,
        "manifest_path": manifest_normalized,
        "file_count": actual.len(),
        "sha256": new_sha256
    }))
    .map_err(|error| format!("fixture_manifest_refresh refused: {error}"))
}
