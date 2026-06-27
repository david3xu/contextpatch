mod protocol;
mod tools;

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

use contextpatch_core::fs::read_range::read_range_in_root;
use contextpatch_core::fs::write_new_file::write_new_file_in_root;
use contextpatch_core::git::status::{status_summary, status_summary_for_path};
use contextpatch_core::patch::diff::preview_exact_replacement_in_root;
use contextpatch_core::process::guarded_command::run_guarded_command;
use contextpatch_core::replace::exact::replace_exact_in_root;
use serde_json::{json, Value};

fn main() -> ExitCode {
    let repo_root = match parse_repo_root(std::env::args().skip(1).collect()) {
        Ok(repo_root) => repo_root,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };

    run_stdio_server(&repo_root)
}

fn parse_repo_root(args: Vec<String>) -> Result<PathBuf, String> {
    let mut repo_root = std::env::current_dir()
        .map_err(|error| format!("failed to read current directory: {error}"))?;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--repo-root" => {
                index += 1;
                repo_root = PathBuf::from(
                    args.get(index)
                        .ok_or_else(|| "--repo-root requires a value".to_string())?,
                );
            }
            "--help" | "-h" => {
                return Err(
                    "usage: contextpatch-server [--repo-root <path>]\n\nRuns the stdio MCP server."
                        .to_string(),
                );
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
        index += 1;
    }

    Ok(repo_root)
}

fn run_stdio_server(repo_root: &Path) -> ExitCode {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                eprintln!("failed to read stdin: {error}");
                return ExitCode::from(1);
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let response = handle_line(repo_root, &line);
        if let Some(response) = response {
            if let Err(error) = writeln!(stdout, "{response}") {
                eprintln!("failed to write stdout: {error}");
                return ExitCode::from(1);
            }
            if let Err(error) = stdout.flush() {
                eprintln!("failed to flush stdout: {error}");
                return ExitCode::from(1);
            }
        }
    }

    ExitCode::SUCCESS
}

fn handle_line(repo_root: &Path, line: &str) -> Option<String> {
    let request: Value = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(error) => {
            return Some(error_response(
                Value::Null,
                -32700,
                &format!("parse error: {error}"),
            ))
        }
    };

    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);
    let Some(method) = method else {
        return id.map(|id| error_response(id, -32600, "missing method"));
    };

    match (method, id) {
        ("initialize", Some(id)) => Some(success_response(
            id,
            json!({
                "protocolVersion": requested_protocol_version(&request),
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": protocol::schema::PROTOCOL_NAME,
                    "version": contextpatch_core::VERSION
                }
            }),
        )),
        ("tools/list", Some(id)) => Some(success_response(
            id,
            json!({
                "tools": tool_definitions()
            }),
        )),
        ("tools/call", Some(id)) => Some(handle_tool_call(repo_root, id, &request)),
        ("notifications/initialized", None) => None,
        (_, Some(id)) => Some(error_response(
            id,
            -32601,
            &format!("unknown method: {method}"),
        )),
        (_, None) => None,
    }
}

