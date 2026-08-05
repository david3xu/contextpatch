use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{json, Value};

use contextpatch_core::git::sync as core_sync;

use crate::tools;
use crate::tools::common::*;
use crate::tools::git::support::*;

/// Address a moved-policy refusal to the calling tool.
fn refused(tool_name: &str, error: contextpatch_core::error::ContextPatchError) -> String {
    format!("{tool_name} refused: {error}")
}

pub(crate) fn call_git_remote_list(
    repository: contextpatch_core::git::GitRepository<'_>,
) -> Result<String, String> {
    const TOOL: &str = tools::git_remote_list::NAME;

    // An anchored target was resolved and validated when it was selected, so re-resolving its path here
    // would be exactly the reopen this phase removes. Only a path-backed target needs resolving.
    let resolved;
    let target = if repository.is_anchored() {
        repository
    } else {
        resolved = resolved_repo_root(repository.path(), TOOL)?;
        contextpatch_core::git::GitRepository::from_path(&resolved)
    };

    let remotes = core_sync::list_remotes(target)
        .map_err(|error| refused(TOOL, error))?
        .into_iter()
        .map(|remote| {
            json!({
                "name": remote.name,
                "url": remote.url,
                "kind": remote.kind
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_string_pretty(&json!({
        "tool": TOOL,
        "remotes": remotes
    }))
    .map_err(|error| format!("git_remote_list refused: {error}"))
}

pub(crate) fn call_git_remote_check(
    repository: contextpatch_core::git::GitRepository<'_>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const TOOL: &str = tools::git_remote_check::NAME;

    let remote = validate_git_remote(
        optional_string(arguments, "remote")?.unwrap_or(core_sync::DEFAULT_REMOTE),
    )?;
    let branch = validate_git_branch(required_string(arguments, "branch")?)?;
    let policy = repository_under_policy(repository, TOOL, WorktreeRootPolicy::ResolvedPath)?;
    let root = policy.bind(repository);
    ensure_remote_exists(TOOL, root, &remote)?;
    let source_status_before = git_status_short(root)?;

    core_sync::fetch(
        root,
        vec!["fetch".to_string(), remote.clone(), branch.clone()],
    )
    .map_err(|error| refused(TOOL, error))?;

    let source_status_after = git_status_short(root)?;
    core_sync::ensure_status_unchanged(&source_status_before, &source_status_after)
        .map_err(|error| refused(TOOL, error))?;

    let remote_ref = core_sync::remote_tracking_ref(&remote, &branch);
    let divergence =
        core_sync::divergence(root, &remote_ref).map_err(|error| refused(TOOL, error))?;

    serde_json::to_string_pretty(&json!({
        "tool": TOOL,
        "remote": remote,
        "branch": branch,
        "remote_ref": remote_ref,
        "head": divergence.head,
        "remote_head": divergence.remote_head,
        "head_to_remote_empty": divergence.remote_ahead_count == 0,
        "remote_ahead_count": divergence.remote_ahead_count,
        "local_ahead_count": divergence.local_ahead_count,
        "remote_is_ancestor_of_head": divergence.remote_is_ancestor_of_head,
        "head_is_ancestor_of_remote": divergence.head_is_ancestor_of_remote,
        "source_status_unchanged": true,
        "status_short": source_status_after
    }))
    .map_err(|error| format!("git_remote_check refused: {error}"))
}

pub(crate) fn call_git_branch_prepare(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "reset branch from remote base";
    const TOOL: &str = tools::git_branch_prepare::NAME;

    let remote = validate_git_remote(
        optional_string(arguments, "remote")?.unwrap_or(core_sync::DEFAULT_REMOTE),
    )?;
    let base_branch = validate_git_branch(required_string(arguments, "base_branch")?)?;
    let branch = validate_git_branch(required_string(arguments, "branch")?)?;
    let required_files = optional_string_array(arguments, "required_files")?;
    let reset_existing = optional_bool(arguments, "reset_existing")?.unwrap_or(false);
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    if reset_existing && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_branch_prepare refused: reset_existing=true requires confirm: {CONFIRMATION:?}"
        ));
    }
    for required_file in &required_files {
        core_sync::required_file_path_syntax(required_file)
            .map_err(|error| refused(TOOL, error))?;
    }

    let root = repo_root_for_policy(repo_root, TOOL, WorktreeRootPolicy::ExactWorktreeRoot)?;
    ensure_remote_exists(TOOL, &root, &remote)?;
    let status_before = git_status_short(&root)?;
    core_sync::ensure_worktree_clean(
        &status_before,
        "worktree must be clean before branch preparation",
    )
    .map_err(|error| refused(TOOL, error))?;

    let remote_ref = core_sync::remote_tracking_ref(&remote, &base_branch);
    let fetch_args = vec![
        "fetch".to_string(),
        remote.clone(),
        core_sync::fetch_refspec(&remote, &base_branch),
    ];

    if dry_run {
        let branch_existed = local_branch_exists(&root, &branch)?;
        let previous_branch = current_branch(TOOL, &root)?;
        let action = core_sync::select_branch_action(branch_existed, reset_existing);
        let commands = std::iter::once(fetch_args.clone())
            .chain(action.commands(&branch, &remote_ref, previous_branch == branch))
            .map(|args| {
                json!({
                    "program": "git",
                    "args": args
                })
            })
            .collect::<Vec<_>>();

        return serde_json::to_string_pretty(&json!({
            "tool": TOOL,
            "dry_run": true,
            "prepared": false,
            "would_prepare": true,
            "action": action.planned_name(),
            "remote": remote,
            "base_branch": base_branch,
            "remote_ref": remote_ref,
            "branch": branch,
            "previous_branch": previous_branch,
            "branch_existed": branch_existed,
            "reset_existing": reset_existing,
            "required_files": required_files,
            "commands": commands,
            "post_fetch_guards_deferred_until_execution": [
                "resolve the fetched remote commit",
                "verify required files against the selected target ref",
                "verify the fetched remote base is an ancestor of the prepared branch",
                "verify the prepared branch, HEAD, required files, and worktree status"
            ],
            "required_confirm_for_reset": CONFIRMATION,
            "status_short": status_before
        }))
        .map_err(|error| format!("git_branch_prepare refused: {error}"));
    }

    core_sync::fetch(&root, fetch_args).map_err(|error| refused(TOOL, error))?;

    let status_after_fetch = git_status_short(&root)?;
    core_sync::ensure_status_unchanged(&status_before, &status_after_fetch)
        .map_err(|error| refused(TOOL, error))?;

    let remote_commit = resolve_commit(TOOL, &root, &remote_ref)?;
    let branch_ref = format!("refs/heads/{branch}");
    let branch_existed = local_branch_exists(&root, &branch)?;
    // Required files are verified against whichever ref will actually be landed on, before switching, so
    // a preparation that would arrive somewhere incomplete refuses first.
    let required_file_ref = if branch_existed && !reset_existing {
        branch_ref.as_str()
    } else {
        remote_ref.as_str()
    };
    let verified_required_files =
        core_sync::required_files_in_ref(&root, required_file_ref, &required_files)
            .map_err(|error| refused(TOOL, error))?;
    let previous_branch = current_branch(TOOL, &root)?;
    let action = core_sync::select_branch_action(branch_existed, reset_existing);

    if action == contextpatch_core::git::sync::BranchAction::SwitchExisting {
        let remote_is_ancestor = core_sync::is_ancestor(&root, &remote_ref, &branch_ref)
            .map_err(|error| refused(TOOL, error))?;
        if !remote_is_ancestor {
            return Err(format!(
                "git_branch_prepare refused: existing branch `{branch}` is not based on `{remote_ref}`; use reset_existing=true with confirm {CONFIRMATION:?} to recreate it from the remote base"
            ));
        }
    }
    for command in action.commands(&branch, &remote_ref, previous_branch == branch) {
        git_success_for_tool(TOOL, &root, command)?;
    }

    let current_branch = current_branch(TOOL, &root)?;
    if current_branch != branch {
        return Err(format!(
            "git_branch_prepare refused: current branch `{current_branch}` does not match prepared branch `{branch}`"
        ));
    }
    let head = resolve_commit(TOOL, &root, "HEAD")?;
    let remote_is_ancestor =
        core_sync::is_ancestor(&root, &remote_ref, "HEAD").map_err(|error| refused(TOOL, error))?;
    if !remote_is_ancestor {
        return Err(format!(
            "git_branch_prepare refused: `{remote_ref}` is not an ancestor of prepared branch `{branch}`"
        ));
    }

    for required_file in &verified_required_files {
        core_sync::required_file_present(&root, required_file)
            .map_err(|error| refused(TOOL, error))?;
    }
    let status_after = git_status_short(&root)?;

    serde_json::to_string_pretty(&json!({
        "tool": TOOL,
        "dry_run": false,
        "prepared": true,
        "action": action.completed_name(),
        "remote": remote,
        "base_branch": base_branch,
        "remote_ref": remote_ref,
        "remote_commit": remote_commit,
        "branch": branch,
        "previous_branch": previous_branch,
        "current_branch": current_branch,
        "head": head,
        "branch_existed": branch_existed,
        "reset_existing": reset_existing,
        "remote_base_is_ancestor": true,
        "required_files": verified_required_files,
        "required_files_present": true,
        "status_short": status_after
    }))
    .map_err(|error| format!("git_branch_prepare refused: {error}"))
}

pub(crate) fn call_git_merge_readiness(
    repository: contextpatch_core::git::GitRepository<'_>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const TOOL: &str = tools::git_merge_readiness::NAME;

    let base_ref = validate_git_ref_expression(required_string(arguments, "base_ref")?)?;
    let target_ref = validate_git_ref_expression(required_string(arguments, "target_ref")?)?;
    let fetch = optional_bool(arguments, "fetch")?.unwrap_or(false);
    let policy = repository_under_policy(repository, TOOL, WorktreeRootPolicy::ResolvedPath)?;
    let root = policy.bind(repository);
    let source_status_before = git_status_short(root)?;

    let mut fetched_remote = None;
    let mut fetched_branch = None;
    if fetch {
        let remote = validate_git_remote(
            optional_string(arguments, "remote")?.unwrap_or(core_sync::DEFAULT_REMOTE),
        )?;
        ensure_remote_exists(TOOL, root, &remote)?;
        let branch = match optional_string(arguments, "target_branch")? {
            Some(branch) => validate_git_branch(branch)?,
            None => core_sync::infer_fetch_branch(&remote, &target_ref)
                .map_err(|error| refused(TOOL, error))?,
        };

        core_sync::fetch(
            root,
            vec![
                "fetch".to_string(),
                remote.clone(),
                core_sync::fetch_refspec(&remote, &branch),
            ],
        )
        .map_err(|error| refused(TOOL, error))?;

        let source_status_after_fetch = git_status_short(root)?;
        core_sync::ensure_status_unchanged(&source_status_before, &source_status_after_fetch)
            .map_err(|error| refused(TOOL, error))?;

        fetched_remote = Some(remote);
        fetched_branch = Some(branch);
    } else if arguments.contains_key("target_branch") {
        return Err(
            "git_merge_readiness refused: target_branch is only valid when fetch is true"
                .to_string(),
        );
    } else if let Some(remote) = optional_string(arguments, "remote")? {
        validate_git_remote(remote)?;
    }

    let base_commit = resolve_commit(TOOL, root, &base_ref)?;
    let target_commit = resolve_commit(TOOL, root, &target_ref)?;
    let merge_base = git_stdout_for_tool(TOOL, root, &["merge-base", &base_commit, &target_commit])?;
    let merge_base = merge_base.trim().to_string();

    let base_ahead_count = rev_count_for_tool(TOOL, root, &format!("{merge_base}..{base_commit}"))?;
    let target_ahead_count =
        rev_count_for_tool(TOOL, root, &format!("{merge_base}..{target_commit}"))?;
    let base_changed_files = core_sync::changed_files_between(root, &merge_base, &base_commit)
        .map_err(|error| refused(TOOL, error))?;
    let target_changed_files = core_sync::changed_files_between(root, &merge_base, &target_commit)
        .map_err(|error| refused(TOOL, error))?;
    let changed_on_both_sides: BTreeSet<String> = base_changed_files
        .intersection(&target_changed_files)
        .cloned()
        .collect();
    let source_status_after = git_status_short(root)?;

    serde_json::to_string_pretty(&json!({
        "tool": TOOL,
        "read_only": true,
        "fetch_performed": fetch,
        "fetched_remote": fetched_remote,
        "fetched_branch": fetched_branch,
        "base_ref": base_ref,
        "target_ref": target_ref,
        "base_commit": base_commit,
        "target_commit": target_commit,
        "merge_base": merge_base,
        "base_is_ancestor_of_target": base_ahead_count == 0,
        "target_is_ancestor_of_base": target_ahead_count == 0,
        "base_ahead_count": base_ahead_count,
        "target_ahead_count": target_ahead_count,
        "base_changed_files_count": base_changed_files.len(),
        "target_changed_files_count": target_changed_files.len(),
        "changed_on_both_sides_count": changed_on_both_sides.len(),
        "changed_on_both_sides": changed_on_both_sides,
        "likely_conflict_candidates": changed_on_both_sides,
        "has_likely_conflict_candidates": !changed_on_both_sides.is_empty(),
        "source_status_unchanged": source_status_after == source_status_before,
        "status_short": source_status_after
    }))
    .map_err(|error| format!("git_merge_readiness refused: {error}"))
}

pub(crate) fn call_git_push_exact(
    repository: contextpatch_core::git::GitRepository<'_>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "push exact commit";
    const TOOL: &str = tools::git_push_exact::NAME;

    let remote = validate_git_remote(required_string(arguments, "remote")?)?;
    let branch = validate_git_branch(required_string(arguments, "branch")?)?;
    let expected_head = core_sync::expected_head(required_string(arguments, "expected_head")?)
        .map_err(|error| refused(TOOL, error))?;
    let confirm = required_string(arguments, "confirm")?;
    if confirm != CONFIRMATION {
        return Err(format!(
            "git_push_exact refused: confirm must be {CONFIRMATION:?}"
        ));
    }

    let policy = repository_under_policy(repository, TOOL, WorktreeRootPolicy::ExactWorktreeRoot)?;
    let root = policy.bind(repository);
    ensure_remote_exists(TOOL, root, &remote)?;
    let status = git_status_short(root)?;
    core_sync::ensure_worktree_clean(&status, "worktree must be clean before push")
        .map_err(|error| refused(TOOL, error))?;

    let current_branch = git_stdout_for_tool(TOOL, root, &["branch", "--show-current"])?;
    let current_branch = current_branch.trim();
    if current_branch != branch {
        return Err(format!(
            "git_push_exact refused: current branch `{current_branch}` does not match requested branch `{branch}`"
        ));
    }

    let head = git_stdout_for_tool(TOOL, root, &["rev-parse", "HEAD"])?;
    let head = head.trim().to_string();
    if !head.starts_with(&expected_head) {
        return Err(format!(
            "git_push_exact refused: HEAD `{head}` does not match expected_head `{expected_head}`"
        ));
    }

    core_sync::fetch(
        root,
        vec!["fetch".to_string(), remote.clone(), branch.clone()],
    )
    .map_err(|error| refused(TOOL, error))?;
    let status_after_fetch = git_status_short(root)?;
    core_sync::ensure_worktree_clean(&status_after_fetch, "worktree changed during fetch")
        .map_err(|error| refused(TOOL, error))?;

    let remote_ref = core_sync::remote_tracking_ref(&remote, &branch);
    let divergence =
        core_sync::divergence(root, &remote_ref).map_err(|error| refused(TOOL, error))?;
    if divergence.remote_ahead_count != 0 {
        return Err(format!(
            "git_push_exact refused: remote `{remote_ref}` is ahead of HEAD by {} commit(s)",
            divergence.remote_ahead_count
        ));
    }
    if !divergence.remote_is_ancestor_of_head {
        return Err(format!(
            "git_push_exact refused: remote `{remote_ref}` is not an ancestor of HEAD; refusing non-fast-forward/divergent push"
        ));
    }

    git_success_for_tool(
        TOOL,
        root,
        vec![
            "push".to_string(),
            remote.clone(),
            format!("HEAD:refs/heads/{branch}"),
        ],
    )?;

    serde_json::to_string_pretty(&json!({
        "tool": TOOL,
        "pushed": true,
        "remote": remote,
        "branch": branch,
        "commit": head,
        "previous_remote_head": divergence.remote_head,
        "force": false,
        "refspec": format!("HEAD:refs/heads/{branch}"),
        "status_short": git_status_short(root)?
    }))
    .map_err(|error| format!("git_push_exact refused: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn moved_sync_policy_keeps_its_refusal_prefixes() {
        // Policy now lives in core, which returns the detail only. These reach core-originated refusals
        // without needing a repository, pinning the seam for the argument-shaped ones.
        let root = std::env::temp_dir();

        assert_eq!(
            call_git_push_exact(
                (&root).into(),
                &arguments(json!({
                    "remote": "origin",
                    "branch": "main",
                    "expected_head": "abc",
                    "confirm": "push exact commit"
                }))
            )
            .unwrap_err(),
            "git_push_exact refused: expected_head must be a 7 to 40 character commit hash"
        );
        assert_eq!(
            call_git_push_exact(
                (&root).into(),
                &arguments(json!({
                    "remote": "origin",
                    "branch": "main",
                    "expected_head": "zzzzzzz",
                    "confirm": "push exact commit"
                }))
            )
            .unwrap_err(),
            "git_push_exact refused: expected_head must be hexadecimal"
        );
        assert_eq!(
            call_git_branch_prepare(
                &root,
                &arguments(json!({
                    "base_branch": "main",
                    "branch": "feature",
                    "required_files": ["main:src/lib.rs"]
                }))
            )
            .unwrap_err(),
            "git_branch_prepare refused: required file path `main:src/lib.rs` contains Git pathspec metacharacters"
        );
    }

    #[test]
    fn confirmations_are_still_checked_before_any_repository_work() {
        // These stay in the adapter, ahead of the root policy and every Git call.
        let root = std::env::temp_dir();

        assert_eq!(
            call_git_push_exact(
                (&root).into(),
                &arguments(json!({
                    "remote": "origin",
                    "branch": "main",
                    "expected_head": "abcdef1",
                    "confirm": "wrong"
                }))
            )
            .unwrap_err(),
            "git_push_exact refused: confirm must be \"push exact commit\""
        );
        assert_eq!(
            call_git_branch_prepare(
                &root,
                &arguments(json!({
                    "base_branch": "main",
                    "branch": "feature",
                    "reset_existing": true
                }))
            )
            .unwrap_err(),
            "git_branch_prepare refused: reset_existing=true requires confirm: \"reset branch from remote base\""
        );

        // `git_merge_readiness` is deliberately not asserted here: its `target_branch` refusal comes
        // after the first status read, so outside a repository the status failure surfaces instead.
        // That ordering is pre-existing and unchanged.
    }
}
