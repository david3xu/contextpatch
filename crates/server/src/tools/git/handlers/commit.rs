use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::*;
use crate::tools::git::support::*;

pub(crate) fn call_git_commit_exact(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "commit exact paths";

    let paths = required_string_array(arguments, "paths")?;
    let subject = validate_commit_subject(required_string(arguments, "subject")?)?;
    let body = optional_string(arguments, "body")?
        .map(validate_commit_body)
        .transpose()?
        .unwrap_or_default();
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("git_commit_exact refused: paths must not be empty".to_string());
    }
    if paths.len() > 100 {
        return Err("git_commit_exact refused: at most 100 paths may be committed".to_string());
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_commit_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = repo_root.canonicalize().map_err(|error| {
        format!("git_commit_exact refused: failed to resolve repo root: {error}")
    })?;
    let normalized_paths = normalize_git_paths(tools::git_commit_exact::NAME, &root, &paths)?;
    let expected_paths: BTreeSet<String> = normalized_paths.iter().cloned().collect();
    if expected_paths.len() != normalized_paths.len() {
        return Err("git_commit_exact refused: duplicate paths are not allowed".to_string());
    }

    let dirty_paths = git_status_paths(&root)?;
    if dirty_paths.is_empty() {
        return Err("git_commit_exact refused: repository has no dirty paths".to_string());
    }
    if dirty_paths != expected_paths {
        return Err(format!(
            "git_commit_exact refused: provided paths must exactly match the full dirty-path set\nexpected_paths:\n{}\nactual_dirty_paths:\n{}\nmissing_from_input:\n{}\nunexpected_in_input:\n{}",
            format_set(&dirty_paths),
            format_set(&expected_paths),
            format_set(&dirty_paths.difference(&expected_paths).cloned().collect()),
            format_set(&expected_paths.difference(&dirty_paths).cloned().collect())
        ));
    }

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::git_commit_exact::NAME,
            "dry_run": true,
            "would_stage_paths": normalized_paths,
            "would_commit": true,
            "subject": subject,
            "body_present": !body.is_empty(),
            "push": false,
            "required_confirm_for_commit": CONFIRMATION
        }))
        .map_err(|error| format!("git_commit_exact refused: {error}"));
    }

    let before_git_head = git_head_for_tool(tools::git_commit_exact::NAME, &root)?;
    let receipt_id =
        crate::tools::journal::begin_git(&root, tools::git_commit_exact::NAME, &before_git_head)?;
    let result = (|| {
        git_success(&root, git_args("add", &normalized_paths))?;
        let staged_paths = git_cached_paths(&root)?;
        if staged_paths != expected_paths {
            return Err(format!(
                "git_commit_exact refused after staging: staged paths differ from requested exact set\nrequested:\n{}\nstaged:\n{}",
                format_set(&expected_paths),
                format_set(&staged_paths)
            ));
        }
        let dirty_paths_after_stage = git_status_paths(&root)?;
        if dirty_paths_after_stage != expected_paths {
            return Err(format!(
                "git_commit_exact refused after staging: dirty paths changed before commit\nrequested:\n{}\ncurrent_dirty_paths:\n{}",
                format_set(&expected_paths),
                format_set(&dirty_paths_after_stage)
            ));
        }

        let mut commit_args = vec![
            "commit".to_string(),
            "--quiet".to_string(),
            "-m".to_string(),
            subject.clone(),
        ];
        if !body.is_empty() {
            commit_args.push("-m".to_string());
            commit_args.push(body);
        }
        git_success(&root, commit_args).map_err(|error| {
            format!(
                "{error}\nindex may contain staged exact paths because git commit failed after staging"
            )
        })?;

        let commit = git_stdout(&root, &["rev-parse", "HEAD"])?;
        let short_commit = git_stdout(&root, &["rev-parse", "--short", "HEAD"])?;
        let status = git_stdout(&root, &["status", "--short"])?;

        serde_json::to_string_pretty(&json!({
            "tool": tools::git_commit_exact::NAME,
            "dry_run": false,
            "committed": true,
            "commit": commit.trim(),
            "short_commit": short_commit.trim(),
            "paths": normalized_paths,
            "push": false,
            "status_short": status
        }))
        .map_err(|error| format!("git_commit_exact refused: {error}"))
    })();
    let after_git_head = git_head_for_tool(tools::git_commit_exact::NAME, &root);
    crate::tools::journal::settle_git(
        &root,
        tools::git_commit_exact::NAME,
        &receipt_id,
        &before_git_head,
        after_git_head,
        result,
    )
}

