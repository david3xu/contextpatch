pub mod artifact_write_base64 {
    pub const NAME: &str = "artifact_write_base64";
}

pub mod artifact_write_text {
    pub const NAME: &str = "artifact_write_text";
}

pub mod bulk_write_new_files_base64 {
    pub const NAME: &str = "bulk_write_new_files_base64";
}

pub mod create_directory {
    pub const NAME: &str = "create_directory";
}

pub mod diff_preview {
    pub const NAME: &str = "diff_preview";
}

pub mod read_range {
    pub const NAME: &str = "read_range";
}

pub mod replace_exact {
    pub const NAME: &str = "replace_exact";
}

pub mod status_guard {
    pub const NAME: &str = "status_guard";
}

pub mod write_existing_file_exact_hash {
    pub const NAME: &str = "write_existing_file_exact_hash";
}

pub mod write_new_file {
    pub const NAME: &str = "write_new_file";
}

pub mod write_new_file_base64 {
    pub const NAME: &str = "write_new_file_base64";
}

use std::collections::{hash_map::DefaultHasher, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use contextpatch_core::fs::create_directory::create_directory_in_root;
use contextpatch_core::fs::read_range::read_range_in_root;
use contextpatch_core::fs::write_new_file::{write_new_file_bytes_in_root, write_new_file_in_root};
use contextpatch_core::git::status::{status_summary, status_summary_for_path};
use contextpatch_core::patch::diff::preview_exact_replacement_in_root;
use contextpatch_core::replace::exact::replace_exact_in_root;
use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::*;
use crate::tools::git::support::canonical_repo_root;

pub(crate) fn call_read_range(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let start_line = required_usize(arguments, "start_line")?;
    let end_line = required_usize(arguments, "end_line")?;

    read_range_in_root(repo_root, Path::new(path), start_line, end_line)
        .map_err(|error| format!("read_range refused: {error}"))
}

pub(crate) fn call_diff_preview(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let old = required_string(arguments, "old")?;
    let new = required_string(arguments, "new")?;

    preview_exact_replacement_in_root(repo_root, Path::new(path), old, new)
        .map_err(|error| format!("diff_preview refused: {error}"))
}

pub(crate) fn call_replace_exact(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let old = required_string(arguments, "old")?;
    let new = required_string(arguments, "new")?;

    let summary = replace_exact_in_root(repo_root, Path::new(path), old, new)
        .map_err(|error| format!("replace_exact refused: {error}"))?;

    Ok(format!(
        "replaced bytes {}..{} in {} ({} bytes written)",
        summary.start_byte,
        summary.end_byte,
        summary.path.display(),
        summary.bytes_written
    ))
}

pub(crate) fn call_status_guard(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    match optional_string(arguments, "path")? {
        Some(path) => status_summary_for_path(repo_root, Some(Path::new(path))),
        None => status_summary(repo_root),
    }
    .map_err(|error| format!("status_guard refused: {error}"))
}

pub(crate) fn call_write_new_file(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let content = required_string(arguments, "content")?;

    let summary = write_new_file_in_root(repo_root, Path::new(path), content)
        .map_err(|error| format!("write_new_file refused: {error}"))?;

    Ok(format!(
        "created {} ({} bytes written)",
        summary.path.display(),
        summary.bytes_written
    ))
}

pub(crate) fn call_write_new_file_base64(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const MAX_DECODED_BYTES: usize = 20 * 1024 * 1024;

    let path = required_string(arguments, "path")?;
    let content_base64 = required_string(arguments, "content_base64")?;
    let expected_bytes = optional_u64(arguments, "expected_bytes")?;
    let bytes = decode_base64(content_base64)
        .map_err(|error| format!("write_new_file_base64 refused: {error}"))?;
    if bytes.len() > MAX_DECODED_BYTES {
        return Err(format!(
            "write_new_file_base64 refused: decoded content is {} bytes, maximum is {MAX_DECODED_BYTES}",
            bytes.len()
        ));
    }
    if let Some(expected_bytes) = expected_bytes {
        let expected_bytes = usize::try_from(expected_bytes).map_err(|_| {
            "write_new_file_base64 refused: expected_bytes is too large".to_string()
        })?;
        if bytes.len() != expected_bytes {
            return Err(format!(
                "write_new_file_base64 refused: decoded content is {} bytes, expected {expected_bytes}",
                bytes.len()
            ));
        }
    }

    let summary = write_new_file_bytes_in_root(repo_root, Path::new(path), &bytes)
        .map_err(|error| format!("write_new_file_base64 refused: {error}"))?;

    Ok(format!(
        "created {} ({} bytes written from base64)",
        summary.path.display(),
        summary.bytes_written
    ))
}

pub(crate) fn call_write_existing_file_exact_hash(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "write exact hash";

    let path = required_string(arguments, "path")?;
    let content = required_string(arguments, "content")?;
    let expected_sha256 = validate_sha256_hex(
        tools::write_existing_file_exact_hash::NAME,
        required_string(arguments, "expected_sha256")?,
    )?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "write_existing_file_exact_hash refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = canonical_repo_root(repo_root, tools::write_existing_file_exact_hash::NAME)?;
    let normalized =
        normalize_repo_relative_path(tools::write_existing_file_exact_hash::NAME, path)?;
    let target = root.join(&normalized);
    if !target.is_file() {
        return Err(format!(
            "write_existing_file_exact_hash refused: `{normalized}` is not an existing regular file"
        ));
    }
    let current = fs::read(&target).map_err(|error| {
        format!("write_existing_file_exact_hash refused: failed to read `{normalized}`: {error}")
    })?;
    let current_sha256 = sha256_hex(&current);
    if current_sha256 != expected_sha256 {
        return Err(format!(
            "write_existing_file_exact_hash refused: `{normalized}` hash mismatch; current_sha256={current_sha256}, expected_sha256={expected_sha256}"
        ));
    }
    let new_bytes = content.as_bytes();
    let new_sha256 = sha256_hex(new_bytes);
    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::write_existing_file_exact_hash::NAME,
            "dry_run": true,
            "would_write": true,
            "path": normalized,
            "current_sha256": current_sha256,
            "new_sha256": new_sha256,
            "bytes_written": new_bytes.len(),
            "confirm_required": CONFIRMATION
        }))
        .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"));
    }

    let temporary = target.with_extension(format!(
        "contextpatch-tmp-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"))?
            .as_nanos()
    ));
    fs::write(&temporary, new_bytes).map_err(|error| {
        format!("write_existing_file_exact_hash refused: failed to write temporary file: {error}")
    })?;
    fs::rename(&temporary, &target).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("write_existing_file_exact_hash refused: failed to replace `{normalized}`: {error}")
    })?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::write_existing_file_exact_hash::NAME,
        "dry_run": false,
        "wrote": true,
        "path": normalized,
        "previous_sha256": current_sha256,
        "sha256": new_sha256,
        "bytes_written": new_bytes.len()
    }))
    .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"))
}