fn requested_protocol_version(request: &Value) -> String {
    request
        .get("params")
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str)
        .unwrap_or("2024-11-05")
        .to_string()
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": tools::capability_manifest::NAME,
            "description": "Report the contextpatch server capability contract, including whether guarded process execution is available.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": tools::preflight_health::NAME,
            "description": "Check repository and local tool readiness for Claude Desktop workflows without mutating the repository.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": tools::read_range::NAME,
            "description": "Read a bounded section of a UTF-8 text file with 1-based line numbers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to the configured repository root."
                    },
                    "start_line": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "First 1-based line number to read."
                    },
                    "end_line": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Last 1-based line number to read."
                    }
                },
                "required": ["path", "start_line", "end_line"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::diff_preview::NAME,
            "description": "Return a unified diff for an exact replacement without writing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to the configured repository root."
                    },
                    "old": {
                        "type": "string",
                        "description": "Existing text that must appear exactly once."
                    },
                    "new": {
                        "type": "string",
                        "description": "Replacement text to preview."
                    }
                },
                "required": ["path", "old", "new"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::replace_exact::NAME,
            "description": "Replace text only when the old text matches exactly once.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to the configured repository root."
                    },
                    "old": {
                        "type": "string",
                        "description": "Existing text that must appear exactly once."
                    },
                    "new": {
                        "type": "string",
                        "description": "Replacement text."
                    }
                },
                "required": ["path", "old", "new"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::status_guard::NAME,
            "description": "Refuse when the repository or requested path has uncommitted Git changes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Optional file or directory path relative to the configured repository root."
                    }
                },
                "additionalProperties": false
            }
        },
        {
            "name": tools::write_new_file::NAME,
            "description": "Create a new UTF-8 text file only when the destination does not already exist.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to the configured repository root."
                    },
                    "content": {
                        "type": "string",
                        "description": "Full file content to write."
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::run_guarded_command::NAME,
            "description": "Run a repo-root-confined allowlisted validation command without using a shell.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "program": {
                        "type": "string",
                        "description": "Allowlisted executable name: git, cargo, bun, npm, or rg."
                    },
                    "args": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "description": "Command arguments. The first argument must be an allowlisted subcommand."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory relative to the configured repository root."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "description": "Optional timeout in seconds. Defaults to 120, maximum 600."
                    }
                },
                "required": ["program", "args"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::read_command_log::NAME,
            "description": "Read a previously captured guarded command log by log_id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log_id": {
                        "type": "string",
                        "description": "Opaque log id returned by run_guarded_command or validation_profile_run."
                    },
                    "max_chars": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 200000,
                        "description": "Optional maximum characters to return. Defaults to 12000."
                    }
                },
                "required": ["log_id"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::validation_profile_run::NAME,
            "description": "Run a predefined sequence of allowlisted validation commands and return compact results with log ids.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "profile": {
                        "type": "string",
                        "description": "Validation profile name: repo-basic, rust-workspace, datacore-vscode, or datacore-m6-vscode."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "description": "Optional per-command timeout. Defaults to each profile command timeout."
                    },
                    "stop_on_failure": {
                        "type": "boolean",
                        "description": "Stop after the first non-zero or timed-out command. Defaults to true."
                    }
                },
                "required": ["profile"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::git_commit_exact::NAME,
            "description": "Dry-run or create one local Git commit from an exact full dirty-path set. Never pushes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Exact repository-relative dirty paths that must be the complete changed-path set."
                    },
                    "subject": {
                        "type": "string",
                        "description": "Commit subject line."
                    },
                    "body": {
                        "type": "string",
                        "description": "Optional commit body/trailers."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Validate and preview without staging or committing. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `commit exact paths` when dry_run is false."
                    }
                },
                "required": ["paths", "subject"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::git_remote_check::NAME,
            "description": "Fetch one remote branch and report whether the remote branch is ahead of HEAD. Does not modify source files.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "remote": {
                        "type": "string",
                        "description": "Git remote name. Defaults to origin."
                    },
                    "branch": {
                        "type": "string",
                        "description": "Branch name to compare with the remote-tracking ref."
                    }
                },
                "required": ["branch"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::git_push_exact::NAME,
            "description": "Push the current branch HEAD to the matching remote branch only after exact hash and divergence checks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "remote": {
                        "type": "string",
                        "description": "Git remote name."
                    },
                    "branch": {
                        "type": "string",
                        "description": "Current branch name and matching remote branch name."
                    },
                    "expected_head": {
                        "type": "string",
                        "description": "Full or short commit hash expected at HEAD."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `push exact commit`."
                    }
                },
                "required": ["remote", "branch", "expected_head", "confirm"],
                "additionalProperties": false
            }
        }
    ])
}