pub(crate) fn call_git_commit_scoped(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "commit scoped paths";

    let paths = required_string_array(arguments, "paths")?;
    let subject = validate_commit_subject(required_string(arguments, "subject")?)?;
    let body = optional_string(arguments, "body")?
        .map(validate_commit_body)
        .transpose()?
        .unwrap_or_default();
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("git_commit_scoped refused: paths must not be empty".to_string());
    }
    if paths.len() > 100 {
        return Err("git_commit_scoped refused: at most 100 paths may be committed".to_string());
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_commit_scoped refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = repo_root_for_policy(
        repo_root,
        tools::git_commit_scoped::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
    let normalized_paths = normalize_git_paths(tools::git_commit_scoped::NAME, &root, &paths)?;
    let requested_paths: BTreeSet<String> = normalized_paths.iter().cloned().collect();
    if requested_paths.len() != normalized_paths.len() {
        return Err("git_commit_scoped refused: duplicate paths are not allowed".to_string());
    }

    let dirty_paths = git_status_paths_for_tool(tools::git_commit_scoped::NAME, &root)?;
    if dirty_paths.is_empty() {
        return Err("git_commit_scoped refused: repository has no dirty paths".to_string());
    }
    let not_dirty = requested_paths
        .difference(&dirty_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !not_dirty.is_empty() {
        return Err(format!(
            "git_commit_scoped refused: every requested path must currently be dirty\nnot_dirty_paths:\n{}",
            format_set(&not_dirty)
        ));
    }

    let staged_before = git_cached_paths_for_tool(tools::git_commit_scoped::NAME, &root)?;
    if !staged_before.is_empty() {
        return Err(format!(
            "git_commit_scoped refused: index must be clean before scoped commit so unrelated staged work is not included\nstaged_paths:\n{}",
            format_set(&staged_before)
        ));
    }

    let remaining_dirty_paths_after_commit = dirty_paths
        .difference(&requested_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::git_commit_scoped::NAME,
            "dry_run": true,
            "would_stage_paths": normalized_paths,
            "would_commit": true,
            "subject": subject,
            "body_present": !body.is_empty(),
            "push": false,
            "remaining_dirty_paths_after_commit": remaining_dirty_paths_after_commit,
            "required_confirm_for_commit": CONFIRMATION
        }))
        .map_err(|error| format!("git_commit_scoped refused: {error}"));
    }

    let before_git_head = git_head_for_tool(tools::git_commit_scoped::NAME, &root)?;
    let receipt_id =
        crate::tools::journal::begin_git(&root, tools::git_commit_scoped::NAME, &before_git_head)?;
    let result = (|| {
        git_success_for_tool(
            tools::git_commit_scoped::NAME,
            &root,
            git_args("add", &normalized_paths),
        )?;
        let staged_paths = git_cached_paths_for_tool(tools::git_commit_scoped::NAME, &root)?;
        if staged_paths != requested_paths {
            return Err(format!(
                "git_commit_scoped refused after staging: staged paths differ from requested scoped set\nrequested:\n{}\nstaged:\n{}",
                format_set(&requested_paths),
                format_set(&staged_paths)
            ));
        }

        let mut commit_args = vec![
            "commit".to_string(),
            "--quiet".to_string(),
            "-m".to_string(),
            subject.clone(),
        ];
        if !body.is_empty() {
            commit_args.push("-m".to_string());
            commit_args.push(body);
        }
        git_success_for_tool(tools::git_commit_scoped::NAME, &root, commit_args).map_err(
            |error| {
                format!(
                    "{error}\nindex may contain staged scoped paths because git commit failed after staging"
                )
            },
        )?;

        let dirty_after = git_status_paths_for_tool(tools::git_commit_scoped::NAME, &root)?;
        let requested_still_dirty = requested_paths
            .intersection(&dirty_after)
            .cloned()
            .collect::<BTreeSet<_>>();
        if !requested_still_dirty.is_empty() {
            return Err(format!(
                "git_commit_scoped refused after commit: requested paths are still dirty\n{}",
                format_set(&requested_still_dirty)
            ));
        }

        let commit = git_stdout_for_tool(
            tools::git_commit_scoped::NAME,
            &root,
            &["rev-parse", "HEAD"],
        )?;
        let short_commit = git_stdout_for_tool(
            tools::git_commit_scoped::NAME,
            &root,
            &["rev-parse", "--short", "HEAD"],
        )?;
        let status = git_status_short(&root)?;

        serde_json::to_string_pretty(&json!({
            "tool": tools::git_commit_scoped::NAME,
            "dry_run": false,
            "committed": true,
            "commit": commit.trim(),
            "short_commit": short_commit.trim(),
            "paths": normalized_paths,
            "push": false,
            "remaining_dirty_paths": dirty_after,
            "status_short": status
        }))
        .map_err(|error| format!("git_commit_scoped refused: {error}"))
    })();
    let after_git_head = git_head_for_tool(tools::git_commit_scoped::NAME, &root);
    crate::tools::journal::settle_git(
        &root,
        tools::git_commit_scoped::NAME,
        &receipt_id,
        &before_git_head,
        after_git_head,
        result,
    )
}

