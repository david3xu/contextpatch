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

pub mod bulk_replace_exact {
    pub const NAME: &str = "bulk_replace_exact";
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

pub mod set_file_executable {
    pub const NAME: &str = "set_file_executable";
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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use contextpatch_core::fs::create_directory::create_directory_in_root;
use contextpatch_core::fs::directory::list_directory_in_root;
use contextpatch_core::fs::guarded_file::{
    inspect_path_in_root, open_regular_file_in_root, validate_new_file_path_in_root,
    GuardedPathKind, GuardedRegularFile,
};
use contextpatch_core::fs::read_range::read_range_in_root;
use contextpatch_core::fs::write_new_file::{
    write_new_file_bytes_in_root, write_new_file_bytes_with_parents_in_root, write_new_file_in_root,
};
use contextpatch_core::git::status::{status_summary, status_summary_for_path};
use contextpatch_core::patch::diff::preview_exact_replacement_in_root;
use contextpatch_core::replace::exact::replace_exact_in_root_with_sha256;
use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::*;
use contextpatch_core::fs::rooted::canonical_label;
use contextpatch_core::git::RepositoryRoot;

/// Address a core refusal to the calling tool.
fn refused(tool_name: &str, error: contextpatch_core::error::ContextPatchError) -> String {
    format!("{tool_name} refused: {error}")
}

/// The canonical path label for the boundaries that are still keyed by name.
///
/// Locks, receipts, and scratch identity are derived from a path and remain so. This resolves a configured
/// root once and returns an anchored selection's logical path untouched, so a selection is never reopened.
fn label(root: RepositoryRoot<'_>, tool_name: &str) -> Result<PathBuf, String> {
    canonical_label(root).map_err(|error| refused(tool_name, error))
}

pub(crate) fn call_read_range<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let start_line = required_usize(arguments, "start_line")?;
    let end_line = required_usize(arguments, "end_line")?;

    read_range_in_root(
        repository_root.into(),
        Path::new(path),
        start_line,
        end_line,
    )
    .map_err(|error| format!("read_range refused: {error}"))
}

pub(crate) fn call_read_write_receipts<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    use contextpatch_core::fs::receipt::{self, DEFAULT_RECENT_LIMIT};

    // The receipt journal is a deferred boundary and stays keyed by path.
    let repo_root = label(
        repository_root.into(),
        tools::read_write_receipts::NAME,
    )?;
    let repo_root = repo_root.as_path();
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

pub(crate) fn call_diff_preview<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let old = required_string(arguments, "old")?;
    let new = required_string(arguments, "new")?;

    preview_exact_replacement_in_root(repository_root.into(), Path::new(path), old, new)
        .map_err(|error| format!("diff_preview refused: {error}"))
}

pub(crate) fn call_replace_exact<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let old = required_string(arguments, "old")?;
    let new = required_string(arguments, "new")?;
    let expected_sha256 = optional_string(arguments, "expected_sha256")?;

    let authority = repository_root.into();
    // The receipt journal is keyed by path; the replacement itself goes through the authority.
    let journal_root = label(authority, tools::replace_exact::NAME)?;
    let summary = crate::tools::journal::recorded(
        &journal_root,
        tools::replace_exact::NAME,
        path,
        || {
            replace_exact_in_root_with_sha256(
                authority,
                Path::new(path),
                old,
                new,
                expected_sha256,
            )
            .map_err(|error| format!("replace_exact refused: {error}"))
        },
    )?;

    Ok(format!(
        "replaced bytes {}..{} in {} ({} bytes written); sha256={}",
        summary.start_byte,
        summary.end_byte,
        summary.path.display(),
        summary.bytes_written,
        summary.sha256
    ))
}

pub(crate) fn call_status_guard<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    // Status confinement and Git execution both derive from the root, so the projection happens inside.
    let root = repository_root.into();
    match optional_string(arguments, "path")? {
        Some(path) => status_summary_for_path(root, Some(Path::new(path))),
        None => status_summary(root),
    }
    .map_err(|error| format!("status_guard refused: {error}"))
}