fn handle_tool_call(repo_root: &Path, id: Value, request: &Value) -> String {
    let Some(params) = request.get("params") else {
        return error_response(id, -32602, "tools/call missing params");
    };
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_response(id, -32602, "tools/call missing tool name");
    };
    let arguments = params
        .get("arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let result = match name {
        tools::capability_manifest::NAME => call_capability_manifest(repo_root),
        tools::preflight_health::NAME => call_preflight_health(repo_root),
        tools::read_range::NAME => call_read_range(repo_root, &arguments),
        tools::diff_preview::NAME => call_diff_preview(repo_root, &arguments),
        tools::replace_exact::NAME => call_replace_exact(repo_root, &arguments),
        tools::status_guard::NAME => call_status_guard(repo_root, &arguments),
        tools::write_new_file::NAME => call_write_new_file(repo_root, &arguments),
        tools::run_guarded_command::NAME => call_run_guarded_command(repo_root, &arguments),
        tools::read_command_log::NAME => call_read_command_log(&arguments),
        tools::validation_profile_run::NAME => call_validation_profile_run(repo_root, &arguments),
        tools::git_commit_exact::NAME => call_git_commit_exact(repo_root, &arguments),
        tools::git_remote_check::NAME => call_git_remote_check(repo_root, &arguments),
        tools::git_push_exact::NAME => call_git_push_exact(repo_root, &arguments),
        unknown => Err(format!("unknown tool: {unknown}")),
    };

    match result {
        Ok(text) => success_response(
            id,
            json!({
                "content": [
                    {
                        "type": "text",
                        "text": text
                    }
                ]
            }),
        ),
        Err(message) => success_response(
            id,
            json!({
                "isError": true,
                "content": [
                    {
                        "type": "text",
                        "text": message
                    }
                ]
            }),
        ),
    }
}

fn call_capability_manifest(repo_root: &Path) -> Result<String, String> {
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("capability_manifest refused: {error}"))?;
    Ok(serde_json::to_string_pretty(&json!({
        "server": "contextpatch",
        "version": contextpatch_core::VERSION,
        "repo_root": root.display().to_string(),
        "file_tools": {
            "read_range": true,
            "diff_preview": true,
            "replace_exact": true,
            "write_new_file": true,
            "status_guard": true,
            "read_command_log": true,
            "git_commit_exact": true,
            "git_remote_check": true,
            "git_push_exact": true
        },
        "git_workflows": {
            "local_commit_exact_paths": true,
            "remote_check": true,
            "push_exact_commit": true,
            "reset_checkout_clean_stash": false,
            "guards": [
                "requires exact complete dirty-path set",
                "defaults to dry_run",
                "requires confirm literal for mutation",
                "stages only explicit paths",
                "creates at most one local commit",
                "fetches only explicit remote branch",
                "pushes only expected HEAD to matching remote branch",
                "never force pushes"
            ]
        },
        "process_execution": {
            "available": true,
            "mode": "allowlisted_no_shell",
            "programs": {
                "git": ["status", "diff", "log", "show", "rev-parse", "ls-tree"],
                "cargo": ["check", "test", "build", "clippy"],
                "bun": ["run", "test"],
                "npm": ["run", "test"],
                "rg": ["search"]
            },
            "validation_profiles": ["repo-basic", "rust-workspace", "datacore-vscode", "datacore-m6-vscode"],
            "guards": [
                "repo-root-confined cwd",
                "no shell interpolation",
                "allowlisted program and subcommand",
                "timeout bounded",
                "secret-like output redaction",
                "stdout/stderr truncation",
                "command/cwd/exit-code/duration metadata"
            ],
            "not_supported": [
                "arbitrary shell",
                "force push/pull/reset/checkout/stash/clean",
                "ungated or automatic commits",
                "path traversal outside repo root",
                "secret-printing environment inspection"
            ]
        }
    }))
    .map_err(|error| format!("capability_manifest refused: {error}"))?)
}