pub(crate) fn call_git_commit_prefix(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "commit prefix paths";
    const MAX_EXPANDED_PATHS: usize = 5000;

    let prefixes = required_string_array(arguments, "prefixes")?;
    let subject = validate_commit_subject(required_string(arguments, "subject")?)?;
    let body = optional_string(arguments, "body")?
        .map(validate_commit_body)
        .transpose()?
        .unwrap_or_default();
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if prefixes.is_empty() {
        return Err("git_commit_prefix refused: prefixes must not be empty".to_string());
    }
    if prefixes.len() > 20 {
        return Err("git_commit_prefix refused: at most 20 prefixes may be expanded".to_string());
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_commit_prefix refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = repo_root_for_policy(
        repo_root,
        tools::git_commit_prefix::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
    let normalized_prefixes =
        normalize_git_prefixes(tools::git_commit_prefix::NAME, &root, &prefixes)?;
    let prefix_set: BTreeSet<String> = normalized_prefixes.iter().cloned().collect();
    if prefix_set.len() != normalized_prefixes.len() {
        return Err("git_commit_prefix refused: duplicate prefixes are not allowed".to_string());
    }

    let dirty_paths = git_status_paths_for_tool(tools::git_commit_prefix::NAME, &root)?;
    if dirty_paths.is_empty() {
        return Err("git_commit_prefix refused: repository has no dirty paths".to_string());
    }
    let requested_paths = paths_under_prefixes(&dirty_paths, &normalized_prefixes);
    if requested_paths.is_empty() {
        return Err(format!(
            "git_commit_prefix refused: no dirty paths matched prefixes\nprefixes:\n{}",
            format_set(&prefix_set)
        ));
    }
    if requested_paths.len() > MAX_EXPANDED_PATHS {
        return Err(format!(
            "git_commit_prefix refused: matched {} dirty paths; maximum is {MAX_EXPANDED_PATHS}",
            requested_paths.len()
        ));
    }

    let staged_before = git_cached_paths_for_tool(tools::git_commit_prefix::NAME, &root)?;
    if !staged_before.is_empty() {
        return Err(format!(
            "git_commit_prefix refused: index must be clean before prefix commit so unrelated staged work is not included\nstaged_paths:\n{}",
            format_set(&staged_before)
        ));
    }

    let normalized_paths = requested_paths.iter().cloned().collect::<Vec<_>>();
    let remaining_dirty_paths_after_commit = dirty_paths
        .difference(&requested_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::git_commit_prefix::NAME,
            "dry_run": true,
            "prefixes": normalized_prefixes,
            "expanded_dirty_paths": normalized_paths,
            "expanded_path_count": requested_paths.len(),
            "would_commit": true,
            "subject": subject,
            "body_present": !body.is_empty(),
            "push": false,
            "remaining_dirty_paths_after_commit": remaining_dirty_paths_after_commit,
            "required_confirm_for_commit": CONFIRMATION
        }))
        .map_err(|error| format!("git_commit_prefix refused: {error}"));
    }

    let before_git_head = git_head_for_tool(tools::git_commit_prefix::NAME, &root)?;
    let receipt_id =
        crate::tools::journal::begin_git(&root, tools::git_commit_prefix::NAME, &before_git_head)?;
    let result = (|| {
        git_success_for_tool(
            tools::git_commit_prefix::NAME,
            &root,
            git_args("add", &normalized_paths),
        )?;
        let staged_paths = git_cached_paths_for_tool(tools::git_commit_prefix::NAME, &root)?;
        if staged_paths != requested_paths {
            return Err(format!(
                "git_commit_prefix refused after staging: staged paths differ from expanded prefix set\nexpanded:\n{}\nstaged:\n{}",
                format_set(&requested_paths),
                format_set(&staged_paths)
            ));
        }

        let mut commit_args = vec![
            "commit".to_string(),
            "--quiet".to_string(),
            "-m".to_string(),
            subject.clone(),
        ];
        if !body.is_empty() {
            commit_args.push("-m".to_string());
            commit_args.push(body);
        }
        git_success_for_tool(tools::git_commit_prefix::NAME, &root, commit_args).map_err(
            |error| {
                format!(
                    "{error}\nindex may contain staged prefix-expanded paths because git commit failed after staging"
                )
            },
        )?;

        let dirty_after = git_status_paths_for_tool(tools::git_commit_prefix::NAME, &root)?;
        let requested_still_dirty = requested_paths
            .intersection(&dirty_after)
            .cloned()
            .collect::<BTreeSet<_>>();
        if !requested_still_dirty.is_empty() {
            return Err(format!(
                "git_commit_prefix refused after commit: expanded paths are still dirty\n{}",
                format_set(&requested_still_dirty)
            ));
        }

        let commit = git_stdout_for_tool(
            tools::git_commit_prefix::NAME,
            &root,
            &["rev-parse", "HEAD"],
        )?;
        let short_commit = git_stdout_for_tool(
            tools::git_commit_prefix::NAME,
            &root,
            &["rev-parse", "--short", "HEAD"],
        )?;
        let status = git_status_short(&root)?;

        serde_json::to_string_pretty(&json!({
            "tool": tools::git_commit_prefix::NAME,
            "dry_run": false,
            "committed": true,
            "commit": commit.trim(),
            "short_commit": short_commit.trim(),
            "prefixes": normalized_prefixes,
            "paths": normalized_paths,
            "push": false,
            "remaining_dirty_paths": dirty_after,
            "status_short": status
        }))
        .map_err(|error| format!("git_commit_prefix refused: {error}"))
    })();
    let after_git_head = git_head_for_tool(tools::git_commit_prefix::NAME, &root);
    crate::tools::journal::settle_git(
        &root,
        tools::git_commit_prefix::NAME,
        &receipt_id,
        &before_git_head,
        after_git_head,
        result,
    )
}