pub(crate) fn call_file_info<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let root = repository_root.into();
    let path = optional_string(arguments, "path")?;
    let has_paths = arguments.contains_key("paths");
    let paths = if has_paths {
        required_string_array(arguments, "paths")?
    } else {
        Vec::new()
    };
    match (path, has_paths) {
        (Some(_), true) => {
            return Err("file_info refused: provide either `path` or `paths`, not both".to_string())
        }
        (None, false) => {
            return Err(
                "file_info refused: provide `path` or a non-empty `paths` array".to_string(),
            )
        }
        _ => {}
    }

    if let Some(path) = path {
        let mut value = file_info_value(root, path)?;
        value
            .as_object_mut()
            .expect("file_info values are objects")
            .insert(
                "tool".to_string(),
                Value::String(tools::file_info::NAME.to_string()),
            );
        return serde_json::to_string_pretty(&value)
            .map_err(|error| format!("file_info refused: {error}"));
    }

    const MAX_PATHS: usize = 64;
    if paths.is_empty() {
        return Err("file_info refused: paths must not be empty".to_string());
    }
    if paths.len() > MAX_PATHS {
        return Err(format!(
            "file_info refused: paths may contain at most {MAX_PATHS} entries"
        ));
    }
    let mut seen = BTreeSet::new();
    let mut entries = Vec::with_capacity(paths.len());
    for path in &paths {
        if !seen.insert(path) {
            return Err(format!("file_info refused: duplicate path `{path}`"));
        }
        entries.push(file_info_value(root, path)?);
    }
    serde_json::to_string_pretty(&json!({
        "tool": tools::file_info::NAME,
        "path_count": entries.len(),
        "entries": entries
    }))
    .map_err(|error| format!("file_info refused: {error}"))
}

fn file_info_value(root: RepositoryRoot<'_>, path: &str) -> Result<Value, String> {
    let normalized = normalize_repo_relative_path(tools::file_info::NAME, path)?;
    let inspection = match inspect_path_in_root(root, Path::new(&normalized))
        .map_err(|error| format!("file_info refused: failed to inspect `{normalized}`: {error}"))?
    {
        Some(inspection) => inspection,
        None => {
            return Ok(json!({
                "path": normalized,
                "exists": false
            }));
        }
    };
    let is_file = inspection.kind == GuardedPathKind::RegularFile;
    let is_dir = inspection.kind == GuardedPathKind::Directory;
    let is_symlink = inspection.kind == GuardedPathKind::Symlink;
    let analysis = inspection
        .regular_file
        .as_ref()
        .map(|file| {
            file.read_range_with_digest(0, 0).map_err(|error| {
                format!("file_info refused: failed to inspect `{normalized}`: {error}")
            })
        })
        .transpose()?;
    let line_count = analysis.as_ref().and_then(|read| read.line_count);
    let sha256 = analysis.as_ref().map(|read| read.sha256.as_str());
    let size_bytes = analysis
        .as_ref()
        .map_or(inspection.size_bytes, |read| read.total_bytes);
    let kind = match inspection.kind {
        GuardedPathKind::RegularFile => "file",
        GuardedPathKind::Directory => "directory",
        GuardedPathKind::Symlink => "symlink",
        GuardedPathKind::Other => "other",
    };

    Ok(json!({
        "path": normalized,
        "exists": true,
        "kind": kind,
        "is_file": is_file,
        "is_directory": is_dir,
        "is_symlink": is_symlink,
        "symlink_target": inspection.symlink_target.map(|path| path.display().to_string()),
        "symlink_resolves_inside_repo": inspection.symlink_resolves_inside_root,
        "size_bytes": if is_symlink { None } else { Some(size_bytes) },
        "sha256": sha256,
        "line_count": line_count,
        "mode": inspection.mode.map(|mode| format!("{mode:04o}")),
        "executable": inspection.mode.map(|mode| mode & 0o111 != 0)
    }))
}

