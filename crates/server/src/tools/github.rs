pub mod github_fork_prepare {
    pub const NAME: &str = "github_fork_prepare";
}

pub mod github_pr_run {
    pub const NAME: &str = "github_pr_run";
}

use std::process::{Command, Stdio};

use contextpatch_core::git::RepositoryRoot;
use contextpatch_core::process::guarded_command::{
    redact_and_truncate_output, redact_and_truncate_output_tail,
};
use contextpatch_core::process::runner::resolve_child_cwd;
use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::{
    nonempty_tool_string, optional_bool, optional_string, optional_u64, required_string,
};
use crate::tools::git::support::git_stdout_for_tool;

pub(crate) fn call_github_pr_run<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CREATE_CONFIRMATION: &str = "create pull request";
    const RERUN_CONFIRMATION: &str = "rerun failed workflow jobs";

    let action = required_string(arguments, "action")?;
    // `gh` needs a working directory to discover which repository it is acting on. It gets the retained
    // descriptor rather than a name, so the repository it discovers is the one that was selected. The
    // logical path is still reported, because that is what a caller reading `cwd` expects to see.
    let root = repository_root.into();
    let cwd = resolve_child_cwd(root, None)
        .map_err(|error| format!("github_pr_run refused: {error}"))?;
    let root = cwd.logical_path();
    let repository = optional_github_repository(arguments)?;
    let job_log_view = if action == "workflow_job_log" {
        Some(workflow_job_log_view(arguments)?)
    } else {
        if arguments.contains_key("log_view") {
            return Err(
                "github_pr_run refused: log_view is accepted only for workflow_job_log".to_string(),
            );
        }
        None
    };
    let args: Vec<String> = match action {
        "auth_status" => {
            if repository.is_some() {
                return Err(
                    "github_pr_run refused: repository is not accepted for auth_status".to_string(),
                );
            }
            vec!["auth".to_string(), "status".to_string()]
        }
        "pr_view" => {
            let number = optional_u64(arguments, "number")?.ok_or_else(|| {
                "github_pr_run refused: number is required for pr_view".to_string()
            })?;
            let mut args = vec![
                "pr".to_string(),
                "view".to_string(),
                number.to_string(),
                "--json".to_string(),
                "number,title,state,url,author,headRefName,headRefOid,baseRefName,comments,reviews,statusCheckRollup".to_string(),
            ];
            append_repository(&mut args, repository.as_deref());
            args
        }
        "pr_comments" => {
            let number = optional_u64(arguments, "number")?.ok_or_else(|| {
                "github_pr_run refused: number is required for pr_comments".to_string()
            })?;
            optional_comment_contains(arguments)?;
            result_limit(arguments, "pr_comments", 20, 100)?;
            let mut args = vec![
                "pr".to_string(),
                "view".to_string(),
                number.to_string(),
                "--json".to_string(),
                "comments".to_string(),
            ];
            append_repository(&mut args, repository.as_deref());
            args
        }
        "pr_checks" => {
            let number = optional_u64(arguments, "number")?.ok_or_else(|| {
                "github_pr_run refused: number is required for pr_checks".to_string()
            })?;
            let mut args = vec![
                "pr".to_string(),
                "checks".to_string(),
                number.to_string(),
                "--json".to_string(),
                "bucket,completedAt,description,event,link,name,startedAt,state,workflow"
                    .to_string(),
            ];
            append_repository(&mut args, repository.as_deref());
            args
        }
        "workflow_runs_for_commit" => {
            let head_sha = required_github_head_sha(arguments)?;
            let limit = result_limit(arguments, "workflow_runs_for_commit", 20, 50)?;
            let mut args = vec![
                "run".to_string(),
                "list".to_string(),
                "--commit".to_string(),
                head_sha,
                "--limit".to_string(),
                limit.to_string(),
                "--json".to_string(),
                "attempt,conclusion,createdAt,databaseId,displayTitle,event,headBranch,headSha,name,number,startedAt,status,updatedAt,url,workflowDatabaseId,workflowName".to_string(),
            ];
            append_repository(&mut args, repository.as_deref());
            args
        }
        "workflow_run_view" => {
            let run_id = optional_u64(arguments, "run_id")?.ok_or_else(|| {
                "github_pr_run refused: run_id is required for workflow_run_view".to_string()
            })?;
            let mut args = vec![
                "run".to_string(),
                "view".to_string(),
                run_id.to_string(),
                "--json".to_string(),
                "attempt,conclusion,createdAt,databaseId,displayTitle,event,headBranch,headSha,jobs,name,number,startedAt,status,updatedAt,url,workflowDatabaseId,workflowName".to_string(),
            ];
            append_repository(&mut args, repository.as_deref());
            args
        }
        "workflow_job_log" => {
            let job_id = optional_u64(arguments, "job_id")?.ok_or_else(|| {
                "github_pr_run refused: job_id is required for workflow_job_log".to_string()
            })?;
            let mut args = vec![
                "run".to_string(),
                "view".to_string(),
                "--job".to_string(),
                job_id.to_string(),
                "--log".to_string(),
            ];
            append_repository(&mut args, repository.as_deref());
            args
        }
        "workflow_run_rerun_failed" => {
            let run_id = optional_u64(arguments, "run_id")?.ok_or_else(|| {
                "github_pr_run refused: run_id is required for workflow_run_rerun_failed"
                    .to_string()
            })?;
            let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
            let mut args = vec![
                "run".to_string(),
                "rerun".to_string(),
                run_id.to_string(),
                "--failed".to_string(),
            ];
            append_repository(&mut args, repository.as_deref());
            if dry_run {
                return serde_json::to_string_pretty(&json!({
                    "tool": tools::github_pr_run::NAME,
                    "action": "workflow_run_rerun_failed",
                    "dry_run": true,
                    "program": "gh",
                    "args": args,
                    "cwd": root.display().to_string(),
                    "required_confirm_for_rerun": RERUN_CONFIRMATION
                }))
                .map_err(|error| format!("github_pr_run refused: {error}"));
            }
            let confirm = required_string(arguments, "confirm")?;
            if confirm != RERUN_CONFIRMATION {
                return Err(format!(
                    "github_pr_run refused: confirm must be {RERUN_CONFIRMATION:?}"
                ));
            }
            args
        }
        "pr_create" => {
            let base = nonempty_tool_string(
                tools::github_pr_run::NAME,
                "base",
                required_string(arguments, "base")?,
            )?;
            let head = nonempty_tool_string(
                tools::github_pr_run::NAME,
                "head",
                required_string(arguments, "head")?,
            )?;
            let title = nonempty_tool_string(
                tools::github_pr_run::NAME,
                "title",
                required_string(arguments, "title")?,
            )?;
            let body = required_string(arguments, "body")?.to_string();
            let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
            let draft = optional_bool(arguments, "draft")?.unwrap_or(false);
            let mut args = vec![
                "pr".to_string(),
                "create".to_string(),
                "--base".to_string(),
                base,
                "--head".to_string(),
                head,
                "--title".to_string(),
                title,
                "--body".to_string(),
                body,
            ];
            if draft {
                args.push("--draft".to_string());
            }
            append_repository(&mut args, repository.as_deref());
            if dry_run {
                return serde_json::to_string_pretty(&json!({
                    "tool": tools::github_pr_run::NAME,
                    "action": "pr_create",
                    "dry_run": true,
                    "program": "gh",
                    "args": args,
                    "cwd": root.display().to_string()
                }))
                .map_err(|error| format!("github_pr_run refused: {error}"));
            }
            let confirm = required_string(arguments, "confirm")?;
            if confirm != CREATE_CONFIRMATION {
                return Err(format!(
                    "github_pr_run refused: confirm must be {CREATE_CONFIRMATION:?}"
                ));
            }
            args
        }
        _ => {
            return Err(format!(
                "github_pr_run refused: unsupported action `{action}`"
            ))
        }
    };

    let mut command = Command::new("gh");
    command.args(&args).stdin(Stdio::null());
    cwd.anchor_command(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("github_pr_run refused: failed to run gh: {error}"))?;

    let raw_stdout = String::from_utf8_lossy(&output.stdout);
    let stdout_source = if action == "pr_comments" && output.status.success() {
        filter_pr_comments(&raw_stdout, arguments)?
    } else {
        raw_stdout.into_owned()
    };
    let (stdout, stdout_truncated) = if job_log_view.as_deref() == Some("tail") {
        redact_and_truncate_output_tail(&stdout_source, 120_000)
    } else {
        redact_and_truncate_output(&stdout_source, 120_000)
    };
    let (stderr, stderr_truncated) = bounded_output(&output.stderr, 20_000);
    let exit_code = output.status.code();
    let mut response = json!({
        "tool": tools::github_pr_run::NAME,
        "action": action,
        "program": "gh",
        "args": args,
        "cwd": root.display().to_string(),
        "exit_code": exit_code,
        "success": output.status.success(),
        "stdout": stdout,
        "stderr": stderr,
        "stdout_truncated": stdout_truncated,
        "stderr_truncated": stderr_truncated
    });
    if let Some(log_view) = job_log_view {
        response["log_view"] = json!(log_view);
    }
    serde_json::to_string_pretty(&response)
        .map_err(|error| format!("github_pr_run refused: {error}"))
}

