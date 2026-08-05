use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::*;
use crate::tools::git::support::*;

pub(crate) fn call_git_remote_list(repo_root: &Path) -> Result<String, String> {
    let root = repo_root_for_policy(
        repo_root,
        tools::git_remote_list::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
    let output = git_stdout_for_tool(tools::git_remote_list::NAME, &root, &["remote", "-v"])?;
    let mut remotes = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let name = fields
            .next()
            .ok_or_else(|| "git_remote_list refused: malformed remote output".to_string())?;
        let url = fields
            .next()
            .ok_or_else(|| "git_remote_list refused: malformed remote output".to_string())?;
        let kind = fields.next().unwrap_or("").trim_matches(['(', ')']);
        remotes.push(json!({
            "name": name,
            "url": url,
            "kind": kind
        }));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_remote_list::NAME,
        "remotes": remotes
    }))
    .map_err(|error| format!("git_remote_list refused: {error}"))
}

pub(crate) fn call_git_remote_check(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let remote = validate_git_remote(optional_string(arguments, "remote")?.unwrap_or("origin"))?;
    let branch = validate_git_branch(required_string(arguments, "branch")?)?;
    let root = repo_root_for_policy(
        repo_root,
        tools::git_remote_check::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
    ensure_remote_exists(tools::git_remote_check::NAME, &root, &remote)?;
    let source_status_before = git_status_short(&root)?;

    git_success_for_tool(
        tools::git_remote_check::NAME,
        &root,
        vec!["fetch".to_string(), remote.clone(), branch.clone()],
    )?;

    let source_status_after = git_status_short(&root)?;
    if source_status_after != source_status_before {
        return Err(format!(
            "git_remote_check refused: source worktree changed during fetch\nbefore:\n{}\nafter:\n{}",
            empty_label(&source_status_before),
            empty_label(&source_status_after)
        ));
    }

    let head = git_stdout_for_tool(tools::git_remote_check::NAME, &root, &["rev-parse", "HEAD"])?;
    let remote_ref = remote_ref(&remote, &branch);
    let remote_head = git_stdout_for_tool(
        tools::git_remote_check::NAME,
        &root,
        &["rev-parse", &remote_ref],
    )?;
    let remote_ahead_count = rev_count_for_tool(
        tools::git_remote_check::NAME,
        &root,
        &format!("HEAD..{remote_ref}"),
    )?;
    let local_ahead_count = rev_count_for_tool(
        tools::git_remote_check::NAME,
        &root,
        &format!("{remote_ref}..HEAD"),
    )?;
    let remote_is_ancestor =
        git_status_code(&root, &["merge-base", "--is-ancestor", &remote_ref, "HEAD"])? == 0;
    let head_is_ancestor =
        git_status_code(&root, &["merge-base", "--is-ancestor", "HEAD", &remote_ref])? == 0;

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_remote_check::NAME,
        "remote": remote,
        "branch": branch,
        "remote_ref": remote_ref,
        "head": head.trim(),
        "remote_head": remote_head.trim(),
        "head_to_remote_empty": remote_ahead_count == 0,
        "remote_ahead_count": remote_ahead_count,
        "local_ahead_count": local_ahead_count,
        "remote_is_ancestor_of_head": remote_is_ancestor,
        "head_is_ancestor_of_remote": head_is_ancestor,
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

    let remote = validate_git_remote(optional_string(arguments, "remote")?.unwrap_or("origin"))?;
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
        validate_required_file_path_syntax(tools::git_branch_prepare::NAME, required_file)?;
    }

    let root = repo_root_for_policy(
        repo_root,
        tools::git_branch_prepare::NAME,
        WorktreeRootPolicy::ExactWorktreeRoot,
    )?;
    ensure_remote_exists(tools::git_branch_prepare::NAME, &root, &remote)?;
    let status_before = git_status_short(&root)?;
    if !status_before.trim().is_empty() {
        return Err(format!(
            "git_branch_prepare refused: worktree must be clean before branch preparation\n{}",
            status_before.trim()
        ));
    }

    let remote_ref = remote_ref(&remote, &base_branch);
    let fetch_args = vec![
        "fetch".to_string(),
        remote.clone(),
        format!("refs/heads/{base_branch}:{remote_ref}"),
    ];
    if dry_run {
        let branch_existed = local_branch_exists(&root, &branch)?;
        let previous_branch = current_branch(tools::git_branch_prepare::NAME, &root)?;
        let (action, branch_commands) = if branch_existed {
            if reset_existing {
                if previous_branch == branch {
                    (
                        "reset_existing_branch",
                        vec![vec![
                            "reset".to_string(),
                            "--hard".to_string(),
                            remote_ref.clone(),
                        ]],
                    )
                } else {
                    (
                        "reset_existing_branch",
                        vec![
                            vec![
                                "branch".to_string(),
                                "-f".to_string(),
                                branch.clone(),
                                remote_ref.clone(),
                            ],
                            vec!["switch".to_string(), branch.clone()],
                        ],
                    )
                }
            } else {
                (
                    "switch_existing_branch",
                    vec![vec!["switch".to_string(), branch.clone()]],
                )
            }
        } else {
            (
                "create_branch",
                vec![vec![
                    "switch".to_string(),
                    "-c".to_string(),
                    branch.clone(),
                    remote_ref.clone(),
                ]],
            )
        };
        let commands = std::iter::once(fetch_args.clone())
            .chain(branch_commands)
            .map(|args| {
                json!({
                    "program": "git",
                    "args": args
                })
            })
            .collect::<Vec<_>>();

        return serde_json::to_string_pretty(&json!({
            "tool": tools::git_branch_prepare::NAME,
            "dry_run": true,
            "prepared": false,
            "would_prepare": true,
            "action": action,
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

    git_success_for_tool(tools::git_branch_prepare::NAME, &root, fetch_args)?;

    let status_after_fetch = git_status_short(&root)?;
    if status_after_fetch != status_before {
        return Err(format!(
            "git_branch_prepare refused: source worktree changed during fetch\nbefore:\n{}\nafter:\n{}",
            empty_label(&status_before),
            empty_label(&status_after_fetch)
        ));
    }

    let remote_commit = resolve_commit(tools::git_branch_prepare::NAME, &root, &remote_ref)?;
    let branch_ref = format!("refs/heads/{branch}");
    let branch_existed = local_branch_exists(&root, &branch)?;
    let required_file_ref = if branch_existed && !reset_existing {
        branch_ref.as_str()
    } else {
        remote_ref.as_str()
    };
    let verified_required_files = validate_required_files_in_ref(
        tools::git_branch_prepare::NAME,
        &root,
        required_file_ref,
        &required_files,
    )?;
    let previous_branch = current_branch(tools::git_branch_prepare::NAME, &root)?;
    let action = if branch_existed {
        if reset_existing {
            if previous_branch == branch {
                git_success_for_tool(
                    tools::git_branch_prepare::NAME,
                    &root,
                    vec![
                        "reset".to_string(),
                        "--hard".to_string(),
                        remote_ref.clone(),
                    ],
                )?;
            } else {
                git_success_for_tool(
                    tools::git_branch_prepare::NAME,
                    &root,
                    vec![
                        "branch".to_string(),
                        "-f".to_string(),
                        branch.clone(),
                        remote_ref.clone(),
                    ],
                )?;
                git_success_for_tool(
                    tools::git_branch_prepare::NAME,
                    &root,
                    vec!["switch".to_string(), branch.clone()],
                )?;
            }
            "reset_existing_branch"
        } else {
            let remote_is_ancestor = git_status_code(
                &root,
                &["merge-base", "--is-ancestor", &remote_ref, &branch_ref],
            )? == 0;
            if !remote_is_ancestor {
                return Err(format!(
                    "git_branch_prepare refused: existing branch `{branch}` is not based on `{remote_ref}`; use reset_existing=true with confirm {CONFIRMATION:?} to recreate it from the remote base"
                ));
            }
            git_success_for_tool(
                tools::git_branch_prepare::NAME,
                &root,
                vec!["switch".to_string(), branch.clone()],
            )?;
            "switched_existing_branch"
        }
    } else {
        git_success_for_tool(
            tools::git_branch_prepare::NAME,
            &root,
            vec![
                "switch".to_string(),
                "-c".to_string(),
                branch.clone(),
                remote_ref.clone(),
            ],
        )?;
        "created_branch"
    };

    let current_branch = current_branch(tools::git_branch_prepare::NAME, &root)?;
    if current_branch != branch {
        return Err(format!(
            "git_branch_prepare refused: current branch `{current_branch}` does not match prepared branch `{branch}`"
        ));
    }
    let head = resolve_commit(tools::git_branch_prepare::NAME, &root, "HEAD")?;
    let remote_is_ancestor =
        git_status_code(&root, &["merge-base", "--is-ancestor", &remote_ref, "HEAD"])? == 0;
    if !remote_is_ancestor {
        return Err(format!(
            "git_branch_prepare refused: `{remote_ref}` is not an ancestor of prepared branch `{branch}`"
        ));
    }

    for required_file in &verified_required_files {
        validate_required_file_path(tools::git_branch_prepare::NAME, &root, required_file)?;
    }
    let status_after = git_status_short(&root)?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_branch_prepare::NAME,
        "dry_run": false,
        "prepared": true,
        "action": action,
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
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let base_ref = validate_git_ref_expression(required_string(arguments, "base_ref")?)?;
    let target_ref = validate_git_ref_expression(required_string(arguments, "target_ref")?)?;
    let fetch = optional_bool(arguments, "fetch")?.unwrap_or(false);
    let root = repo_root_for_policy(
        repo_root,
        tools::git_merge_readiness::NAME,
        WorktreeRootPolicy::ResolvedPath,
    )?;
    let source_status_before = git_status_short(&root)?;

    let mut fetched_remote = None;
    let mut fetched_branch = None;
    if fetch {
        let remote =
            validate_git_remote(optional_string(arguments, "remote")?.unwrap_or("origin"))?;
        ensure_remote_exists(tools::git_merge_readiness::NAME, &root, &remote)?;
        let branch = match optional_string(arguments, "target_branch")? {
            Some(branch) => validate_git_branch(branch)?,
            None => infer_fetch_branch(&remote, &target_ref)?,
        };

        git_success_for_tool(
            tools::git_merge_readiness::NAME,
            &root,
            vec![
                "fetch".to_string(),
                remote.clone(),
                format!("refs/heads/{branch}:refs/remotes/{remote}/{branch}"),
            ],
        )?;

        let source_status_after_fetch = git_status_short(&root)?;
        if source_status_after_fetch != source_status_before {
            return Err(format!(
                "git_merge_readiness refused: source worktree changed during fetch\nbefore:\n{}\nafter:\n{}",
                empty_label(&source_status_before),
                empty_label(&source_status_after_fetch)
            ));
        }

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

    let base_commit = resolve_commit(tools::git_merge_readiness::NAME, &root, &base_ref)?;
    let target_commit = resolve_commit(tools::git_merge_readiness::NAME, &root, &target_ref)?;
    let merge_base = git_stdout_for_tool(
        tools::git_merge_readiness::NAME,
        &root,
        &["merge-base", &base_commit, &target_commit],
    )?;
    let merge_base = merge_base.trim().to_string();

    let base_ahead_count = rev_count_for_tool(
        tools::git_merge_readiness::NAME,
        &root,
        &format!("{merge_base}..{base_commit}"),
    )?;
    let target_ahead_count = rev_count_for_tool(
        tools::git_merge_readiness::NAME,
        &root,
        &format!("{merge_base}..{target_commit}"),
    )?;
    let base_changed_files = changed_files_between(
        tools::git_merge_readiness::NAME,
        &root,
        &merge_base,
        &base_commit,
    )?;
    let target_changed_files = changed_files_between(
        tools::git_merge_readiness::NAME,
        &root,
        &merge_base,
        &target_commit,
    )?;
    let changed_on_both_sides: BTreeSet<String> = base_changed_files
        .intersection(&target_changed_files)
        .cloned()
        .collect();
    let source_status_after = git_status_short(&root)?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_merge_readiness::NAME,
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
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "push exact commit";

    let remote = validate_git_remote(required_string(arguments, "remote")?)?;
    let branch = validate_git_branch(required_string(arguments, "branch")?)?;
    let expected_head = validate_expected_head(required_string(arguments, "expected_head")?)?;
    let confirm = required_string(arguments, "confirm")?;
    if confirm != CONFIRMATION {
        return Err(format!(
            "git_push_exact refused: confirm must be {CONFIRMATION:?}"
        ));
    }

    let root = repo_root_for_policy(
        repo_root,
        tools::git_push_exact::NAME,
        WorktreeRootPolicy::ExactWorktreeRoot,
    )?;
    ensure_remote_exists(tools::git_push_exact::NAME, &root, &remote)?;
    let status = git_status_short(&root)?;
    if !status.trim().is_empty() {
        return Err(format!(
            "git_push_exact refused: worktree must be clean before push\n{}",
            status.trim()
        ));
    }

    let current_branch = git_stdout_for_tool(
        tools::git_push_exact::NAME,
        &root,
        &["branch", "--show-current"],
    )?;
    let current_branch = current_branch.trim();
    if current_branch != branch {
        return Err(format!(
            "git_push_exact refused: current branch `{current_branch}` does not match requested branch `{branch}`"
        ));
    }

    let head = git_stdout_for_tool(tools::git_push_exact::NAME, &root, &["rev-parse", "HEAD"])?;
    let head = head.trim().to_string();
    if !head.starts_with(&expected_head) {
        return Err(format!(
            "git_push_exact refused: HEAD `{head}` does not match expected_head `{expected_head}`"
        ));
    }

    git_success_for_tool(
        tools::git_push_exact::NAME,
        &root,
        vec!["fetch".to_string(), remote.clone(), branch.clone()],
    )?;
    let status_after_fetch = git_status_short(&root)?;
    if !status_after_fetch.trim().is_empty() {
        return Err(format!(
            "git_push_exact refused: worktree changed during fetch\n{}",
            status_after_fetch.trim()
        ));
    }

    let remote_ref = remote_ref(&remote, &branch);
    let remote_head = git_stdout_for_tool(
        tools::git_push_exact::NAME,
        &root,
        &["rev-parse", &remote_ref],
    )?;
    let remote_head = remote_head.trim().to_string();
    let remote_ahead_count = rev_count_for_tool(
        tools::git_push_exact::NAME,
        &root,
        &format!("HEAD..{remote_ref}"),
    )?;
    if remote_ahead_count != 0 {
        return Err(format!(
            "git_push_exact refused: remote `{remote_ref}` is ahead of HEAD by {remote_ahead_count} commit(s)"
        ));
    }
    let remote_is_ancestor =
        git_status_code(&root, &["merge-base", "--is-ancestor", &remote_ref, "HEAD"])? == 0;
    if !remote_is_ancestor {
        return Err(format!(
            "git_push_exact refused: remote `{remote_ref}` is not an ancestor of HEAD; refusing non-fast-forward/divergent push"
        ));
    }

    git_success_for_tool(
        tools::git_push_exact::NAME,
        &root,
        vec![
            "push".to_string(),
            remote.clone(),
            format!("HEAD:refs/heads/{branch}"),
        ],
    )?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_push_exact::NAME,
        "pushed": true,
        "remote": remote,
        "branch": branch,
        "commit": head,
        "previous_remote_head": remote_head,
        "force": false,
        "refspec": format!("HEAD:refs/heads/{branch}"),
        "status_short": git_status_short(&root)?
    }))
    .map_err(|error| format!("git_push_exact refused: {error}"))
}
