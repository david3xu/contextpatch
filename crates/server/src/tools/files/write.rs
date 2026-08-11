use serde_json::{json, Value};

use super::*;

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
    let summary =
        crate::tools::journal::recorded(&journal_root, tools::replace_exact::NAME, path, || {
            replace_exact_in_root_with_sha256(authority, Path::new(path), old, new, expected_sha256)
                .map_err(|error| format!("replace_exact refused: {error}"))
        })?;

    Ok(format!(
        "replaced bytes {}..{} in {} ({} bytes written); sha256={}",
        summary.start_byte,
        summary.end_byte,
        summary.path.display(),
        summary.bytes_written,
        summary.sha256
    ))
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
    applied.sort_by_key(|entry| {
        entry
            .get("entry")
            .and_then(Value::as_u64)
            .unwrap_or_default()
    });

    serde_json::to_string_pretty(&json!({
        "tool": tool_name,
        "applied": applied.len(),
        "files": planned.len(),
        "atomicity": "per_file",
        "entries": applied
    }))
    .map_err(|error| format!("{tool_name} refused: {error}"))
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
