use serde_json::{json, Value};

use contextpatch_core::git::commit as core_commit;

use crate::tools;
use crate::tools::common::*;
use crate::tools::git::support::*;

/// Address a moved-policy refusal to the calling tool.
fn refused(tool_name: &str, error: contextpatch_core::error::ContextPatchError) -> String {
    format!("{tool_name} refused: {error}")
}

/// Address a refusal that happened after staging, which is a different claim from refusing before it.
fn refused_after_staging(
    tool_name: &str,
    error: contextpatch_core::error::ContextPatchError,
) -> String {
    format!("{tool_name} refused after staging: {error}")
}

/// Read the commit message arguments shared by all three commit shapes.
fn commit_message(
    arguments: &serde_json::Map<String, Value>,
) -> Result<(String, String), String> {
    let subject = validate_commit_subject(required_string(arguments, "subject")?)?;
    let body = optional_string(arguments, "body")?
        .map(validate_commit_body)
        .transpose()?
        .unwrap_or_default();
    Ok((subject, body))
}

pub(crate) fn call_git_commit_exact<'a>(
    repository: impl Into<contextpatch_core::git::GitRepository<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "commit exact paths";
    const TOOL: &str = tools::git_commit_exact::NAME;

    let paths = required_string_array(arguments, "paths")?;
    let (subject, body) = commit_message(arguments)?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("git_commit_exact refused: paths must not be empty".to_string());
    }
    if paths.len() > core_commit::MAX_COMMIT_PATHS {
        return Err(format!(
            "git_commit_exact refused: at most {} paths may be committed",
            core_commit::MAX_COMMIT_PATHS
        ));
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_commit_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let repository = repository.into();
    let policy = repository_under_policy(repository, TOOL, WorktreeRootPolicy::ResolvedPath)?;
    let root = policy.bind(repository);
    // Path confinement resolves caller-named paths against a root, so it needs the logical path. That is
    // one of the three remaining path-backed boundaries, alongside locks and the receipt journal.
    let normalized_paths = normalize_git_paths(TOOL, root.path(), &paths)?;
    let plan = core_commit::plan_commit_exact(root, &normalized_paths)
        .map_err(|error| refused(TOOL, error))?;

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": TOOL,
            "dry_run": true,
            "would_stage_paths": plan.paths,
            "would_commit": true,
            "subject": subject,
            "body_present": !body.is_empty(),
            "push": false,
            "required_confirm_for_commit": CONFIRMATION
        }))
        .map_err(|error| format!("git_commit_exact refused: {error}"));
    }

    let before_git_head = git_head_for_tool(TOOL, root)?;
    let receipt_id = crate::tools::journal::begin_git(root.path(), TOOL, &before_git_head)?;
    let result = (|| {
        core_commit::stage_paths(root, &plan.paths).map_err(|error| refused(TOOL, error))?;
        core_commit::verify_exact_staged(root, &plan)
            .map_err(|error| refused_after_staging(TOOL, error))?;
        core_commit::commit_staged(root, &subject, &body).map_err(|error| {
            format!(
                "{}\nindex may contain staged exact paths because git commit failed after staging",
                refused(TOOL, error)
            )
        })?;

        let identity =
            core_commit::read_commit_identity(root).map_err(|error| refused(TOOL, error))?;
        serde_json::to_string_pretty(&json!({
            "tool": TOOL,
            "dry_run": false,
            "committed": true,
            "commit": identity.commit,
            "short_commit": identity.short_commit,
            "paths": plan.paths,
            "push": false,
            "status_short": identity.status_short
        }))
        .map_err(|error| format!("git_commit_exact refused: {error}"))
    })();
    let after_git_head = git_head_for_tool(TOOL, root);
    crate::tools::journal::settle_git(
        root.path(),
        TOOL,
        &receipt_id,
        &before_git_head,
        after_git_head,
        result,
    )
}