pub(crate) fn call_artifact_write_text(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let content = required_string(arguments, "content")?;
    let parents = optional_bool(arguments, "parents")?.unwrap_or(false);
    write_artifact(
        repo_root,
        path,
        content.as_bytes(),
        parents,
        tools::artifact_write_text::NAME,
    )
}

pub(crate) fn call_artifact_write_base64(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const MAX_DECODED_BYTES: usize = 20 * 1024 * 1024;

    let path = required_string(arguments, "path")?;
    let content_base64 = required_string(arguments, "content_base64")?;
    let expected_bytes = optional_u64(arguments, "expected_bytes")?;
    let parents = optional_bool(arguments, "parents")?.unwrap_or(false);
    let bytes = decode_base64(content_base64)
        .map_err(|error| format!("artifact_write_base64 refused: {error}"))?;
    if bytes.len() > MAX_DECODED_BYTES {
        return Err(format!(
            "artifact_write_base64 refused: decoded content is {} bytes, maximum is {MAX_DECODED_BYTES}",
            bytes.len()
        ));
    }
    if let Some(expected_bytes) = expected_bytes {
        let expected_bytes = usize::try_from(expected_bytes).map_err(|_| {
            "artifact_write_base64 refused: expected_bytes is too large".to_string()
        })?;
        if bytes.len() != expected_bytes {
            return Err(format!(
                "artifact_write_base64 refused: decoded content is {} bytes, expected {expected_bytes}",
                bytes.len()
            ));
        }
    }

    write_artifact(
        repo_root,
        path,
        &bytes,
        parents,
        tools::artifact_write_base64::NAME,
    )
}