fn call_preflight_health(repo_root: &Path) -> Result<String, String> {
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("preflight_health refused: {error}"))?;
    let git_status = match status_summary(&root) {
        Ok(summary) => json!({ "clean": true, "summary": summary }),
        Err(error) => json!({ "clean": false, "summary": error.to_string() }),
    };
    Ok(serde_json::to_string_pretty(&json!({
        "server": "contextpatch",
        "version": contextpatch_core::VERSION,
        "repo_root": root.display().to_string(),
        "repository": git_status,
        "guarded_process_execution": {
            "available": true,
            "mode": "allowlisted_no_shell",
            "default_timeout_secs": 120,
            "max_timeout_secs": 600
        },
        "tools": {
            "git": executable_available("git"),
            "cargo": executable_available("cargo"),
            "bun": executable_available("bun"),
            "npm": executable_available("npm"),
            "rg": executable_available("rg")
        }
    }))
    .map_err(|error| format!("preflight_health refused: {error}"))?)
}

fn call_read_range(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let start_line = required_usize(arguments, "start_line")?;
    let end_line = required_usize(arguments, "end_line")?;

    read_range_in_root(repo_root, Path::new(path), start_line, end_line)
        .map_err(|error| format!("read_range refused: {error}"))
}

fn call_diff_preview(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let old = required_string(arguments, "old")?;
    let new = required_string(arguments, "new")?;

    preview_exact_replacement_in_root(repo_root, Path::new(path), old, new)
        .map_err(|error| format!("diff_preview refused: {error}"))
}

fn call_replace_exact(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let old = required_string(arguments, "old")?;
    let new = required_string(arguments, "new")?;

    let summary = replace_exact_in_root(repo_root, Path::new(path), old, new)
        .map_err(|error| format!("replace_exact refused: {error}"))?;

    Ok(format!(
        "replaced bytes {}..{} in {} ({} bytes written)",
        summary.start_byte,
        summary.end_byte,
        summary.path.display(),
        summary.bytes_written
    ))
}

fn call_status_guard(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    match optional_string(arguments, "path")? {
        Some(path) => status_summary_for_path(repo_root, Some(Path::new(path))),
        None => status_summary(repo_root),
    }
    .map_err(|error| format!("status_guard refused: {error}"))
}

fn call_write_new_file(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let content = required_string(arguments, "content")?;

    let summary = write_new_file_in_root(repo_root, Path::new(path), content)
        .map_err(|error| format!("write_new_file refused: {error}"))?;

    Ok(format!(
        "created {} ({} bytes written)",
        summary.path.display(),
        summary.bytes_written
    ))
}

fn call_run_guarded_command(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let program = required_string(arguments, "program")?;
    let args = required_string_array(arguments, "args")?;
    let cwd = optional_string(arguments, "cwd")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;

    let output = run_guarded_command(repo_root, cwd.map(Path::new), program, &args, timeout_secs)
        .map_err(|error| format!("run_guarded_command refused: {error}"))?;
    let log_id = write_command_log(&output)
        .map_err(|error| format!("run_guarded_command log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{output}"))
}

fn call_read_command_log(arguments: &serde_json::Map<String, Value>) -> Result<String, String> {
    let log_id = required_string(arguments, "log_id")?;
    let max_chars = optional_u64(arguments, "max_chars")?.unwrap_or(12_000);
    if max_chars == 0 || max_chars > 200_000 {
        return Err("read_command_log refused: max_chars must be between 1 and 200000".to_string());
    }
    let path = command_log_path(log_id)?;
    let mut text = fs::read_to_string(&path)
        .map_err(|error| format!("read_command_log refused: failed to read {log_id}: {error}"))?;
    if text.len() > max_chars as usize {
        text.truncate(max_chars as usize);
        text.push_str("\n[truncated]");
    }
    Ok(format!("log_id: {log_id}\n{text}"))
}

