pub mod artifact_delete_exact {
    pub const NAME: &str = "artifact_delete_exact";
    pub const CONFIRMATION: &str = "delete artifact exact";
}

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

pub mod file_info {
    pub const NAME: &str = "file_info";
}

pub mod read_write_receipts {
    pub const NAME: &str = "read_write_receipts";
}

pub mod list_directory {
    pub const NAME: &str = "list_directory";
}

pub mod diff_preview {
    pub const NAME: &str = "diff_preview";
}

pub mod read_file_bytes {
    pub const NAME: &str = "read_file_bytes";
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
use std::fs::{self, FileType};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use contextpatch_core::fs::create_directory::create_directory_in_root;
use contextpatch_core::fs::read_range::read_range_in_root;
use contextpatch_core::fs::write_new_file::{write_new_file_bytes_in_root, write_new_file_in_root};
use contextpatch_core::git::status::{status_summary, status_summary_for_path};
use contextpatch_core::patch::diff::preview_exact_replacement_in_root;
use contextpatch_core::replace::exact::replace_exact_in_root_with_sha256;
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

pub(crate) fn call_read_write_receipts(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    use contextpatch_core::fs::receipt::{self, DEFAULT_RECENT_LIMIT};

    let interrupted_only = optional_bool(arguments, "interrupted_only")?.unwrap_or(false);
    let limit = optional_u64(arguments, "limit")?
        .map(|value| {
            usize::try_from(value)
                .map_err(|_| "read_write_receipts refused: limit is too large".to_string())
        })
        .transpose()?
        .unwrap_or(DEFAULT_RECENT_LIMIT);

    let receipts = if interrupted_only {
        receipt::unsettled(repo_root, limit)
    } else {
        receipt::recent(repo_root, limit)
    }
    .map_err(|error| format!("read_write_receipts refused: {error}"))?;

    let interrupted = receipts.iter().filter(|entry| entry.interrupted()).count();
    let entries: Vec<Value> = receipts.iter().map(|entry| entry.to_json()).collect();

    serde_json::to_string_pretty(&json!({
        "tool": tools::read_write_receipts::NAME,
        "journal": receipt::journal_path(repo_root)
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        "interrupted_only": interrupted_only,
        "returned": entries.len(),
        "interrupted": interrupted,
        // The recovery step is stated rather than implied, because the whole reason this tool exists
        // is that the caller has just lost an answer and needs to know what to do next.
        "recovery": "for an interrupted file entry, compare before_sha256 with the current digest from file_info; for an interrupted Git entry, compare before_git_head with the current HEAD from run_guarded_command git rev-parse HEAD",
        "receipts": entries
    }))
    .map_err(|error| format!("read_write_receipts refused: {error}"))
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
    let expected_sha256 = optional_string(arguments, "expected_sha256")?;

    let summary =
        crate::tools::journal::recorded(repo_root, tools::replace_exact::NAME, path, || {
            replace_exact_in_root_with_sha256(repo_root, Path::new(path), old, new, expected_sha256)
                .map_err(|error| format!("replace_exact refused: {error}"))
        })?;

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

pub(crate) fn call_file_info(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let root = canonical_repo_root(repo_root, tools::file_info::NAME)?;
    let normalized = normalize_repo_relative_path(tools::file_info::NAME, path)?;
    let target = root.join(&normalized);
    let symlink_metadata = match fs::symlink_metadata(&target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return serde_json::to_string_pretty(&json!({
                "tool": tools::file_info::NAME,
                "path": normalized,
                "exists": false
            }))
            .map_err(|error| format!("file_info refused: {error}"));
        }
        Err(error) => {
            return Err(format!(
                "file_info refused: failed to inspect `{normalized}`: {error}"
            ));
        }
    };
    let file_type = symlink_metadata.file_type();
    let is_symlink = file_type.is_symlink();
    let resolved_inside_repo = if is_symlink {
        match target.canonicalize() {
            Ok(resolved) => resolved.starts_with(&root),
            Err(_) => false,
        }
    } else {
        true
    };
    let target_metadata = if is_symlink && resolved_inside_repo {
        fs::metadata(&target).ok()
    } else if is_symlink {
        None
    } else {
        Some(symlink_metadata)
    };
    let is_file = target_metadata
        .as_ref()
        .map(fs::Metadata::is_file)
        .unwrap_or(false);
    let is_dir = target_metadata
        .as_ref()
        .map(fs::Metadata::is_dir)
        .unwrap_or(false);
    let bytes = if is_file {
        Some(fs::read(&target).map_err(|error| {
            format!("file_info refused: failed to read `{normalized}` for digest: {error}")
        })?)
    } else {
        None
    };
    let line_count = bytes.as_ref().and_then(|bytes| {
        std::str::from_utf8(bytes)
            .ok()
            .map(|text| text.lines().count())
    });
    let sha256 = bytes.as_ref().map(|bytes| sha256_hex(bytes));

    serde_json::to_string_pretty(&json!({
        "tool": tools::file_info::NAME,
        "path": normalized,
        "exists": true,
        "kind": file_kind(file_type, is_file, is_dir),
        "is_file": is_file,
        "is_directory": is_dir,
        "is_symlink": is_symlink,
        "symlink_target": if is_symlink { fs::read_link(&target).ok().map(|path| path.display().to_string()) } else { None },
        "symlink_resolves_inside_repo": resolved_inside_repo,
        "size_bytes": target_metadata.as_ref().map(fs::Metadata::len),
        "sha256": sha256,
        "line_count": line_count
    }))
    .map_err(|error| format!("file_info refused: {error}"))
}

pub(crate) fn call_list_directory(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const MAX_ENTRIES: usize = 2000;

    let path = optional_string(arguments, "path")?.unwrap_or(".");
    let include_hidden = optional_bool(arguments, "include_hidden")?.unwrap_or(false);
    let root = canonical_repo_root(repo_root, tools::list_directory::NAME)?;
    let normalized = normalize_repo_relative_path(tools::list_directory::NAME, path)?;
    let target = root.join(&normalized);
    if !target.is_dir() {
        return Err(format!(
            "list_directory refused: `{normalized}` is not an existing directory"
        ));
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(&target).map_err(|error| {
        format!("list_directory refused: failed to read `{normalized}`: {error}")
    })? {
        let entry = entry
            .map_err(|error| format!("list_directory refused: failed to read entry: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !include_hidden && name.starts_with('.') {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            return Err(format!(
                "list_directory refused: directory has more than {MAX_ENTRIES} entries; use a narrower path"
            ));
        }
        let relative = if normalized == "." {
            name.clone()
        } else {
            format!("{normalized}/{name}")
        };
        let file_type = entry.file_type().map_err(|error| {
            format!("list_directory refused: failed to inspect `{relative}`: {error}")
        })?;
        let metadata = if file_type.is_symlink() {
            None
        } else {
            entry.metadata().ok()
        };
        entries.push(json!({
            "name": name,
            "path": relative,
            "kind": file_kind(file_type, metadata.as_ref().map(fs::Metadata::is_file).unwrap_or(false), metadata.as_ref().map(fs::Metadata::is_dir).unwrap_or(false)),
            "is_symlink": file_type.is_symlink(),
            "size_bytes": metadata.as_ref().filter(|metadata| metadata.is_file()).map(fs::Metadata::len)
        }));
    }
    entries.sort_by(|left, right| {
        left["path"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["path"].as_str().unwrap_or_default())
    });

    serde_json::to_string_pretty(&json!({
        "tool": tools::list_directory::NAME,
        "path": normalized,
        "entry_count": entries.len(),
        "entries": entries
    }))
    .map_err(|error| format!("list_directory refused: {error}"))
}

pub(crate) fn call_read_file_bytes(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const MAX_BYTES: usize = 1_048_576;

    let path = required_string(arguments, "path")?;
    let offset = optional_u64(arguments, "offset")?.unwrap_or(0);
    let max_bytes = optional_u64(arguments, "max_bytes")?.unwrap_or(4096);
    let encoding = optional_string(arguments, "encoding")?.unwrap_or("hex");
    if max_bytes == 0 || max_bytes > MAX_BYTES as u64 {
        return Err(format!(
            "read_file_bytes refused: max_bytes must be between 1 and {MAX_BYTES}"
        ));
    }
    if encoding != "hex" && encoding != "base64" {
        return Err("read_file_bytes refused: encoding must be `hex` or `base64`".to_string());
    }
    let root = canonical_repo_root(repo_root, tools::read_file_bytes::NAME)?;
    let normalized = normalize_repo_relative_path(tools::read_file_bytes::NAME, path)?;
    let target = root.join(&normalized);
    let metadata = fs::symlink_metadata(&target).map_err(|error| {
        format!("read_file_bytes refused: failed to inspect `{normalized}`: {error}")
    })?;
    if metadata.file_type().is_symlink() {
        let resolved = target.canonicalize().map_err(|error| {
            format!("read_file_bytes refused: failed to resolve `{normalized}`: {error}")
        })?;
        if !resolved.starts_with(&root) {
            return Err(format!(
                "read_file_bytes refused: `{normalized}` is a symlink that resolves outside the repository"
            ));
        }
        let target_metadata = fs::metadata(&target).map_err(|error| {
            format!("read_file_bytes refused: failed to inspect `{normalized}` target: {error}")
        })?;
        if !target_metadata.is_file() {
            return Err(format!(
                "read_file_bytes refused: `{normalized}` is not an existing regular file"
            ));
        }
    } else if !metadata.is_file() {
        return Err(format!(
            "read_file_bytes refused: `{normalized}` is not an existing regular file"
        ));
    }
    let bytes = fs::read(&target).map_err(|error| {
        format!("read_file_bytes refused: failed to read `{normalized}`: {error}")
    })?;
    let start = usize::try_from(offset)
        .map_err(|_| "read_file_bytes refused: offset is too large".to_string())?;
    if start > bytes.len() {
        return Err(format!(
            "read_file_bytes refused: offset {offset} is past end of `{normalized}` ({} bytes)",
            bytes.len()
        ));
    }
    let end = start.saturating_add(max_bytes as usize).min(bytes.len());
    let slice = &bytes[start..end];
    let data = if encoding == "hex" {
        hex_encode(slice)
    } else {
        base64_encode(slice)
    };

    serde_json::to_string_pretty(&json!({
        "tool": tools::read_file_bytes::NAME,
        "path": normalized,
        "encoding": encoding,
        "offset": offset,
        "bytes_returned": slice.len(),
        "total_bytes": bytes.len(),
        "truncated": end < bytes.len(),
        "sha256": sha256_hex(&bytes),
        "data": data
    }))
    .map_err(|error| format!("read_file_bytes refused: {error}"))
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

    crate::tools::journal::recorded(
        repo_root,
        tools::write_existing_file_exact_hash::NAME,
        &normalized,
        || {
            let _mutation_lock =
                contextpatch_core::fs::mutation_lock::try_file_mutation_lock(&root, &target)
                    .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"))?;
            let current = fs::read(&target).map_err(|error| {
                format!(
                    "write_existing_file_exact_hash refused: failed to re-read `{normalized}` \
                     under the mutation lock: {error}"
                )
            })?;
            let current_sha256 = sha256_hex(&current);
            if current_sha256 != expected_sha256 {
                return Err(format!(
                    "write_existing_file_exact_hash refused: `{normalized}` changed before the \
                     mutation; current_sha256={current_sha256}, expected_sha256={expected_sha256}"
                ));
            }
            contextpatch_core::fs::atomic_write::write_atomic(&target, new_bytes).map_err(
                |error| {
                    format!(
                        "write_existing_file_exact_hash refused: failed to replace \
                         `{normalized}`: {error}"
                    )
                },
            )?;

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
        },
    )
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

pub(crate) fn call_artifact_delete_exact(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let tool_name = tools::artifact_delete_exact::NAME;
    let path = required_string(arguments, "path")?;
    let expected_sha256 = optional_string(arguments, "expected_sha256")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    if let Some(expected_sha256) = expected_sha256 {
        contextpatch_core::fs::hash::validate_sha256(expected_sha256)
            .map_err(|error| format!("{tool_name} refused: {error}"))?;
    }
    if !dry_run && expected_sha256.is_none() {
        return Err(format!(
            "{tool_name} refused: dry_run=false requires expected_sha256 from a current dry run"
        ));
    }
    if !dry_run && confirm != Some(tools::artifact_delete_exact::CONFIRMATION) {
        return Err(format!(
            "{tool_name} refused: dry_run=false requires confirm: {:?}",
            tools::artifact_delete_exact::CONFIRMATION
        ));
    }

    let root = artifact_root(repo_root, tool_name)?;
    let shown = normalize_repo_relative_path(tool_name, path)?;
    if shown != path {
        return Err(format!(
            "{tool_name} refused: path must be a normalized relative path"
        ));
    }
    let target = root.join(&shown);
    let _target_lock =
        contextpatch_core::fs::mutation_lock::try_file_mutation_lock(repo_root, &target)
            .map_err(|error| format!("{tool_name} refused: {error}"))?;

    let metadata = inspect_exact_artifact_file(tool_name, &root, Path::new(&shown))?;
    let current_sha256 = contextpatch_core::fs::hash::sha256_file(&target)
        .map_err(|error| format!("{tool_name} refused: {error}"))?;
    if expected_sha256.is_some_and(|expected| expected != current_sha256) {
        return Err(format!(
            "{tool_name} refused: hash mismatch for `{shown}`; current_sha256={current_sha256}, expected_sha256={}",
            expected_sha256.unwrap_or_default()
        ));
    }

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tool_name,
            "dry_run": true,
            "deleted": false,
            "artifact_root": root.display().to_string(),
            "path": shown,
            "sha256": current_sha256,
            "bytes": metadata.len(),
            "required_confirm_for_delete": tools::artifact_delete_exact::CONFIRMATION,
            "repo_mutation": false
        }))
        .map_err(|error| format!("{tool_name} refused: {error}"));
    }

    let verified_metadata = inspect_exact_artifact_file(tool_name, &root, Path::new(&shown))?;
    let verified_sha256 = contextpatch_core::fs::hash::sha256_file(&target)
        .map_err(|error| format!("{tool_name} refused: {error}"))?;
    let expected_sha256 = expected_sha256.unwrap_or_default();
    if verified_sha256 != expected_sha256 {
        return Err(format!(
            "{tool_name} refused: artifact `{shown}` changed during validation; current_sha256={verified_sha256}, expected_sha256={expected_sha256}"
        ));
    }
    fs::remove_file(&target)
        .map_err(|error| format!("{tool_name} refused: failed to delete `{shown}`: {error}"))?;
    match fs::symlink_metadata(&target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(format!(
                "{tool_name} refused: delete verification failed; `{shown}` still exists"
            ));
        }
        Err(error) => {
            return Err(format!(
                "{tool_name} refused: delete verification failed for `{shown}`: {error}"
            ));
        }
    }

    serde_json::to_string_pretty(&json!({
        "tool": tool_name,
        "dry_run": false,
        "deleted": true,
        "artifact_root": root.display().to_string(),
        "path": shown,
        "sha256": verified_sha256,
        "bytes_freed": verified_metadata.len(),
        "repo_mutation": false
    }))
    .map_err(|error| format!("{tool_name} refused: {error}"))
}