pub(crate) fn call_bulk_write_new_files_base64(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const MAX_FILES: usize = 500;
    const MAX_TOTAL_DECODED_BYTES: usize = 20 * 1024 * 1024;

    let entries = arguments
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "bulk_write_new_files_base64 refused: entries is required".to_string())?;
    if entries.is_empty() {
        return Err("bulk_write_new_files_base64 refused: entries must not be empty".to_string());
    }
    if entries.len() > MAX_FILES {
        return Err(format!(
            "bulk_write_new_files_base64 refused: {} entries exceeds maximum {MAX_FILES}",
            entries.len()
        ));
    }
    let parents = optional_bool(arguments, "parents")?.unwrap_or(false);
    let root = canonical_repo_root(repo_root, tools::bulk_write_new_files_base64::NAME)?;

    let mut decoded_entries = Vec::with_capacity(entries.len());
    let mut seen_paths = BTreeSet::new();
    let mut total_bytes = 0usize;

    for (index, entry) in entries.iter().enumerate() {
        let entry = entry.as_object().ok_or_else(|| {
            format!("bulk_write_new_files_base64 refused: entry {index} must be an object")
        })?;
        let path = required_string(entry, "path")?;
        let normalized =
            normalize_repo_relative_path(tools::bulk_write_new_files_base64::NAME, path)?;
        if !seen_paths.insert(normalized.clone()) {
            return Err(format!(
                "bulk_write_new_files_base64 refused: duplicate path `{normalized}`"
            ));
        }
        let content_base64 = required_string(entry, "content_base64")?;
        let bytes = decode_base64(content_base64)
            .map_err(|error| format!("bulk_write_new_files_base64 refused: {error}"))?;
        if let Some(expected_bytes) = optional_u64(entry, "expected_bytes")? {
            let expected_bytes = usize::try_from(expected_bytes).map_err(|_| {
                "bulk_write_new_files_base64 refused: expected_bytes is too large".to_string()
            })?;
            if bytes.len() != expected_bytes {
                return Err(format!(
                    "bulk_write_new_files_base64 refused: `{normalized}` decoded content is {} bytes, expected {expected_bytes}",
                    bytes.len()
                ));
            }
        }
        total_bytes = total_bytes.checked_add(bytes.len()).ok_or_else(|| {
            "bulk_write_new_files_base64 refused: decoded byte total overflow".to_string()
        })?;
        if total_bytes > MAX_TOTAL_DECODED_BYTES {
            return Err(format!(
                "bulk_write_new_files_base64 refused: decoded content is {total_bytes} bytes, maximum is {MAX_TOTAL_DECODED_BYTES}"
            ));
        }
        let target = root.join(&normalized);
        if target.exists() {
            return Err(format!(
                "bulk_write_new_files_base64 refused: target already exists: {normalized}"
            ));
        }
        if !parents {
            let parent = target.parent().ok_or_else(|| {
                format!("bulk_write_new_files_base64 refused: path `{normalized}` has no parent")
            })?;
            if !parent.is_dir() {
                return Err(format!(
                    "bulk_write_new_files_base64 refused: parent does not exist for `{normalized}`"
                ));
            }
        }
        decoded_entries.push((normalized, bytes));
    }

    for (path, _) in &decoded_entries {
        if parents {
            let target = root.join(path);
            let parent = target.parent().ok_or_else(|| {
                format!("bulk_write_new_files_base64 refused: path `{path}` has no parent")
            })?;
            fs::create_dir_all(parent).map_err(|error| {
                format!("bulk_write_new_files_base64 refused: failed to create parents: {error}")
            })?;
        }
    }

    let mut created = Vec::with_capacity(decoded_entries.len());
    for (path, bytes) in decoded_entries {
        let summary = write_new_file_bytes_in_root(&root, Path::new(&path), &bytes)
            .map_err(|error| format!("bulk_write_new_files_base64 refused: {error}"))?;
        created.push(json!({
            "path": summary.path.display().to_string(),
            "bytes_written": summary.bytes_written
        }));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::bulk_write_new_files_base64::NAME,
        "created": true,
        "file_count": created.len(),
        "total_bytes_written": total_bytes,
        "files": created
    }))
    .map_err(|error| format!("bulk_write_new_files_base64 refused: {error}"))
}

fn write_artifact(
    repo_root: &Path,
    path: &str,
    bytes: &[u8],
    parents: bool,
    tool_name: &str,
) -> Result<String, String> {
    let artifact_root = artifact_root(repo_root, tool_name)?;
    let relative = validate_relative_path(tool_name, path)?;
    let target = artifact_root.join(&relative);
    if parents {
        let parent = target
            .parent()
            .ok_or_else(|| format!("{tool_name} refused: artifact path has no parent"))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("{tool_name} refused: failed to create parents: {error}"))?;
    }
    let summary = write_new_file_bytes_in_root(&artifact_root, &relative, bytes)
        .map_err(|error| format!("{tool_name} refused: {error}"))?;

    serde_json::to_string_pretty(&json!({
        "tool": tool_name,
        "created": true,
        "artifact_root": artifact_root.display().to_string(),
        "path": summary.path.display().to_string(),
        "bytes_written": summary.bytes_written,
        "repo_mutation": false
    }))
    .map_err(|error| format!("{tool_name} refused: {error}"))
}

fn artifact_root(repo_root: &Path, tool_name: &str) -> Result<PathBuf, String> {
    let root = canonical_repo_root(repo_root, tool_name)?;
    let mut hasher = DefaultHasher::new();
    root.display().to_string().hash(&mut hasher);
    let repo_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let artifact_root = std::env::temp_dir()
        .join("contextpatch-artifacts")
        .join(format!("{repo_name}-{:016x}", hasher.finish()));
    fs::create_dir_all(&artifact_root)
        .map_err(|error| format!("{tool_name} refused: failed to create artifact root: {error}"))?;
    artifact_root
        .canonicalize()
        .map_err(|error| format!("{tool_name} refused: failed to resolve artifact root: {error}"))
}

pub(crate) fn call_create_directory(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let parents = optional_bool(arguments, "parents")?.unwrap_or(false);

    let summary = create_directory_in_root(repo_root, Path::new(path), parents)
        .map_err(|error| format!("create_directory refused: {error}"))?;

    Ok(format!(
        "created directory {} ({} directories created)",
        summary.path.display(),
        summary.directories_created.len()
    ))
}
