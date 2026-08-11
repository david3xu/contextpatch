use serde_json::{json, Value};

use super::*;

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
    let _target_lock = contextpatch_core::fs::mutation_lock::try_file_mutation_lock_for_open_file(
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
pub(super) fn open_exact_artifact_file(
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

pub(super) fn write_artifact<'a>(
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