pub(crate) fn call_set_file_executable<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let executable = arguments
        .get("executable")
        .and_then(Value::as_bool)
        .ok_or_else(|| "missing or invalid boolean argument: executable".to_string())?;
    let expected_sha256 = optional_string(arguments, "expected_sha256")?;
    let expected_mode = optional_string(arguments, "expected_mode")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    let change = contextpatch_core::fs::executable::set_file_executable_in_root(
        repository_root.into(),
        Path::new(path),
        executable,
        expected_sha256,
        expected_mode,
        dry_run,
        confirm,
    )
    .map_err(|error| format!("set_file_executable refused: {error}"))?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::set_file_executable::NAME,
        "path": change.path,
        "sha256": change.sha256,
        "before_mode": change.before_mode,
        "after_mode": change.after_mode,
        "before_executable": change.before_executable,
        "after_executable": change.after_executable,
        "changed": change.changed,
        "dry_run": change.dry_run,
        "confirm_required": contextpatch_core::fs::executable::CONFIRMATION
    }))
    .map_err(|error| format!("set_file_executable refused: {error}"))
}

pub(crate) fn call_list_directory<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const MAX_ALLOWED_ENTRIES: usize = 2000;
    const MAX_ALLOWED_DEPTH: usize = 16;

    let path = optional_string(arguments, "path")?.unwrap_or(".");
    let include_hidden = optional_bool(arguments, "include_hidden")?.unwrap_or(false);
    let recursive = optional_bool(arguments, "recursive")?.unwrap_or(false);
    let max_depth = optional_u64(arguments, "max_depth")?.unwrap_or(4);
    let max_entries = optional_u64(arguments, "max_entries")?.unwrap_or(2000);
    let max_depth = usize::try_from(max_depth)
        .map_err(|_| "list_directory refused: max_depth is too large".to_string())?;
    let max_entries = usize::try_from(max_entries)
        .map_err(|_| "list_directory refused: max_entries is too large".to_string())?;
    if max_depth == 0 || max_depth > MAX_ALLOWED_DEPTH {
        return Err(format!(
            "list_directory refused: max_depth must be between 1 and {MAX_ALLOWED_DEPTH}"
        ));
    }
    if max_entries == 0 || max_entries > MAX_ALLOWED_ENTRIES {
        return Err(format!(
            "list_directory refused: max_entries must be between 1 and {MAX_ALLOWED_ENTRIES}"
        ));
    }
    let listing = list_directory_in_root(
        repository_root.into(),
        Path::new(path),
        include_hidden,
        recursive,
        max_depth,
        max_entries,
    )
    .map_err(|error| format!("list_directory refused: {error}"))?;
    let normalized = listing.path;
    let truncated = listing.truncated;
    let mut entries = listing
        .entries
        .into_iter()
        .map(|entry| {
            json!({
                "name": entry.name,
                "path": entry.path,
                "depth": entry.depth,
                "kind": entry.kind,
                "is_symlink": entry.kind == "symlink",
                "size_bytes": entry.size_bytes
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left["path"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["path"].as_str().unwrap_or_default())
    });

    serde_json::to_string_pretty(&json!({
        "tool": tools::list_directory::NAME,
        "path": normalized,
        "recursive": recursive,
        "max_depth": if recursive { max_depth } else { 1 },
        "max_entries": max_entries,
        "entry_count": entries.len(),
        "truncated": truncated,
        "entries": entries
    }))
    .map_err(|error| format!("list_directory refused: {error}"))
}

pub(crate) fn call_read_file_bytes<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
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
    let normalized = normalize_repo_relative_path(tools::read_file_bytes::NAME, path)?;
    let file = open_regular_file_in_root(repository_root.into(), Path::new(&normalized))
        .map_err(|error| format!("read_file_bytes refused: {error}"))?;
    let read = file
        .read_range_with_digest(offset, max_bytes)
        .map_err(|error| format!("read_file_bytes refused: {error}"))?;
    let data = if encoding == "hex" {
        hex_encode(&read.bytes)
    } else {
        base64_encode(&read.bytes)
    };
    let returned_end = offset.saturating_add(read.bytes.len() as u64);

    serde_json::to_string_pretty(&json!({
        "tool": tools::read_file_bytes::NAME,
        "path": normalized,
        "encoding": encoding,
        "offset": offset,
        "bytes_returned": read.bytes.len(),
        "total_bytes": read.total_bytes,
        "truncated": returned_end < read.total_bytes,
        "sha256": read.sha256,
        "data": data
    }))
    .map_err(|error| format!("read_file_bytes refused: {error}"))
}