pub(crate) fn call_git_stage_exact(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "stage exact paths";

    let paths = required_string_array(arguments, "paths")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("git_stage_exact refused: paths must not be empty".to_string());
    }
    if paths.len() > 500 {
        return Err("git_stage_exact refused: at most 500 paths may be staged".to_string());
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_stage_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = repo_root_for_policy(
        repo_root,
        tools::git_stage_exact::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
    let normalized_paths = normalize_git_paths(tools::git_stage_exact::NAME, &root, &paths)?;
    let requested_paths: BTreeSet<String> = normalized_paths.iter().cloned().collect();
    if requested_paths.len() != normalized_paths.len() {
        return Err("git_stage_exact refused: duplicate paths are not allowed".to_string());
    }

    let dirty_paths = git_status_paths_for_tool(tools::git_stage_exact::NAME, &root)?;
    if dirty_paths.is_empty() {
        return Err("git_stage_exact refused: repository has no dirty paths".to_string());
    }
    let not_dirty = requested_paths
        .difference(&dirty_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !not_dirty.is_empty() {
        return Err(format!(
            "git_stage_exact refused: every requested path must currently be dirty\nnot_dirty_paths:\n{}",
            format_set(&not_dirty)
        ));
    }

    let staged_before = git_cached_paths_for_tool(tools::git_stage_exact::NAME, &root)?;
    if !staged_before.is_empty() {
        return Err(format!(
            "git_stage_exact refused: index must be clean before staging exact paths\nstaged_paths:\n{}",
            format_set(&staged_before)
        ));
    }

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::git_stage_exact::NAME,
            "dry_run": true,
            "would_stage_paths": normalized_paths,
            "would_commit": false,
            "required_confirm_for_stage": CONFIRMATION
        }))
        .map_err(|error| format!("git_stage_exact refused: {error}"));
    }

    git_success_for_tool(
        tools::git_stage_exact::NAME,
        &root,
        git_args("add", &normalized_paths),
    )?;
    let staged_paths = git_cached_paths_for_tool(tools::git_stage_exact::NAME, &root)?;
    if staged_paths != requested_paths {
        return Err(format!(
            "git_stage_exact refused after staging: staged paths differ from requested exact set\nrequested:\n{}\nstaged:\n{}",
            format_set(&requested_paths),
            format_set(&staged_paths)
        ));
    }
    let status = git_status_short(&root)?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_stage_exact::NAME,
        "dry_run": false,
        "staged": true,
        "committed": false,
        "paths": normalized_paths,
        "status_short": status
    }))
    .map_err(|error| format!("git_stage_exact refused: {error}"))
}

