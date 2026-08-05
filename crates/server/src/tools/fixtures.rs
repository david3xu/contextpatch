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
use std::path::Path;

use contextpatch_core::error::ContextPatchError;
use contextpatch_core::fs::guarded_file::open_regular_file_in_root;
use contextpatch_core::fs::rooted::{self, RootedEntryKind};
use contextpatch_core::fs::write_new_file::write_new_file_bytes_in_root;
use contextpatch_core::git::RepositoryRoot;
use contextpatch_core::process::guarded_command::run_guarded_command;
use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::*;
use crate::tools::git::support::{
    format_set, git_status_paths_for_tool, normalize_git_path, normalize_git_paths,
};

/// The one base-image check script this surface is allowed to run.
///
/// Named once so the presence check, the argument vector, and the refusal text cannot drift apart.
const BASE_IMAGE_CHECK_SCRIPT: &str = "references/check-base-image.sh";

pub(crate) fn call_fixture_generator_run<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
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
    // Confinement, the script check, Git status, and the child's working directory all derive from one
    // authority, so a generator cannot be validated against one repository and run in another.
    let root = repository_root.into();
    let script = normalize_git_path(tools::fixture_generator_run::NAME, root, script_path)?;
    if !rooted::is_regular_file(root, &script)
        .map_err(|error| format!("fixture_generator_run refused: {error}"))?
    {
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
            root,
            &required_string_array(arguments, "allowed_existing_dirty_paths")?,
        )?
    } else {
        vec![script.clone()]
    }
    .into_iter()
    .collect::<BTreeSet<_>>();

    let before = git_status_paths_for_tool(tools::fixture_generator_run::NAME, root.git())?;
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
        root,
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

    // Outputs are rechecked through the same authority that validated them before the run.
    let after = git_status_paths_for_tool(tools::fixture_generator_run::NAME, root.git())?;
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

pub(crate) fn call_base_image_check_run<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "run base image check";

    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(120);
    let project_path = optional_string(arguments, "project_path")?;
    let command_args = base_image_check_args(project_path)?;
    // The presence check and the run share one authority, so the script that was found is the script the
    // child resolves inside its anchored working directory.
    let root = repository_root.into();
    if !rooted::is_regular_file(root, BASE_IMAGE_CHECK_SCRIPT)
        .map_err(|error| format!("base_image_check_run refused: {error}"))?
    {
        return Err(format!(
            "base_image_check_run refused: required script `{BASE_IMAGE_CHECK_SCRIPT}` is not present"
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

    let output = run_guarded_command(root, None, "bash", &command_args, Some(timeout_secs))
        .map_err(|error| format!("base_image_check_run refused: {error}"))?;
    if !guarded_output_succeeded(&output) {
        return Err(format!(
            "base_image_check_run refused: check command failed\n{output}"
        ));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::base_image_check_run::NAME,
        "ran": true,
        "script": BASE_IMAGE_CHECK_SCRIPT,
        "args": command_args,
        "command_output": output
    }))
    .map_err(|error| format!("base_image_check_run refused: {error}"))
}