fn workflow_job_log_view(arguments: &serde_json::Map<String, Value>) -> Result<String, String> {
    let value = optional_string(arguments, "log_view")?.unwrap_or("tail");
    if matches!(value, "head" | "tail") {
        Ok(value.to_string())
    } else {
        Err("github_pr_run refused: log_view must be one of head, tail".to_string())
    }
}

fn required_github_head_sha(arguments: &serde_json::Map<String, Value>) -> Result<String, String> {
    let value = nonempty_tool_string(
        tools::github_pr_run::NAME,
        "head_sha",
        required_string(arguments, "head_sha")?,
    )?;
    if value.len() != 40 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(
            "github_pr_run refused: head_sha must be a full 40-character hexadecimal commit SHA"
                .to_string(),
        );
    }
    Ok(value.to_ascii_lowercase())
}

fn optional_comment_contains(
    arguments: &serde_json::Map<String, Value>,
) -> Result<Option<String>, String> {
    let Some(value) = optional_string(arguments, "comment_contains")? else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() || value.contains('\0') || value.chars().count() > 256 {
        return Err(
            "github_pr_run refused: comment_contains must contain 1 to 256 characters and no NUL"
                .to_string(),
        );
    }
    Ok(Some(value.to_string()))
}

fn result_limit(
    arguments: &serde_json::Map<String, Value>,
    action: &str,
    default: u64,
    maximum: u64,
) -> Result<usize, String> {
    let limit = optional_u64(arguments, "limit")?.unwrap_or(default);
    if limit == 0 || limit > maximum {
        return Err(format!(
            "github_pr_run refused: limit for {action} must be between 1 and {maximum}"
        ));
    }
    usize::try_from(limit)
        .map_err(|_| format!("github_pr_run refused: limit for {action} is out of range"))
}