pub(crate) fn call_write_new_file<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let content = required_string(arguments, "content")?;

    let summary = write_new_file_in_root(repository_root.into(), Path::new(path), content)
        .map_err(|error| format!("write_new_file refused: {error}"))?;

    Ok(format!(
        "created {} ({} bytes written); sha256={}",
        summary.path.display(),
        summary.bytes_written,
        sha256_hex(content.as_bytes())
    ))
}

pub(crate) fn call_write_new_file_base64<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
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

    let summary = write_new_file_bytes_in_root(repository_root.into(), Path::new(path), &bytes)
        .map_err(|error| format!("write_new_file_base64 refused: {error}"))?;

    Ok(format!(
        "created {} ({} bytes written from base64); sha256={}",
        summary.path.display(),
        summary.bytes_written,
        sha256_hex(&bytes)
    ))
}

pub(crate) fn call_write_existing_file_exact_hash<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
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

    let authority = repository_root.into();
    // Locks and receipts are keyed by path; access goes through the authority.
    let root = label(authority, tools::write_existing_file_exact_hash::NAME)?;
    let normalized =
        normalize_repo_relative_path(tools::write_existing_file_exact_hash::NAME, path)?;
    let file = open_regular_file_in_root(authority, Path::new(&normalized))
        .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"))?;
    let current = file
        .read_all()
        .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"))?;
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
        &root,
        tools::write_existing_file_exact_hash::NAME,
        &normalized,
        || {
            let file = open_regular_file_in_root(authority, Path::new(&normalized))
                .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"))?;
            let target = file.target_path();
            let _mutation_lock =
                contextpatch_core::fs::mutation_lock::try_file_mutation_lock_for_open_file(
                    &root,
                    &target,
                    file.file(),
                )
                .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"))?;
            let current = file
                .read_all()
                .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"))?;
            let current_sha256 = sha256_hex(&current);
            if current_sha256 != expected_sha256 {
                return Err(format!(
                    "write_existing_file_exact_hash refused: `{normalized}` changed before the \
                     mutation; current_sha256={current_sha256}, expected_sha256={expected_sha256}"
                ));
            }
            file.replace_atomic(new_bytes).map_err(|error| {
                format!(
                    "write_existing_file_exact_hash refused: failed to replace \
                     `{normalized}`: {error}"
                )
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
        },
    )
}

pub(crate) fn call_artifact_write_text<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let content = required_string(arguments, "content")?;
    let parents = optional_bool(arguments, "parents")?.unwrap_or(false);
    write_artifact(
        repository_root,
        path,
        content.as_bytes(),
        parents,
        tools::artifact_write_text::NAME,
    )
}

pub(crate) fn call_artifact_write_base64<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
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
        repository_root,
        path,
        &bytes,
        parents,
        tools::artifact_write_base64::NAME,
    )
}