fn call_validation_profile_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let profile = required_string(arguments, "profile")?;
    let timeout_override = optional_u64(arguments, "timeout_secs")?;
    let stop_on_failure = optional_bool(arguments, "stop_on_failure")?.unwrap_or(true);
    let commands = validation_profile(profile)?;

    let started = std::time::Instant::now();
    let mut lines = vec![
        format!("profile: {profile}"),
        format!("commands_planned: {}", commands.len()),
        format!("stop_on_failure: {stop_on_failure}"),
    ];
    let mut failed = false;
    let mut ran = 0usize;

    for (index, command) in commands.iter().enumerate() {
        ran += 1;
        let timeout_secs = timeout_override.or(command.timeout_secs);
        let output = run_guarded_command(
            repo_root,
            command.cwd.map(Path::new),
            command.program,
            &command
                .args
                .iter()
                .map(|arg| arg.to_string())
                .collect::<Vec<_>>(),
            timeout_secs,
        )
        .map_err(|error| {
            format!(
                "validation_profile_run refused at command {} ({}): {error}",
                index + 1,
                command.display()
            )
        })?;
        let log_id = write_command_log(&output)
            .map_err(|error| format!("validation_profile_run log write failed: {error}"))?;
        let exit_code = extract_field(&output, "exit_code").unwrap_or("unknown");
        let timed_out = extract_field(&output, "timed_out").unwrap_or("unknown");
        let duration_ms = extract_field(&output, "duration_ms").unwrap_or("unknown");
        let command_failed = timed_out == "true" || exit_code != "0";
        failed |= command_failed;
        lines.push(format!(
            "{}. {} | exit_code: {exit_code} | timed_out: {timed_out} | duration_ms: {duration_ms} | log_id: {log_id}",
            index + 1,
            command.display()
        ));
        if command_failed && stop_on_failure {
            lines.push(format!("stopped_after_failure: {}", index + 1));
            break;
        }
    }

    lines.insert(3, format!("commands_run: {ran}"));
    lines.insert(4, format!("failed: {failed}"));
    lines.push(format!("duration_ms: {}", started.elapsed().as_millis()));
    Ok(lines.join("\n"))
}

fn call_git_commit_exact(
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
    let normalized_paths = normalize_git_paths(&root, &paths)?;
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
}

fn call_git_remote_check(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let remote = validate_git_remote(optional_string(arguments, "remote")?.unwrap_or("origin"))?;
    let branch = validate_git_branch(required_string(arguments, "branch")?)?;
    let root = canonical_repo_root(repo_root, tools::git_remote_check::NAME)?;
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

fn call_git_push_exact(
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

    let root = canonical_repo_root(repo_root, tools::git_push_exact::NAME)?;
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

fn required_string<'a>(
    arguments: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or invalid string argument: {key}"))
}

fn optional_string<'a>(
    arguments: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    match arguments.get(key) {
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("invalid string argument: {key}")),
        None => Ok(None),
    }
}

fn required_string_array(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let values = arguments
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing or invalid string array argument: {key}"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| format!("invalid string array item in argument: {key}"))
        })
        .collect()
}

fn required_usize(arguments: &serde_json::Map<String, Value>, key: &str) -> Result<usize, String> {
    let value = arguments
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing or invalid integer argument: {key}"))?;

    usize::try_from(value).map_err(|_| format!("integer argument out of range: {key}"))
}

fn optional_u64(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<u64>, String> {
    match arguments.get(key) {
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid integer argument: {key}")),
        None => Ok(None),
    }
}

fn optional_bool(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<bool>, String> {
    match arguments.get(key) {
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| format!("invalid boolean argument: {key}")),
        None => Ok(None),
    }
}

fn validate_commit_subject(subject: &str) -> Result<String, String> {
    let trimmed = subject.trim();
    if trimmed.is_empty() {
        return Err("git_commit_exact refused: subject must not be empty".to_string());
    }
    if trimmed.len() > 200 {
        return Err("git_commit_exact refused: subject must be at most 200 bytes".to_string());
    }
    if subject.contains('\n') || subject.contains('\r') || subject.contains('\0') {
        return Err(
            "git_commit_exact refused: subject must be a single line without NUL bytes".to_string(),
        );
    }
    Ok(trimmed.to_string())
}

fn validate_commit_body(body: &str) -> Result<String, String> {
    if body.contains('\0') {
        return Err("git_commit_exact refused: body must not contain NUL bytes".to_string());
    }
    if body.len() > 20_000 {
        return Err("git_commit_exact refused: body must be at most 20000 bytes".to_string());
    }
    Ok(body.trim().to_string())
}

fn normalize_git_paths(root: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| normalize_git_path(root, path))
        .collect()
}