pub(crate) fn call_git_commit_scoped<'a>(
    repository: impl Into<contextpatch_core::git::GitRepository<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "commit scoped paths";
    const TOOL: &str = tools::git_commit_scoped::NAME;

    let paths = required_string_array(arguments, "paths")?;
    let (subject, body) = commit_message(arguments)?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("git_commit_scoped refused: paths must not be empty".to_string());
    }
    if paths.len() > core_commit::MAX_COMMIT_PATHS {
        return Err(format!(
            "git_commit_scoped refused: at most {} paths may be committed",
            core_commit::MAX_COMMIT_PATHS
        ));
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_commit_scoped refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let repository = repository.into();
    let policy = repository_under_policy(repository, TOOL, WorktreeRootPolicy::ResolvedPath)?;
    let root = policy.bind(repository);
    let normalized_paths = normalize_git_paths(TOOL, root.path(), &paths)?;
    let plan = core_commit::plan_commit_scoped(root, &normalized_paths)
        .map_err(|error| refused(TOOL, error))?;

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": TOOL,
            "dry_run": true,
            "would_stage_paths": plan.paths,
            "would_commit": true,
            "subject": subject,
            "body_present": !body.is_empty(),
            "push": false,
            "remaining_dirty_paths_after_commit": plan.remaining_dirty_after,
            "required_confirm_for_commit": CONFIRMATION
        }))
        .map_err(|error| format!("git_commit_scoped refused: {error}"));
    }

    let before_git_head = git_head_for_tool(TOOL, root)?;
    let receipt_id = crate::tools::journal::begin_git(root.path(), TOOL, &before_git_head)?;
    let result = (|| {
        core_commit::stage_paths(root, &plan.paths).map_err(|error| refused(TOOL, error))?;
        core_commit::verify_scoped_staged(root, &plan)
            .map_err(|error| refused_after_staging(TOOL, error))?;
        core_commit::commit_staged(root, &subject, &body).map_err(|error| {
            format!(
                "{}\nindex may contain staged scoped paths because git commit failed after staging",
                refused(TOOL, error)
            )
        })?;

        let (dirty_after, still_dirty) =
            core_commit::requested_still_dirty(root, &plan.requested_paths)
                .map_err(|error| refused(TOOL, error))?;
        if !still_dirty.is_empty() {
            return Err(format!(
                "git_commit_scoped refused after commit: requested paths are still dirty\n{}",
                format_set(&still_dirty)
            ));
        }

        let identity =
            core_commit::read_commit_identity(root).map_err(|error| refused(TOOL, error))?;
        serde_json::to_string_pretty(&json!({
            "tool": TOOL,
            "dry_run": false,
            "committed": true,
            "commit": identity.commit,
            "short_commit": identity.short_commit,
            "paths": plan.paths,
            "push": false,
            "remaining_dirty_paths": dirty_after,
            "status_short": git_status_short(root)?
        }))
        .map_err(|error| format!("git_commit_scoped refused: {error}"))
    })();
    let after_git_head = git_head_for_tool(TOOL, root);
    crate::tools::journal::settle_git(
        root.path(),
        TOOL,
        &receipt_id,
        &before_git_head,
        after_git_head,
        result,
    )
}

pub(crate) fn call_git_commit_prefix<'a>(
    repository: impl Into<contextpatch_core::git::GitRepository<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "commit prefix paths";
    const TOOL: &str = tools::git_commit_prefix::NAME;

    let prefixes = required_string_array(arguments, "prefixes")?;
    let (subject, body) = commit_message(arguments)?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if prefixes.is_empty() {
        return Err("git_commit_prefix refused: prefixes must not be empty".to_string());
    }
    if prefixes.len() > core_commit::MAX_COMMIT_PREFIXES {
        return Err(format!(
            "git_commit_prefix refused: at most {} prefixes may be expanded",
            core_commit::MAX_COMMIT_PREFIXES
        ));
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_commit_prefix refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let repository = repository.into();
    let policy = repository_under_policy(repository, TOOL, WorktreeRootPolicy::ResolvedPath)?;
    let root = policy.bind(repository);
    let normalized_prefixes = normalize_git_prefixes(TOOL, root.path(), &prefixes)?;
    let plan = core_commit::plan_commit_prefix(root, &normalized_prefixes)
        .map_err(|error| refused(TOOL, error))?;

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": TOOL,
            "dry_run": true,
            "prefixes": plan.prefixes,
            "expanded_dirty_paths": plan.paths,
            "expanded_path_count": plan.requested_paths.len(),
            "would_commit": true,
            "subject": subject,
            "body_present": !body.is_empty(),
            "push": false,
            "remaining_dirty_paths_after_commit": plan.remaining_dirty_after,
            "required_confirm_for_commit": CONFIRMATION
        }))
        .map_err(|error| format!("git_commit_prefix refused: {error}"));
    }

    let before_git_head = git_head_for_tool(TOOL, root)?;
    let receipt_id = crate::tools::journal::begin_git(root.path(), TOOL, &before_git_head)?;
    let result = (|| {
        core_commit::stage_paths(root, &plan.paths).map_err(|error| refused(TOOL, error))?;
        core_commit::verify_prefix_staged(root, &plan)
            .map_err(|error| refused_after_staging(TOOL, error))?;
        core_commit::commit_staged(root, &subject, &body).map_err(|error| {
            format!(
                "{}\nindex may contain staged prefix-expanded paths because git commit failed after staging",
                refused(TOOL, error)
            )
        })?;

        let (dirty_after, still_dirty) =
            core_commit::requested_still_dirty(root, &plan.requested_paths)
                .map_err(|error| refused(TOOL, error))?;
        if !still_dirty.is_empty() {
            return Err(format!(
                "git_commit_prefix refused after commit: expanded paths are still dirty\n{}",
                format_set(&still_dirty)
            ));
        }

        let identity =
            core_commit::read_commit_identity(root).map_err(|error| refused(TOOL, error))?;
        serde_json::to_string_pretty(&json!({
            "tool": TOOL,
            "dry_run": false,
            "committed": true,
            "commit": identity.commit,
            "short_commit": identity.short_commit,
            "prefixes": plan.prefixes,
            "paths": plan.paths,
            "push": false,
            "remaining_dirty_paths": dirty_after,
            "status_short": git_status_short(root)?
        }))
        .map_err(|error| format!("git_commit_prefix refused: {error}"))
    })();
    let after_git_head = git_head_for_tool(TOOL, root);
    crate::tools::journal::settle_git(
        root.path(),
        TOOL,
        &receipt_id,
        &before_git_head,
        after_git_head,
        result,
    )
}