fn filter_pr_comments(
    raw: &str,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|error| format!("github_pr_run refused: invalid gh comments JSON: {error}"))?;
    let comments = parsed
        .get("comments")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "github_pr_run refused: gh comments response did not contain a comments array"
                .to_string()
        })?;
    let contains = optional_comment_contains(arguments)?;
    let normalized_contains = contains.as_ref().map(|value| value.to_lowercase());
    let mut matching = comments
        .iter()
        .filter(|comment| {
            normalized_contains.as_ref().is_none_or(|needle| {
                comment
                    .get("body")
                    .and_then(Value::as_str)
                    .is_some_and(|body| body.to_lowercase().contains(needle))
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| comment_created_at(right).cmp(comment_created_at(left)));
    let matched_count = matching.len();
    matching.truncate(result_limit(arguments, "pr_comments", 20, 100)?);

    serde_json::to_string_pretty(&json!({
        "comment_contains": contains,
        "matched_count": matched_count,
        "returned_count": matching.len(),
        "order": "newest_first",
        "comments": matching
    }))
    .map_err(|error| format!("github_pr_run refused: {error}"))
}

fn comment_created_at(comment: &Value) -> &str {
    comment
        .get("createdAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn optional_github_repository(
    arguments: &serde_json::Map<String, Value>,
) -> Result<Option<String>, String> {
    optional_string(arguments, "repository")?
        .map(validate_github_repository)
        .transpose()
}

fn validate_github_repository(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    let Some((owner, repository)) = value.split_once('/') else {
        return Err("github_pr_run refused: repository must use OWNER/REPO format".to_string());
    };
    if repository.contains('/')
        || !valid_github_name(owner, 39, false)
        || !valid_github_name(repository, 100, true)
    {
        return Err(
            "github_pr_run refused: repository must use OWNER/REPO with only letters, digits, `.`, `_`, or `-`".to_string(),
        );
    }
    Ok(value.to_string())
}

fn valid_github_name(value: &str, max_len: usize, allow_dot_underscore: bool) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value != "."
        && value != ".."
        && value.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || ch == '-'
                || (allow_dot_underscore && matches!(ch, '.' | '_'))
        })
}