fn normalize_git_path(root: &Path, raw: &str) -> Result<String, String> {
    if raw.is_empty() || raw.contains('\0') {
        return Err("git_commit_exact refused: path must not be empty or contain NUL".to_string());
    }
    if raw.starts_with(':') || raw.contains('*') || raw.contains('?') || raw.contains('[') {
        return Err(format!(
            "git_commit_exact refused: path `{raw}` contains Git pathspec metacharacters"
        ));
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!(
            "git_commit_exact refused: path `{raw}` must be repository-relative"
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "git_commit_exact refused: path `{raw}` must be a normalized relative path"
                ))
            }
        }
    }

    let candidate = root.join(path);
    let resolved = if candidate.exists() {
        candidate.canonicalize().map_err(|error| {
            format!("git_commit_exact refused: failed to resolve path `{raw}`: {error}")
        })?
    } else {
        let parent = candidate
            .parent()
            .ok_or_else(|| format!("git_commit_exact refused: path `{raw}` has no parent"))?;
        let parent = parent.canonicalize().map_err(|error| {
            format!("git_commit_exact refused: failed to resolve parent for `{raw}`: {error}")
        })?;
        let file_name = candidate
            .file_name()
            .ok_or_else(|| format!("git_commit_exact refused: path `{raw}` has no file name"))?;
        parent.join(file_name)
    };

    if !resolved.starts_with(root) {
        return Err(format!(
            "git_commit_exact refused: path `{raw}` resolves outside repository root"
        ));
    }

    Ok(raw.to_string())
}

fn git_status_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    let output = git_output(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_porcelain_paths(&output.stdout, "git status")
}

fn git_cached_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    let output = git_output(root, &["diff", "--cached", "--name-only", "-z"])?;
    parse_nul_paths(&output.stdout, "git diff --cached")
}

fn parse_porcelain_paths(bytes: &[u8], label: &str) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    let mut entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(format!(
                "git_commit_exact refused: unexpected {label} entry"
            ));
        }
        if matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C') {
            return Err(format!(
                "git_commit_exact refused: rename/copy entries require a future dedicated tool"
            ));
        }
        let path = std::str::from_utf8(&entry[3..]).map_err(|error| {
            format!("git_commit_exact refused: {label} path is not UTF-8: {error}")
        })?;
        paths.insert(path.to_string());
    }
    Ok(paths)
}

fn parse_nul_paths(bytes: &[u8], label: &str) -> Result<BTreeSet<String>, String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            std::str::from_utf8(entry)
                .map(|path| path.to_string())
                .map_err(|error| {
                    format!("git_commit_exact refused: {label} path is not UTF-8: {error}")
                })
        })
        .collect()
}

fn git_args(subcommand: &str, paths: &[String]) -> Vec<String> {
    std::iter::once(subcommand.to_string())
        .chain(std::iter::once("--".to_string()))
        .chain(paths.iter().cloned())
        .collect()
}

fn git_stdout(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_output(root, args)?;
    String::from_utf8(output.stdout)
        .map_err(|error| format!("git_commit_exact refused: git output was not UTF-8: {error}"))
}