fn base_image_check_args(project_path: Option<&str>) -> Result<Vec<String>, String> {
    match project_path {
        Some("task") => Ok(vec![
            BASE_IMAGE_CHECK_SCRIPT.to_string(),
            "task".to_string(),
        ]),
        Some(other) => Err(format!(
            "base_image_check_run refused: project_path must be exactly `task` when provided, got `{other}`"
        )),
        None => Ok(vec![BASE_IMAGE_CHECK_SCRIPT.to_string()]),
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

/// Collect fixture files and their digests through the repository's own authority.
///
/// Every existence check, directory walk, and content read goes through the root, so a manifest is verified
/// against the repository that was selected. Symlinks are refused rather than followed at any component,
/// which the previous path-based walk only checked at the entry it happened to be looking at.
fn collect_fixture_files_from_args(
    tool_name: &str,
    root: RepositoryRoot<'_>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<String, String>, String> {
    let fixture_paths = optional_string_array(arguments, "fixture_paths")?;
    let fixture_prefixes = optional_string_array(arguments, "fixture_prefixes")?;
    if fixture_paths.is_empty() && fixture_prefixes.is_empty() {
        return Err(format!(
            "{tool_name} refused: at least one fixture_paths or fixture_prefixes entry is required"
        ));
    }

    let refused = |error: ContextPatchError| format!("{tool_name} refused: {error}");

    let mut paths = BTreeSet::new();
    for path in normalize_repo_relative_paths(tool_name, &fixture_paths)? {
        if !rooted::is_regular_file(root, &path).map_err(refused)? {
            return Err(format!(
                "{tool_name} refused: fixture path `{path}` is not a regular file"
            ));
        }
        paths.insert(path);
    }
    for prefix in normalize_repo_relative_paths(tool_name, &fixture_prefixes)? {
        match rooted::entry_kind(root, &prefix).map_err(refused)? {
            Some(RootedEntryKind::RegularFile) => {
                paths.insert(prefix);
            }
            Some(RootedEntryKind::Directory) => {
                collect_regular_files(tool_name, root, &prefix, &mut paths)?;
            }
            _ => {
                return Err(format!(
                    "{tool_name} refused: fixture prefix `{prefix}` is not a file or directory"
                ));
            }
        }
    }

    let mut result = BTreeMap::new();
    for path in paths {
        let digest = rooted::sha256(root, &path)
            .map_err(|error| format!("{tool_name} refused: failed to read `{path}`: {error}"))?;
        result.insert(path, digest);
    }
    Ok(result)
}

fn collect_regular_files(
    tool_name: &str,
    root: RepositoryRoot<'_>,
    directory: &str,
    paths: &mut BTreeSet<String>,
) -> Result<(), String> {
    // Entries arrive sorted and named relative to the repository root, so the manifest order is
    // deterministic without a second sort and without reconstructing relative paths from absolute ones.
    let entries = rooted::read_dir(root, directory)
        .map_err(|error| format!("{tool_name} refused: failed to read directory: {error}"))?;
    for entry in entries {
        match entry.kind {
            RootedEntryKind::Symlink => {
                return Err(format!(
                    "{tool_name} refused: fixture path `{}` is a symlink",
                    entry.relative
                ));
            }
            RootedEntryKind::Directory => {
                collect_regular_files(tool_name, root, &entry.relative, paths)?;
            }
            RootedEntryKind::RegularFile => {
                paths.insert(entry.relative);
            }
            RootedEntryKind::Other => {}
        }
    }
    Ok(())
}

pub(crate) fn call_fixture_manifest_verify<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let manifest_path = required_string(arguments, "manifest_path")?;
    // The manifest and the fixtures it describes are reached through one authority, so a manifest cannot be
    // read from one repository and then compared against another repository's files.
    let root = repository_root.into();
    let manifest_normalized =
        normalize_repo_relative_path(tools::fixture_manifest_verify::NAME, manifest_path)?;
    if !rooted::is_regular_file(root, &manifest_normalized)
        .map_err(|error| format!("fixture_manifest_verify refused: {error}"))?
    {
        return Err(format!(
            "fixture_manifest_verify refused: manifest `{manifest_normalized}` is not an existing file"
        ));
    }
    let manifest_bytes = open_regular_file_in_root(root, Path::new(&manifest_normalized))
        .and_then(|manifest| manifest.read_all())
        .map_err(|error| {
            format!(
                "fixture_manifest_verify refused: failed to read `{manifest_normalized}`: {error}"
            )
        })?;
    let manifest_value: Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        format!("fixture_manifest_verify refused: manifest JSON is invalid: {error}")
    })?;
    let expected = parse_fixture_manifest(tools::fixture_manifest_verify::NAME, &manifest_value)?;
    let actual =
        collect_fixture_files_from_args(tools::fixture_manifest_verify::NAME, root, arguments)?;
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