fn append_repository(args: &mut Vec<String>, repository: Option<&str>) {
    if let Some(repository) = repository {
        args.push("--repo".to_string());
        args.push(repository.to_string());
    }
}

fn bounded_output(bytes: &[u8], max_chars: usize) -> (String, bool) {
    let text = String::from_utf8_lossy(bytes);
    redact_and_truncate_output(&text, max_chars)
}

pub(crate) fn call_github_fork_prepare<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "prepare github fork";

    let repository_root = repository_root.into();
    let cwd = resolve_child_cwd(repository_root, None)
        .map_err(|error| format!("github_fork_prepare refused: {error}"))?;
    let root = cwd.logical_path();
    let remote = optional_bool(arguments, "remote")?.unwrap_or(true);
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);

    let mut args = vec!["repo".to_string(), "fork".to_string()];
    if remote {
        args.push("--remote".to_string());
    }

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::github_fork_prepare::NAME,
            "dry_run": true,
            "program": "gh",
            "args": args,
            "cwd": root.display().to_string(),
            "required_confirm_for_fork": CONFIRMATION
        }))
        .map_err(|error| format!("github_fork_prepare refused: {error}"));
    }

    let confirm = required_string(arguments, "confirm")?;
    if confirm != CONFIRMATION {
        return Err(format!(
            "github_fork_prepare refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let mut command = Command::new("gh");
    command.args(&args).stdin(Stdio::null());
    cwd.anchor_command(&mut command);
    let output = command
        .output()
        .map_err(|error| format!("github_fork_prepare refused: failed to run gh: {error}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    // Read back through the repository's own Git projection rather than by naming the directory again.
    let remotes = git_stdout_for_tool(
        tools::github_fork_prepare::NAME,
        repository_root.git(),
        &["remote", "-v"],
    )?;
    serde_json::to_string_pretty(&json!({
        "tool": tools::github_fork_prepare::NAME,
        "dry_run": false,
        "program": "gh",
        "args": args,
        "cwd": root.display().to_string(),
        "exit_code": output.status.code(),
        "success": output.status.success(),
        "stdout": stdout,
        "stderr": stderr,
        "remotes": remotes
    }))
    .map_err(|error| format!("github_fork_prepare refused: {error}"))
}