pub(crate) fn call_bulk_write_new_files_base64<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
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
    let root = label(
        repository_root.into(),
        tools::bulk_write_new_files_base64::NAME,
    )?;

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
        validate_new_file_path_in_root(&root, Path::new(&normalized), parents)
            .map_err(|error| format!("bulk_write_new_files_base64 refused: {error}"))?;
        decoded_entries.push((normalized, bytes));
    }

    let mut created = Vec::with_capacity(decoded_entries.len());
    for (path, bytes) in decoded_entries {
        let summary =
            write_new_file_bytes_with_parents_in_root(&root, Path::new(&path), &bytes, parents)
                .map_err(|error| format!("bulk_write_new_files_base64 refused: {error}"))?;
        created.push(json!({
            "path": summary.path.display().to_string(),
            "bytes_written": summary.bytes_written,
            "sha256": sha256_hex(&bytes)
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

pub(crate) fn call_artifact_delete_exact<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
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

    let root = artifact_root(repository_root, tool_name)?;
    let shown = normalize_repo_relative_path(tool_name, path)?;
    if shown != path {
        return Err(format!(
            "{tool_name} refused: path must be a normalized relative path"
        ));
    }
    // Opened once through the artifact directory's own authority. The digest, the lock, and the deletion all
    // refer to that one file, so nothing can be swapped in between reading it and removing it.
    let authority = RepositoryRoot::from_path(&root);
    let target = open_exact_artifact_file(tool_name, authority, &shown)?;
    let target_path = root.join(&shown);
    let _target_lock =
        contextpatch_core::fs::mutation_lock::try_file_mutation_lock_for_open_file(
            &root,
            &target_path,
            target.file(),
        )
        .map_err(|error| format!("{tool_name} refused: {error}"))?;

    let bytes = target
        .size_bytes()
        .map_err(|error| format!("{tool_name} refused: {error}"))?;
    let current_sha256 = target
        .sha256()
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
            "bytes": bytes,
            "required_confirm_for_delete": tools::artifact_delete_exact::CONFIRMATION,
            "repo_mutation": false
        }))
        .map_err(|error| format!("{tool_name} refused: {error}"));
    }

    // Reproved immediately before the removal, against the same handle rather than against the name again.
    target
        .revalidate_current_path()
        .map_err(|error| format!("{tool_name} refused: {error}"))?;
    let verified_sha256 = target
        .sha256()
        .map_err(|error| format!("{tool_name} refused: {error}"))?;
    let verified_bytes = target
        .size_bytes()
        .map_err(|error| format!("{tool_name} refused: {error}"))?;
    let expected_sha256 = expected_sha256.unwrap_or_default();
    if verified_sha256 != expected_sha256 {
        return Err(format!(
            "{tool_name} refused: artifact `{shown}` changed during validation; current_sha256={verified_sha256}, expected_sha256={expected_sha256}"
        ));
    }
    contextpatch_core::fs::rooted::remove_file(authority, &shown)
        .map_err(|error| format!("{tool_name} refused: failed to delete `{shown}`: {error}"))?;
    match contextpatch_core::fs::rooted::entry_kind(authority, &shown) {
        Ok(None) => {}
        Ok(Some(_)) => {
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
        "bytes_freed": verified_bytes,
        "repo_mutation": false
    }))
    .map_err(|error| format!("{tool_name} refused: {error}"))
}

/// Open one artifact file through the artifact directory's authority, refusing anything that is not one.
///
/// The distinct refusals are kept because they tell a caller different things. What no longer happens is the
/// hand-rolled walk that used `symlink_metadata` on each joined component and then canonicalized the leaf to
/// check containment: the rooted primitives refuse a symlink at any component and cannot leave the directory
/// in the first place, so containment is structural rather than checked afterwards.
fn open_exact_artifact_file(
    tool_name: &str,
    authority: RepositoryRoot<'_>,
    shown: &str,
) -> Result<GuardedRegularFile, String> {
    use contextpatch_core::fs::rooted::{self, RootedEntryKind};

    match rooted::entry_kind(authority, shown).map_err(|error| {
        format!("{tool_name} refused: failed to inspect artifact `{shown}`: {error}")
    })? {
        None => {
            return Err(format!(
                "{tool_name} refused: artifact `{shown}` does not exist"
            ))
        }
        Some(RootedEntryKind::Symlink) => {
            return Err(format!(
                "{tool_name} refused: artifact `{shown}` contains a symlink component"
            ))
        }
        Some(RootedEntryKind::RegularFile) => {}
        Some(_) => {
            return Err(format!(
                "{tool_name} refused: artifact `{shown}` is not a regular file"
            ))
        }
    }
    open_regular_file_in_root(authority, Path::new(shown)).map_err(|error| {
        format!("{tool_name} refused: failed to inspect artifact `{shown}`: {error}")
    })
}

