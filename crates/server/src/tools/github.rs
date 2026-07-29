pub mod github_fork_prepare {
    pub const NAME: &str = "github_fork_prepare";
}

pub mod github_pr_run {
    pub const NAME: &str = "github_pr_run";
}

use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

use crate::tools;
use crate::tools::common::{nonempty_tool_string, optional_bool, optional_u64, required_string};
use crate::tools::git::support::{canonical_repo_root, git_stdout_for_tool};

pub(crate) fn call_github_pr_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "create pull request";

    let action = required_string(arguments, "action")?;
    let root = canonical_repo_root(repo_root, tools::github_pr_run::NAME)?;
    let args: Vec<String> = match action {
        "auth_status" => vec!["auth".to_string(), "status".to_string()],
        "pr_view" => {
            let number = optional_u64(arguments, "number")?.ok_or_else(|| {
                "github_pr_run refused: number is required for pr_view".to_string()
            })?;
            vec![
                "pr".to_string(),
                "view".to_string(),
                number.to_string(),
                "--json".to_string(),
                "number,title,state,url,author,headRefName,baseRefName,comments,reviews,statusCheckRollup".to_string(),
            ]
        }
        "pr_checks" => {
            let number = optional_u64(arguments, "number")?.ok_or_else(|| {
                "github_pr_run refused: number is required for pr_checks".to_string()
            })?;
            vec!["pr".to_string(), "checks".to_string(), number.to_string()]
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
            if confirm != CONFIRMATION {
                return Err(format!(
                    "github_pr_run refused: confirm must be {CONFIRMATION:?}"
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

    let output = Command::new("gh")
        .args(&args)
        .current_dir(&root)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("github_pr_run refused: failed to run gh: {error}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code();
    serde_json::to_string_pretty(&json!({
        "tool": tools::github_pr_run::NAME,
        "action": action,
        "program": "gh",
        "args": args,
        "cwd": root.display().to_string(),
        "exit_code": exit_code,
        "success": output.status.success(),
        "stdout": stdout,
        "stderr": stderr
    }))
    .map_err(|error| format!("github_pr_run refused: {error}"))
}

pub(crate) fn call_github_fork_prepare(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "prepare github fork";

    let root = canonical_repo_root(repo_root, tools::github_fork_prepare::NAME)?;
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

    let output = Command::new("gh")
        .args(&args)
        .current_dir(&root)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("github_fork_prepare refused: failed to run gh: {error}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let remotes = git_stdout_for_tool(tools::github_fork_prepare::NAME, &root, &["remote", "-v"])?;
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