fn git_success(root: &Path, args: Vec<String>) -> Result<(), String> {
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let output = git_output(root, &arg_refs)?;
    if !output.status.success() {
        return Err(format!(
            "git_commit_exact refused: git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn git_output(root: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("git_commit_exact refused: failed to run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git_commit_exact refused: git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

fn canonical_repo_root(repo_root: &Path, tool_name: &str) -> Result<PathBuf, String> {
    repo_root
        .canonicalize()
        .map_err(|error| format!("{tool_name} refused: failed to resolve repo root: {error}"))
}

fn git_status_short(root: &Path) -> Result<String, String> {
    git_stdout_for_tool(
        "git_status_short",
        root,
        &["status", "--short", "--untracked-files=all"],
    )
}

fn git_stdout_for_tool(tool_name: &str, root: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_output_for_tool(tool_name, root, args)?;
    String::from_utf8(output.stdout)
        .map_err(|error| format!("{tool_name} refused: git output was not UTF-8: {error}"))
}

fn git_success_for_tool(tool_name: &str, root: &Path, args: Vec<String>) -> Result<(), String> {
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let _ = git_output_for_tool(tool_name, root, &arg_refs)?;
    Ok(())
}

fn git_output_for_tool(
    tool_name: &str,
    root: &Path,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("{tool_name} refused: failed to run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "{tool_name} refused: git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

fn git_status_code(root: &Path, args: &[&str]) -> Result<i32, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("git operation refused: failed to run git: {error}"))?;
    Ok(output.status.code().unwrap_or(-1))
}

fn rev_count_for_tool(tool_name: &str, root: &Path, range: &str) -> Result<u64, String> {
    let output = git_stdout_for_tool(tool_name, root, &["rev-list", "--count", range])?;
    output.trim().parse::<u64>().map_err(|error| {
        format!(
            "{tool_name} refused: failed to parse revision count `{}`: {error}",
            output.trim()
        )
    })
}

fn ensure_remote_exists(tool_name: &str, root: &Path, remote: &str) -> Result<(), String> {
    let remotes = git_stdout_for_tool(tool_name, root, &["remote"])?;
    if remotes.lines().any(|line| line == remote) {
        Ok(())
    } else {
        Err(format!(
            "{tool_name} refused: remote `{remote}` is not configured"
        ))
    }
}

fn validate_git_remote(remote: &str) -> Result<String, String> {
    validate_git_ref_component("remote", remote)?;
    Ok(remote.to_string())
}

fn validate_git_branch(branch: &str) -> Result<String, String> {
    validate_git_ref_component("branch", branch)?;
    if branch.contains("..")
        || branch.contains("@{")
        || branch.starts_with('-')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.ends_with(".lock")
    {
        return Err(format!("git workflow refused: invalid branch `{branch}`"));
    }
    let status = Command::new("git")
        .args(["check-ref-format", "--branch", branch])
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("git workflow refused: failed to validate branch: {error}"))?;
    if !status.status.success() {
        return Err(format!("git workflow refused: invalid branch `{branch}`"));
    }
    Ok(branch.to_string())
}

fn validate_git_ref_component(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.contains('\0')
        || value.chars().any(char::is_whitespace)
        || value.starts_with('-')
        || value.contains('\\')
        || value.contains(':')
        || value.contains('^')
        || value.contains('~')
        || value.contains('?')
        || value.contains('*')
        || value.contains('[')
    {
        return Err(format!("git workflow refused: invalid {label} `{value}`"));
    }
    Ok(())
}

fn validate_expected_head(expected_head: &str) -> Result<String, String> {
    if expected_head.len() < 7 || expected_head.len() > 40 {
        return Err(
            "git_push_exact refused: expected_head must be a 7 to 40 character commit hash"
                .to_string(),
        );
    }
    if !expected_head.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("git_push_exact refused: expected_head must be hexadecimal".to_string());
    }
    Ok(expected_head.to_ascii_lowercase())
}

fn remote_ref(remote: &str, branch: &str) -> String {
    format!("refs/remotes/{remote}/{branch}")
}

fn empty_label(text: &str) -> &str {
    if text.trim().is_empty() {
        "(empty)"
    } else {
        text
    }
}

fn format_set(paths: &BTreeSet<String>) -> String {
    if paths.is_empty() {
        return "(none)".to_string();
    }
    paths.iter().cloned().collect::<Vec<_>>().join("\n")
}

struct ProfileCommand {
    program: &'static str,
    args: Vec<&'static str>,
    cwd: Option<&'static str>,
    timeout_secs: Option<u64>,
}

impl ProfileCommand {
    fn display(&self) -> String {
        std::iter::once(self.program.to_string())
            .chain(self.args.iter().map(|arg| shell_display_arg(arg)))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn validation_profile(profile: &str) -> Result<Vec<ProfileCommand>, String> {
    match profile {
        "repo-basic" => Ok(vec![
            ProfileCommand {
                program: "git",
                args: vec!["status", "--branch", "--short"],
                cwd: None,
                timeout_secs: Some(30),
            },
            ProfileCommand {
                program: "git",
                args: vec!["diff", "--check"],
                cwd: None,
                timeout_secs: Some(30),
            },
        ]),
        "rust-workspace" => Ok(vec![ProfileCommand {
            program: "cargo",
            args: vec!["test", "--workspace"],
            cwd: None,
            timeout_secs: Some(600),
        }]),
        "datacore-vscode" => Ok(vec![
            ProfileCommand {
                program: "bun",
                args: vec!["run", "vscode:check"],
                cwd: None,
                timeout_secs: Some(600),
            },
            ProfileCommand {
                program: "bun",
                args: vec!["run", "sdk:typescript:test"],
                cwd: None,
                timeout_secs: Some(600),
            },
            ProfileCommand {
                program: "bun",
                args: vec!["run", "validation/contract-compatibility/run.ts"],
                cwd: None,
                timeout_secs: Some(600),
            },
        ]),
        "datacore-m6-vscode" => {
            let mut commands = validation_profile("datacore-vscode")?;
            commands.push(ProfileCommand {
                program: "bun",
                args: vec!["run", "validate:live-answer"],
                cwd: None,
                timeout_secs: Some(600),
            });
            commands.push(ProfileCommand {
                program: "bun",
                args: vec!["run", "vscode:test:live"],
                cwd: None,
                timeout_secs: Some(600),
            });
            Ok(commands)
        }
        _ => Err(format!(
            "validation_profile_run refused: unknown profile `{profile}`; expected repo-basic, rust-workspace, datacore-vscode, or datacore-m6-vscode"
        )),
    }
}

fn extract_field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}: ");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix).map(str::trim))
}

fn write_command_log(text: &str) -> Result<String, String> {
    let dir = command_log_dir();
    fs::create_dir_all(&dir).map_err(|error| {
        format!(
            "failed to create command log directory {}: {error}",
            dir.display()
        )
    })?;
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock error: {error}"))?
        .as_nanos();
    let log_id = format!("cmd-{}-{unique}", std::process::id());
    fs::write(dir.join(format!("{log_id}.log")), text)
        .map_err(|error| format!("failed to write command log {log_id}: {error}"))?;
    Ok(log_id)
}

fn command_log_path(log_id: &str) -> Result<PathBuf, String> {
    if log_id.is_empty()
        || !log_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Err("read_command_log refused: invalid log_id".to_string());
    }
    Ok(command_log_dir().join(format!("{log_id}.log")))
}

fn command_log_dir() -> PathBuf {
    std::env::temp_dir().join("contextpatch-command-logs")
}

fn shell_display_arg(arg: &str) -> String {
    if arg
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '=' | ','))
    {
        arg.to_string()
    } else {
        format!("{arg:?}")
    }
}

fn executable_available(program: &str) -> Value {
    let output = std::process::Command::new(program)
        .arg("--version")
        .output();
    match output {
        Ok(output) => json!({
            "available": output.status.success(),
            "exit_code": output.status.code().unwrap_or(-1)
        }),
        Err(error) => json!({
            "available": false,
            "error": error.to_string()
        }),
    }
}

fn success_response(id: Value, result: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
    .to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
    .to_string()
}