pub(crate) fn call_git_staged_scope_check(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let allowed_paths = optional_string_array(arguments, "allowed_paths")?;
    let allowed_prefixes = optional_string_array(arguments, "allowed_prefixes")?;
    let required_paths = optional_string_array(arguments, "required_paths")?;

    if allowed_paths.len() > 500 {
        return Err(
            "git_staged_scope_check refused: at most 500 allowed_paths are supported".to_string(),
        );
    }
    if allowed_prefixes.len() > 50 {
        return Err(
            "git_staged_scope_check refused: at most 50 allowed_prefixes are supported".to_string(),
        );
    }
    if required_paths.len() > 500 {
        return Err(
            "git_staged_scope_check refused: at most 500 required_paths are supported".to_string(),
        );
    }

    let root = repo_root_for_policy(
        repo_root,
        tools::git_staged_scope_check::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
    let normalized_allowed_paths =
        normalize_git_paths(tools::git_staged_scope_check::NAME, &root, &allowed_paths)?;
    let normalized_allowed_prefixes = normalize_git_prefixes(
        tools::git_staged_scope_check::NAME,
        &root,
        &allowed_prefixes,
    )?;
    let normalized_required_paths =
        normalize_git_paths(tools::git_staged_scope_check::NAME, &root, &required_paths)?;

    let allowed_path_set: BTreeSet<String> = normalized_allowed_paths.iter().cloned().collect();
    if allowed_path_set.len() != normalized_allowed_paths.len() {
        return Err(
            "git_staged_scope_check refused: duplicate allowed_paths are not allowed".to_string(),
        );
    }
    let allowed_prefix_set: BTreeSet<String> =
        normalized_allowed_prefixes.iter().cloned().collect();
    if allowed_prefix_set.len() != normalized_allowed_prefixes.len() {
        return Err(
            "git_staged_scope_check refused: duplicate allowed_prefixes are not allowed"
                .to_string(),
        );
    }
    let required_path_set: BTreeSet<String> = normalized_required_paths.iter().cloned().collect();
    if required_path_set.len() != normalized_required_paths.len() {
        return Err(
            "git_staged_scope_check refused: duplicate required_paths are not allowed".to_string(),
        );
    }

    let staged_paths = git_cached_paths_for_tool(tools::git_staged_scope_check::NAME, &root)?;
    let disallowed_paths = staged_paths
        .iter()
        .filter(|path| {
            !allowed_path_set.contains(*path)
                && !normalized_allowed_prefixes
                    .iter()
                    .any(|prefix| *path == prefix || path.starts_with(&format!("{prefix}/")))
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing_required_paths = required_path_set
        .difference(&staged_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    let passed = disallowed_paths.is_empty() && missing_required_paths.is_empty();

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_staged_scope_check::NAME,
        "read_only": true,
        "passed": passed,
        "staged_paths": staged_paths,
        "staged_path_count": staged_paths.len(),
        "allowed_paths": normalized_allowed_paths,
        "allowed_prefixes": normalized_allowed_prefixes,
        "required_paths": normalized_required_paths,
        "disallowed_paths": disallowed_paths,
        "missing_required_paths": missing_required_paths
    }))
    .map_err(|error| format!("git_staged_scope_check refused: {error}"))
}