pub(crate) fn call_git_stage_exact<'a>(
    repository: impl Into<contextpatch_core::git::GitRepository<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "stage exact paths";
    const TOOL: &str = tools::git_stage_exact::NAME;

    let paths = required_string_array(arguments, "paths")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("git_stage_exact refused: paths must not be empty".to_string());
    }
    if paths.len() > core_commit::MAX_STAGE_PATHS {
        return Err(format!(
            "git_stage_exact refused: at most {} paths may be staged",
            core_commit::MAX_STAGE_PATHS
        ));
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_stage_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let repository = repository.into();
    let policy = repository_under_policy(repository, TOOL, WorktreeRootPolicy::ResolvedPath)?;
    let root = policy.bind(repository);
    let normalized_paths = normalize_git_paths(TOOL, root.path(), &paths)?;
    let plan = core_commit::plan_stage_exact(root, &normalized_paths)
        .map_err(|error| refused(TOOL, error))?;

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": TOOL,
            "dry_run": true,
            "would_stage_paths": plan.paths,
            "would_commit": false,
            "required_confirm_for_stage": CONFIRMATION
        }))
        .map_err(|error| format!("git_stage_exact refused: {error}"));
    }

    core_commit::stage_paths(root, &plan.paths).map_err(|error| refused(TOOL, error))?;
    core_commit::verify_stage_exact(root, &plan)
        .map_err(|error| refused_after_staging(TOOL, error))?;
    let status = git_status_short(root)?;

    serde_json::to_string_pretty(&json!({
        "tool": TOOL,
        "dry_run": false,
        "staged": true,
        "committed": false,
        "paths": plan.paths,
        "status_short": status
    }))
    .map_err(|error| format!("git_stage_exact refused: {error}"))
}