fn inspect_exact_artifact_file(
    tool_name: &str,
    root: &Path,
    relative: &Path,
) -> Result<fs::Metadata, String> {
    let shown = relative.display();
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!("{tool_name} refused: artifact `{shown}` does not exist")
            } else {
                format!("{tool_name} refused: failed to inspect artifact `{shown}`: {error}")
            }
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "{tool_name} refused: artifact `{shown}` contains a symlink component"
            ));
        }
    }

    let target = root.join(relative);
    let metadata = fs::symlink_metadata(&target)
        .map_err(|error| format!("{tool_name} refused: failed to inspect `{shown}`: {error}"))?;
    if !metadata.is_file() {
        return Err(format!(
            "{tool_name} refused: artifact `{shown}` is not a regular file"
        ));
    }
    let resolved = target
        .canonicalize()
        .map_err(|error| format!("{tool_name} refused: failed to resolve `{shown}`: {error}"))?;
    if !resolved.starts_with(root) {
        return Err(format!(
            "{tool_name} refused: artifact `{shown}` resolves outside the artifact root"
        ));
    }
    Ok(metadata)
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

pub(crate) fn artifact_root(repo_root: &Path, tool_name: &str) -> Result<PathBuf, String> {
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

fn file_kind(file_type: FileType, is_file: bool, is_dir: bool) -> &'static str {
    if file_type.is_symlink() {
        "symlink"
    } else if is_file {
        "file"
    } else if is_dir {
        "directory"
    } else {
        "other"
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        output.push(TABLE[(b0 >> 2) as usize] as char);
        output.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
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