/// Validate many exact replacements before applying them one file at a time.
///
/// Registering a single tool in this repository takes six single-file edits: name, handler, dispatch
/// arm, schema, module export, capability entry. Six round trips is six chances to lose the answer to a
/// transport wedge, and losing one mid-sequence leaves a half-wired feature that compiles. This makes it
/// one call.
///
/// Every entry is validated before the first write. Application is not a cross-file transaction: each
/// file is revalidated and atomically replaced under its target lock, and a later failure can leave an
/// applied prefix. Each attempt is journalled separately for recovery through `read_write_receipts`.
pub(crate) fn call_bulk_replace_exact<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    struct ParsedEntry {
        path: String,
        old: String,
        new: String,
        expected_sha256: Option<String>,
    }

    let tool_name = tools::bulk_replace_exact::NAME;
    let authority = repository_root.into();
    // Receipts are keyed by path; planning and applying go through the authority.
    let journal_root = label(authority, tool_name)?;
    let entries = arguments
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{tool_name} refused: entries is required"))?;
    if entries.is_empty() {
        return Err(format!("{tool_name} refused: entries must not be empty"));
    }
    if entries.len() > contextpatch_core::replace::exact::MAX_BULK_REPLACE_ENTRIES {
        return Err(format!(
            "{tool_name} refused: {} entries exceeds maximum {}",
            entries.len(),
            contextpatch_core::replace::exact::MAX_BULK_REPLACE_ENTRIES
        ));
    }

    // Parse the complete request before core starts reading targets. This keeps malformed later entries
    // in the same validation-before-write phase as missing anchors and stale hashes.
    let mut parsed = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let entry = entry
            .as_object()
            .ok_or_else(|| format!("{tool_name} refused: entry {index} must be an object"))?;
        let path = required_string(entry, "path")
            .map_err(|error| format!("{tool_name} refused: entry {index}: {error}"))?;
        let old = required_string(entry, "old")
            .map_err(|error| format!("{tool_name} refused: entry {index}: {error}"))?;
        let new = required_string(entry, "new")
            .map_err(|error| format!("{tool_name} refused: entry {index}: {error}"))?;
        let expected_sha256 = optional_string(entry, "expected_sha256")
            .map_err(|error| format!("{tool_name} refused: entry {index}: {error}"))?;
        let normalized = normalize_repo_relative_path(tool_name, path).map_err(|error| {
            let prefix = format!("{tool_name} refused: ");
            let detail = error.strip_prefix(&prefix).unwrap_or(&error);
            format!("{tool_name} refused: entry {index}: {detail}")
        })?;
        parsed.push(ParsedEntry {
            path: normalized,
            old: old.to_string(),
            new: new.to_string(),
            expected_sha256: expected_sha256.map(str::to_string),
        });
    }

    let core_entries: Vec<contextpatch_core::replace::exact::ReplaceExactEntry<'_>> = parsed
        .iter()
        .map(
            |entry| contextpatch_core::replace::exact::ReplaceExactEntry {
                path: Path::new(&entry.path),
                old: &entry.old,
                new: &entry.new,
                expected_sha256: entry.expected_sha256.as_deref(),
            },
        )
        .collect();
    let planned = match contextpatch_core::replace::exact::plan_bulk_replace_exact_in_root(
        authority,
        &core_entries,
    ) {
        Ok(planned) => planned,
        Err(error) => {
            // Validation completes before the first write, so a refusal here leaves every target
            // alone. Journal that fact: without it, a refused batch is indistinguishable from a batch
            // that was never attempted. Targets are sorted and deduplicated so the receipts appear in
            // the same order the planner would have used, and a file named by several hunks is
            // journalled once, because one file would have received one write.
            let mut targets: Vec<String> = parsed.iter().map(|entry| entry.path.clone()).collect();
            targets.sort();
            targets.dedup();

            let mut refusal =
                format!("{tool_name} refused during validation: {error}; no file was changed");
            for auxiliary in
                crate::tools::journal::record_refused_batch(&journal_root, tool_name, &targets)
            {
                refusal.push_str("; ");
                refusal.push_str(&auxiliary);
            }
            return Err(refusal);
        }
    };

    // One receipt per file, matching the one atomic write per file that follows. Entries that share a
    // file share its receipt, because they share its write.
    let receipt_paths = planned
        .iter()
        .map(|plan| plan.relative_path().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let mut receipt_batch =
        crate::tools::journal::begin_file_batch(&journal_root, tool_name, &receipt_paths).map_err(
            |error| format!("{error}; receipt capacity was not reserved, so no file was changed"),
        )?;

    let mut applied: Vec<Value> = Vec::with_capacity(parsed.len());
    for (index, plan) in planned.iter().enumerate() {
        let path = plan.relative_path().to_string_lossy().to_string();
        let result = crate::tools::journal::recorded_in_batch(
            &mut receipt_batch,
            &journal_root,
            tool_name,
            &path,
            || {
                contextpatch_core::replace::exact::apply_planned_replacement(authority, plan)
                    .map_err(|error| format!("{tool_name} refused for `{path}`: {error}"))
            },
        );
        let summary = match result {
            Ok(summary) => summary,
            Err(error) => {
                return Err(format!(
                    "{tool_name} apply phase stopped at file {index} (`{path}`) after {} confirmed \
                     files: {error}. The batch is not rolled back; an applied prefix may remain \
                     and the current file's outcome may be unknown. Inspect read_write_receipts \
                     and file_info before retrying",
                    index
                ));
            }
        };
        // One result per submitted entry, so a caller always learns where its own anchor landed.
        // `bytes_written` is the size of the single write that carried every hunk for this file.
        for hunk in &summary.hunks {
            applied.push(json!({
                "entry": hunk.entry_index,
                "path": path,
                "start_byte": hunk.start_byte,
                "end_byte": hunk.end_byte,
                "bytes_written": summary.bytes_written,
                "sha256": summary.sha256
            }));
        }
    }

    // Restore submission order, because plans are grouped and sorted by path.
    applied.sort_by_key(|entry| entry.get("entry").and_then(Value::as_u64).unwrap_or_default());

    serde_json::to_string_pretty(&json!({
        "tool": tool_name,
        "applied": applied.len(),
        "files": planned.len(),
        "atomicity": "per_file",
        "entries": applied
    }))
    .map_err(|error| format!("{tool_name} refused: {error}"))
}