pub(crate) fn call_git_staged_scope_check<'a>(
    repository: impl Into<contextpatch_core::git::GitRepository<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const TOOL: &str = tools::git_staged_scope_check::NAME;

    let allowed_paths = optional_string_array(arguments, "allowed_paths")?;
    let allowed_prefixes = optional_string_array(arguments, "allowed_prefixes")?;
    let required_paths = optional_string_array(arguments, "required_paths")?;

    if allowed_paths.len() > core_commit::MAX_SCOPE_CHECK_PATHS {
        return Err(format!(
            "git_staged_scope_check refused: at most {} allowed_paths are supported",
            core_commit::MAX_SCOPE_CHECK_PATHS
        ));
    }
    if allowed_prefixes.len() > core_commit::MAX_SCOPE_CHECK_PREFIXES {
        return Err(format!(
            "git_staged_scope_check refused: at most {} allowed_prefixes are supported",
            core_commit::MAX_SCOPE_CHECK_PREFIXES
        ));
    }
    if required_paths.len() > core_commit::MAX_SCOPE_CHECK_PATHS {
        return Err(format!(
            "git_staged_scope_check refused: at most {} required_paths are supported",
            core_commit::MAX_SCOPE_CHECK_PATHS
        ));
    }

    let repository = repository.into();
    let policy = repository_under_policy(repository, TOOL, WorktreeRootPolicy::ResolvedPath)?;
    let root = policy.bind(repository);
    let normalized_allowed_paths = normalize_git_paths(TOOL, root.path(), &allowed_paths)?;
    let normalized_allowed_prefixes = normalize_git_prefixes(TOOL, root.path(), &allowed_prefixes)?;
    let normalized_required_paths = normalize_git_paths(TOOL, root.path(), &required_paths)?;

    let report = core_commit::evaluate_staged_scope(
        root,
        &normalized_allowed_paths,
        &normalized_allowed_prefixes,
        &normalized_required_paths,
    )
    .map_err(|error| refused(TOOL, error))?;

    serde_json::to_string_pretty(&json!({
        "tool": TOOL,
        "read_only": true,
        "passed": report.passed,
        "staged_paths": report.staged_paths,
        "staged_path_count": report.staged_paths.len(),
        "allowed_paths": normalized_allowed_paths,
        "allowed_prefixes": normalized_allowed_prefixes,
        "required_paths": normalized_required_paths,
        "disallowed_paths": report.disallowed_paths,
        "missing_required_paths": report.missing_required_paths
    }))
    .map_err(|error| format!("git_staged_scope_check refused: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn moved_commit_policy_keeps_its_refusal_prefixes() {
        // Planning now lives in core, which returns the detail only. A duplicate entry is the cheapest
        // core-originated refusal to reach for each shape, so it pins the seam without a repository.
        let root = std::env::temp_dir();
        let message = json!({"subject": "Subject"});
        let mut exact = arguments(message.clone());
        exact.insert("paths".to_string(), json!(["a.txt", "a.txt"]));
        let mut scoped = arguments(message.clone());
        scoped.insert("paths".to_string(), json!(["a.txt", "a.txt"]));
        let mut prefix = arguments(message);
        prefix.insert("prefixes".to_string(), json!(["src", "src"]));

        assert_eq!(
            call_git_commit_exact(&root, &exact).unwrap_err(),
            "git_commit_exact refused: duplicate paths are not allowed"
        );
        assert_eq!(
            call_git_commit_scoped(&root, &scoped).unwrap_err(),
            "git_commit_scoped refused: duplicate paths are not allowed"
        );
        assert_eq!(
            call_git_commit_prefix(&root, &prefix).unwrap_err(),
            "git_commit_prefix refused: duplicate prefixes are not allowed"
        );
        assert_eq!(
            call_git_stage_exact(&root, &arguments(json!({"paths": ["a.txt", "a.txt"]})))
                .unwrap_err(),
            "git_stage_exact refused: duplicate paths are not allowed"
        );
        assert_eq!(
            call_git_staged_scope_check(
                &root,
                &arguments(json!({"allowed_paths": ["a.txt", "a.txt"]}))
            )
            .unwrap_err(),
            "git_staged_scope_check refused: duplicate allowed_paths are not allowed"
        );
    }

    #[test]
    fn limits_and_confirmations_are_still_checked_before_any_repository_work() {
        // These stay in the adapter, ahead of normalization and planning, so the order in which a caller
        // learns a request is malformed is unchanged.
        let root = std::env::temp_dir();
        let too_many: Vec<String> = (0..=core_commit::MAX_COMMIT_PATHS)
            .map(|index| format!("file{index}.txt"))
            .collect();

        assert_eq!(
            call_git_commit_exact(
                &root,
                &arguments(json!({"paths": too_many, "subject": "Subject"}))
            )
            .unwrap_err(),
            "git_commit_exact refused: at most 100 paths may be committed"
        );
        assert_eq!(
            call_git_commit_exact(&root, &arguments(json!({"paths": [], "subject": "Subject"})))
                .unwrap_err(),
            "git_commit_exact refused: paths must not be empty"
        );
        assert_eq!(
            call_git_commit_exact(
                &root,
                &arguments(json!({"paths": ["a.txt"], "subject": "Subject", "dry_run": false}))
            )
            .unwrap_err(),
            "git_commit_exact refused: dry_run=false requires confirm: \"commit exact paths\""
        );
        assert_eq!(
            call_git_stage_exact(
                &root,
                &arguments(json!({"paths": ["a.txt"], "dry_run": false}))
            )
            .unwrap_err(),
            "git_stage_exact refused: dry_run=false requires confirm: \"stage exact paths\""
        );
    }

    #[test]
    fn a_commit_message_is_still_validated_before_paths_are_resolved() {
        // The subject refusal stays addressed to git_commit_exact even for the other commit tools,
        // because that is the string callers have always received.
        let root = std::env::temp_dir();

        assert_eq!(
            call_git_commit_scoped(&root, &arguments(json!({"paths": ["a.txt"], "subject": "  "})))
                .unwrap_err(),
            "git_commit_exact refused: subject must not be empty"
        );
    }
}