pub(crate) fn call_fixture_manifest_refresh<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
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
    let root = repository_root.into();
    let manifest_normalized =
        normalize_repo_relative_path(tools::fixture_manifest_refresh::NAME, manifest_path)?;
    // Opened once and held, so the digest that gates the overwrite and the write that follows refer to the
    // same file rather than to the same name twice. A symlink is refused here rather than followed, which is
    // the uniform rule the rooted primitives apply everywhere else.
    let existing = match rooted::entry_kind(root, &manifest_normalized)
        .map_err(|error| format!("fixture_manifest_refresh refused: {error}"))?
    {
        None => None,
        Some(RootedEntryKind::RegularFile) => Some(
            open_regular_file_in_root(root, Path::new(&manifest_normalized)).map_err(|error| {
                format!(
                    "fixture_manifest_refresh refused: failed to read `{manifest_normalized}`: {error}"
                )
            })?,
        ),
        Some(_) => {
            return Err(format!(
                "fixture_manifest_refresh refused: manifest path `{manifest_normalized}` is not a regular file"
            ))
        }
    };
    let current_manifest = match &existing {
        Some(manifest) => {
            let bytes = manifest.read_all().map_err(|error| {
                format!(
                    "fixture_manifest_refresh refused: failed to read `{manifest_normalized}`: {error}"
                )
            })?;
            Some(sha256_hex(&bytes))
        }
        None => None,
    };
    if !dry_run {
        if let Some(current_sha) = &current_manifest {
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
        collect_fixture_files_from_args(tools::fixture_manifest_refresh::NAME, root, arguments)?;
    let files = actual
        .iter()
        .map(|(path, sha256)| {
            // Reported through the same authority that produced the digest. A size that cannot be read is
            // still reported as zero rather than failing the refresh, which is the behaviour this response
            // has always had.
            let bytes = open_regular_file_in_root(root, Path::new(path))
                .and_then(|fixture| fixture.size_bytes())
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
            "current_manifest_sha256": current_manifest.as_ref(),
            "new_manifest_sha256": new_sha256,
            "confirm_required": CONFIRMATION
        }))
        .map_err(|error| format!("fixture_manifest_refresh refused: {error}"));
    }

    if let Some(parent) = manifest_parent(&manifest_normalized) {
        if !rooted::is_directory(root, parent)
            .map_err(|error| format!("fixture_manifest_refresh refused: {error}"))?
        {
            return Err(format!(
                "fixture_manifest_refresh refused: parent directory for `{manifest_normalized}` does not exist"
            ));
        }
    }
    // An existing manifest is replaced through the handle that was already opened and hashed, so the
    // temporary file, its permission bits, and the rename are the guarded layer's concern rather than this
    // handler's. A manifest that is not there yet is created, which keeps the write create-only.
    match existing {
        Some(manifest) => manifest
            .replace_atomic(manifest_text.as_bytes())
            .map_err(|error| {
                format!(
                    "fixture_manifest_refresh refused: failed to replace `{manifest_normalized}`: {error}"
                )
            })?,
        None => {
            write_new_file_bytes_in_root(
                root,
                Path::new(&manifest_normalized),
                manifest_text.as_bytes(),
            )
            .map_err(|error| {
                format!(
                    "fixture_manifest_refresh refused: failed to write `{manifest_normalized}`: {error}"
                )
            })?;
        }
    }

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

/// The manifest's parent directory, named relative to the repository root.
///
/// `None` when the manifest sits directly under the root, which needs no check: a root that any authority
/// exists for is already a directory.
fn manifest_parent(manifest_normalized: &str) -> Option<&str> {
    manifest_normalized
        .rsplit_once('/')
        .map(|(parent, _leaf)| parent)
}
