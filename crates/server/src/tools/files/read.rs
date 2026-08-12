use serde_json::{json, Value};

use super::*;

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
    let repo_root = label(repository_root.into(), tools::read_write_receipts::NAME)?;
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

pub(super) fn file_info_value(root: RepositoryRoot<'_>, path: &str) -> Result<Value, String> {
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