fn write_artifact<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    path: &str,
    bytes: &[u8],
    parents: bool,
    tool_name: &str,
) -> Result<String, String> {
    let artifact_root = artifact_root(repository_root, tool_name)?;
    let relative = validate_relative_path(tool_name, path)?;
    // The artifact directory is its own authority: one no-follow open of a directory this process created,
    // then every component beneath it reached relative to that descriptor. Parent creation goes through the
    // same guarded layer rather than a joined `create_dir_all`.
    let summary =
        write_new_file_bytes_with_parents_in_root(&artifact_root, &relative, bytes, parents)
            .map_err(|error| format!("{tool_name} refused: {error}"))?;

    serde_json::to_string_pretty(&json!({
        "tool": tool_name,
        "created": true,
        "artifact_root": artifact_root.display().to_string(),
        "path": summary.path.display().to_string(),
        "bytes_written": summary.bytes_written,
        "sha256": sha256_hex(bytes),
        "repo_mutation": false
    }))
    .map_err(|error| format!("{tool_name} refused: {error}"))
}

/// The sidecar directory for one repository, created if absent.
///
/// Derived from repository identity rather than from the repository's path spelling, so it follows a rename
/// and is never inherited by a different repository that acquires the old name. The returned path is the
/// location of a directory outside the worktree; it is not authority over the repository.
pub(crate) fn artifact_root<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    tool_name: &str,
) -> Result<PathBuf, String> {
    contextpatch_core::fs::artifact::ensure_artifact_root(repository_root)
        .map_err(|error| format!("{tool_name} refused: {error}"))
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

pub(crate) fn call_create_directory<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let parents = optional_bool(arguments, "parents")?.unwrap_or(false);

    let summary = create_directory_in_root(repository_root.into(), Path::new(path), parents)
        .map_err(|error| format!("create_directory refused: {error}"))?;

    Ok(format!(
        "created directory {} ({} directories created)",
        summary.path.display(),
        summary.directories_created.len()
    ))
}
