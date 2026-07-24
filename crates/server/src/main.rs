mod protocol;
mod tools;

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use contextpatch_core::fs::read_range::read_range_in_root;
use contextpatch_core::fs::write_new_file::write_new_file_in_root;
use contextpatch_core::git::status::{status_summary, status_summary_for_path};
use contextpatch_core::native_build::{native_build_run, NativeBuildParams};
use contextpatch_core::native_device::{native_device_run, NativeDeviceParams};
use contextpatch_core::patch::diff::preview_exact_replacement_in_root;
use contextpatch_core::process::guarded_command::run_guarded_command;
use contextpatch_core::replace::exact::replace_exact_in_root;
use contextpatch_core::setup::profile::{setup_profile_run, CapacitorPlatform, SetupActionParams};
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
            "name": tools::setup_profile_run::NAME,
            "description": "Plan or run a predefined repository setup action from a typed profile. Callers do not provide raw commands.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "profile": {
                        "type": "string",
                        "description": "Setup profile name. Supported: node-capacitor-shell."
                    },
                    "action": {
                        "type": "string",
                        "description": "Profile action, such as install_capacitor_dependencies, cap_init, cap_add_ios, cap_add_android, or cap_sync."
                    },
                    "params": {
                        "type": "object",
                        "description": "Typed action parameters. cap_init requires app_id, app_name, and web_dir; cap_sync accepts platform: ios, android, or all."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory relative to the configured repository root."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "description": "Optional timeout for the planned command. Defaults to 120, maximum 600."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Plan only without mutating. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required mutation confirmation literal `run setup profile` when dry_run is false."
                    }
                },
                "required": ["profile", "action"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::native_build_run::NAME,
            "description": "Plan or run a typed native build/test action without exposing raw xcodebuild or Gradle commands.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "Native build action: ios_build, ios_test, android_assemble_debug, or android_unit_test."
                    },
                    "params": {
                        "type": "object",
                        "description": "Typed action parameters. iOS uses workspace, scheme, optional configuration/sdk/destination. Android accepts optional gradlew path."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory relative to the configured repository root."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Plan only without running the build. Defaults to true."
                    }
                },
                "required": ["action", "params"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::native_device_run::NAME,
            "description": "Plan or run bounded typed native simulator/emulator/device smoke actions without arbitrary xcrun or adb access.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "Native device action such as ios_list_simulators, ios_boot_simulator, ios_install_app, ios_launch_app, ios_read_logs, android_list_devices, android_install_app, android_launch_app, or android_read_logcat."
                    },
                    "params": {
                        "type": "object",
                        "description": "Typed action parameters such as device, serial, app_id, app_path, apk_path, or lines."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory relative to the configured repository root."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Plan only without touching simulator/device state. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal `run native device` when dry_run is false for device-state changes."
                    }
                },
                "required": ["action"],
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
            "name": tools::git_branch_prepare::NAME,
            "description": "Prepare and switch to a local branch from one explicit remote base branch with guard checks.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "remote": {
                        "type": "string",
                        "description": "Git remote name. Defaults to origin."
                    },
                    "base_branch": {
                        "type": "string",
                        "description": "Remote base branch to fetch and prepare from."
                    },
                    "branch": {
                        "type": "string",
                        "description": "Local branch to create or switch to."
                    },
                    "required_files": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "description": "Optional repository-relative files that must exist after preparation."
                    },
                    "reset_existing": {
                        "type": "boolean",
                        "description": "Reset an existing local branch to the fetched remote base. Defaults to false."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `reset branch from remote base` when reset_existing is true."
                    }
                },
                "required": ["base_branch", "branch"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::git_merge_readiness::NAME,
            "description": "Read-only merge/PR readiness analysis between two refs, including changed-on-both-sides conflict candidates.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "base_ref": {
                        "type": "string",
                        "description": "Base ref to compare, such as HEAD, main, origin/main, refs/heads/main, or a commit hash."
                    },
                    "target_ref": {
                        "type": "string",
                        "description": "Target ref to compare, such as feature, origin/feature, refs/remotes/origin/feature, or a commit hash."
                    },
                    "fetch": {
                        "type": "boolean",
                        "description": "Optionally fetch one explicit remote branch before analysis. Defaults to false."
                    },
                    "remote": {
                        "type": "string",
                        "description": "Remote name used only when fetch is true. Defaults to origin."
                    },
                    "target_branch": {
                        "type": "string",
                        "description": "Remote branch to fetch when fetch is true. Inferred from target_ref when target_ref is a remote-tracking ref."
                    }
                },
                "required": ["base_ref", "target_ref"],
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
        tools::setup_profile_run::NAME => call_setup_profile_run(repo_root, &arguments),
        tools::native_build_run::NAME => call_native_build_run(repo_root, &arguments),
        tools::native_device_run::NAME => call_native_device_run(repo_root, &arguments),
        tools::git_commit_exact::NAME => call_git_commit_exact(repo_root, &arguments),
        tools::git_remote_check::NAME => call_git_remote_check(repo_root, &arguments),
        tools::git_branch_prepare::NAME => call_git_branch_prepare(repo_root, &arguments),
        tools::git_merge_readiness::NAME => call_git_merge_readiness(repo_root, &arguments),
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
    serde_json::to_string_pretty(&json!({
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
            "setup_profile_run": true,
            "native_build_run": true,
            "native_device_run": true,
            "git_commit_exact": true,
            "git_remote_check": true,
            "git_branch_prepare": true,
            "git_merge_readiness": true,
            "git_push_exact": true
        },
        "git_workflows": {
            "local_commit_exact_paths": true,
            "remote_check": true,
            "branch_prepare": true,
            "merge_readiness": true,
            "push_exact_commit": true,
            "reset_checkout_clean_stash": false,
            "guards": [
                "requires exact complete dirty-path set",
                "defaults to dry_run",
                "requires confirm literal for mutation",
                "stages only explicit paths",
                "creates at most one local commit",
                "fetches only explicit remote branch",
                "branch preparation requires clean worktree and validates required files",
                "merge readiness is read-only and reports changed-on-both-sides candidates",
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
        ,
        "setup_profiles": {
            "mode": "declarative_profile_actions",
            "profiles": {
                "node-capacitor-shell": {
                    "actions": [
                        "install_capacitor_dependencies",
                        "cap_init",
                        "cap_add_ios",
                        "cap_add_android",
                        "cap_sync",
                        "ios_pod_install"
                    ],
                    "external_mutator": true,
                    "caller_supplies_raw_command": false,
                    "mutation_enabled": true,
                    "required_confirm_for_mutation": "run setup profile"
                }
            },
            "examples": [
                {
                    "tool": "setup_profile_run",
                    "description": "Initialize Capacitor shell files without raw npx/cap commands.",
                    "arguments": {
                        "profile": "node-capacitor-shell",
                        "action": "cap_init",
                        "params": {
                            "app_id": "com.example.app",
                            "app_name": "Example",
                            "web_dir": "dist"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "setup_profile_run",
                    "description": "Install iOS CocoaPods dependencies without raw pod access.",
                    "arguments": {
                        "profile": "node-capacitor-shell",
                        "action": "ios_pod_install",
                        "cwd": "ios/App",
                        "dry_run": true
                    }
                }
            ],
            "guards": [
                "profile derives exact command plan",
                "typed action parameters only",
                "repo-root-confined cwd",
                "dry-run first",
                "mutation requires clean worktree and exact confirmation",
                "mutation reports before/after Git status and changed paths"
            ]
        },
        "native_build": {
            "mode": "typed_native_build_actions",
            "actions": {
                "ios_build": { "program": "xcodebuild", "caller_supplies_raw_command": false },
                "ios_test": { "program": "xcodebuild", "caller_supplies_raw_command": false },
                "android_assemble_debug": { "program": "./gradlew", "caller_supplies_raw_command": false },
                "android_unit_test": { "program": "./gradlew", "caller_supplies_raw_command": false }
            },
            "examples": [
                {
                    "tool": "native_build_run",
                    "description": "Plan an iOS simulator build.",
                    "arguments": {
                        "action": "ios_build",
                        "params": {
                            "workspace": "ios/App/App.xcworkspace",
                            "scheme": "App"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_build_run",
                    "description": "Plan an Android debug build using the repository Gradle wrapper.",
                    "arguments": {
                        "action": "android_assemble_debug",
                        "params": {},
                        "dry_run": true
                    }
                }
            ],
            "guards": [
                "typed action parameters only",
                "repo-root-confined cwd",
                "repo-relative Gradle wrapper is canonicalized under the repository",
                "source Git status is checked before and after execution",
                "large output is available through read_command_log"
            ]
        },
        "native_device": {
            "mode": "typed_native_device_actions",
            "actions": [
                "ios_list_simulators",
                "ios_boot_simulator",
                "ios_install_app",
                "ios_launch_app",
                "ios_read_logs",
                "android_list_devices",
                "android_install_app",
                "android_launch_app",
                "android_read_logcat"
            ],
            "required_confirm_for_device_state": "run native device",
            "examples": [
                {
                    "tool": "native_device_run",
                    "description": "Plan an iOS app launch on the booted simulator.",
                    "arguments": {
                        "action": "ios_launch_app",
                        "params": {
                            "device": "booted",
                            "app_id": "com.example.app"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_device_run",
                    "description": "Read recent Android logcat output without changing device state.",
                    "arguments": {
                        "action": "android_read_logcat",
                        "params": {
                            "serial": "emulator-5554",
                            "lines": 200
                        },
                        "dry_run": true
                    }
                }
            ],
            "guards": [
                "typed action parameters only",
                "no arbitrary xcrun or adb",
                "device-state changes require dry_run=false plus exact confirmation",
                "bounded timeout and redacted log output",
                "repository source files are not mutated"
            ]
        }
    }))
    .map_err(|error| format!("capability_manifest refused: {error}"))
}

fn call_preflight_health(repo_root: &Path) -> Result<String, String> {
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("preflight_health refused: {error}"))?;
    let git_status = match status_summary(&root) {
        Ok(summary) => json!({ "clean": true, "summary": summary }),
        Err(error) => json!({ "clean": false, "summary": error.to_string() }),
    };
    serde_json::to_string_pretty(&json!({
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
        },
        "setup_profiles": {
            "node-capacitor-shell": {
                "available": executable_is_available("npm"),
                "required_tools": {
                    "npm": executable_available("npm"),
                    "pod": executable_available("pod")
                },
                "mutation_enabled": true
            }
        },
        "native_build": {
            "available": executable_is_available("xcodebuild") || gradlew_available(&root),
            "required_tools": {
                "xcodebuild": executable_available("xcodebuild"),
                "gradlew": gradlew_available(&root),
                "swift": executable_available("swift"),
                "kotlinc": executable_available("kotlinc")
            }
        },
        "native_device": {
            "available": executable_is_available("xcrun") || executable_is_available("adb"),
            "required_tools": {
                "xcrun": executable_available("xcrun"),
                "adb": executable_available("adb")
            }
        }
    }))
    .map_err(|error| format!("preflight_health refused: {error}"))
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

fn call_setup_profile_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let profile = required_string(arguments, "profile")?;
    let action = required_string(arguments, "action")?;
    let params = setup_action_params(action, arguments.get("params"))?;
    let cwd = optional_string(arguments, "cwd")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    let result = setup_profile_run(
        repo_root,
        cwd.map(Path::new),
        profile,
        action,
        params,
        timeout_secs,
        dry_run,
        confirm,
    )
    .map_err(|error| format!("setup_profile_run refused: {error}"))?;
    let summary = result.summary();
    let log_id = write_command_log(&summary)
        .map_err(|error| format!("setup_profile_run log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{summary}"))
}

fn setup_action_params(action: &str, value: Option<&Value>) -> Result<SetupActionParams, String> {
    let params = match value {
        Some(value) => value
            .as_object()
            .ok_or_else(|| "invalid object argument: params".to_string())?,
        None => {
            return match action {
                "cap_sync" => Ok(SetupActionParams::CapSync { platform: None }),
                _ => Ok(SetupActionParams::None),
            };
        }
    };

    match action {
        "cap_init" => Ok(SetupActionParams::CapInit {
            app_id: required_string(params, "app_id")?.to_string(),
            app_name: required_string(params, "app_name")?.to_string(),
            web_dir: required_string(params, "web_dir")?.to_string(),
        }),
        "cap_sync" => {
            let platform = optional_string(params, "platform")?
                .map(parse_capacitor_platform)
                .transpose()?;
            Ok(SetupActionParams::CapSync { platform })
        }
        "install_capacitor_dependencies"
        | "cap_add_ios"
        | "cap_add_android"
        | "ios_pod_install"
            if params.is_empty() =>
        {
            Ok(SetupActionParams::None)
        }
        "install_capacitor_dependencies"
        | "cap_add_ios"
        | "cap_add_android"
        | "ios_pod_install" => Err(format!(
            "setup_profile_run refused: action `{action}` does not accept params"
        )),
        _ => Ok(SetupActionParams::None),
    }
}

fn parse_capacitor_platform(value: &str) -> Result<CapacitorPlatform, String> {
    match value {
        "ios" => Ok(CapacitorPlatform::Ios),
        "android" => Ok(CapacitorPlatform::Android),
        "all" => Ok(CapacitorPlatform::All),
        _ => Err(format!(
            "setup_profile_run refused: unsupported cap_sync platform `{value}`"
        )),
    }
}

fn call_native_build_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let action = required_string(arguments, "action")?;
    let params = native_build_params(action, arguments.get("params"))?;
    let cwd = optional_string(arguments, "cwd")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);

    let result = native_build_run(
        repo_root,
        cwd.map(Path::new),
        action,
        params,
        timeout_secs,
        dry_run,
    )
    .map_err(|error| format!("native_build_run refused: {error}"))?;
    let summary = result.summary();
    let log_id = write_command_log(&summary)
        .map_err(|error| format!("native_build_run log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{summary}"))
}

fn native_build_params(action: &str, value: Option<&Value>) -> Result<NativeBuildParams, String> {
    let params = value
        .and_then(Value::as_object)
        .ok_or_else(|| "missing or invalid object argument: params".to_string())?;
    match action {
        "ios_build" | "ios_test" => Ok(NativeBuildParams::Ios {
            workspace: required_string(params, "workspace")?.to_string(),
            scheme: required_string(params, "scheme")?.to_string(),
            configuration: optional_string(params, "configuration")?.map(ToString::to_string),
            sdk: optional_string(params, "sdk")?.map(ToString::to_string),
            destination: optional_string(params, "destination")?.map(ToString::to_string),
        }),
        "android_assemble_debug" | "android_unit_test" => Ok(NativeBuildParams::Android {
            gradlew: optional_string(params, "gradlew")?.map(ToString::to_string),
        }),
        _ => Ok(NativeBuildParams::Android { gradlew: None }),
    }
}

fn call_native_device_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let action = required_string(arguments, "action")?;
    let params = native_device_params(action, arguments.get("params"))?;
    let cwd = optional_string(arguments, "cwd")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    let result = native_device_run(
        repo_root,
        cwd.map(Path::new),
        action,
        params,
        timeout_secs,
        dry_run,
        confirm,
    )
    .map_err(|error| format!("native_device_run refused: {error}"))?;
    let summary = result.summary();
    let log_id = write_command_log(&summary)
        .map_err(|error| format!("native_device_run log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{summary}"))
}

fn native_device_params(action: &str, value: Option<&Value>) -> Result<NativeDeviceParams, String> {
    let params = match value {
        Some(value) => value
            .as_object()
            .ok_or_else(|| "invalid object argument: params".to_string())?,
        None => {
            return match action {
                "ios_list_simulators" | "android_list_devices" => Ok(NativeDeviceParams::None),
                _ => Err("missing object argument: params".to_string()),
            };
        }
    };
    match action {
        "ios_list_simulators" => {
            if params.is_empty() {
                Ok(NativeDeviceParams::None)
            } else {
                Err(
                    "native_device_run refused: ios_list_simulators does not accept params"
                        .to_string(),
                )
            }
        }
        "ios_boot_simulator" | "ios_read_logs" => Ok(NativeDeviceParams::IosDevice {
            device: required_string(params, "device")?.to_string(),
        }),
        "ios_install_app" => Ok(NativeDeviceParams::IosInstall {
            device: required_string(params, "device")?.to_string(),
            app_path: required_string(params, "app_path")?.to_string(),
        }),
        "ios_launch_app" => Ok(NativeDeviceParams::IosLaunch {
            device: required_string(params, "device")?.to_string(),
            app_id: required_string(params, "app_id")?.to_string(),
        }),
        "android_list_devices" => Ok(NativeDeviceParams::AndroidSerial {
            serial: optional_string(params, "serial")?.map(ToString::to_string),
        }),
        "android_install_app" => Ok(NativeDeviceParams::AndroidInstall {
            serial: optional_string(params, "serial")?.map(ToString::to_string),
            apk_path: required_string(params, "apk_path")?.to_string(),
        }),
        "android_launch_app" => Ok(NativeDeviceParams::AndroidLaunch {
            serial: optional_string(params, "serial")?.map(ToString::to_string),
            app_id: required_string(params, "app_id")?.to_string(),
        }),
        "android_read_logcat" => {
            let lines = optional_u64(params, "lines")?
                .map(|value| {
                    u32::try_from(value)
                        .map_err(|_| "native_device_run refused: lines is too large".to_string())
                })
                .transpose()?;
            Ok(NativeDeviceParams::AndroidLogcat {
                serial: optional_string(params, "serial")?.map(ToString::to_string),
                lines,
            })
        }
        _ => Ok(NativeDeviceParams::None),
    }
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

fn call_git_branch_prepare(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "reset branch from remote base";

    let remote = validate_git_remote(optional_string(arguments, "remote")?.unwrap_or("origin"))?;
    let base_branch = validate_git_branch(required_string(arguments, "base_branch")?)?;
    let branch = validate_git_branch(required_string(arguments, "branch")?)?;
    let required_files = optional_string_array(arguments, "required_files")?;
    let reset_existing = optional_bool(arguments, "reset_existing")?.unwrap_or(false);
    let confirm = optional_string(arguments, "confirm")?;
    if reset_existing && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_branch_prepare refused: reset_existing=true requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = canonical_repo_root(repo_root, tools::git_branch_prepare::NAME)?;
    ensure_remote_exists(tools::git_branch_prepare::NAME, &root, &remote)?;
    let status_before = git_status_short(&root)?;
    if !status_before.trim().is_empty() {
        return Err(format!(
            "git_branch_prepare refused: worktree must be clean before branch preparation\n{}",
            status_before.trim()
        ));
    }

    let remote_ref = remote_ref(&remote, &base_branch);
    git_success_for_tool(
        tools::git_branch_prepare::NAME,
        &root,
        vec![
            "fetch".to_string(),
            remote.clone(),
            format!("refs/heads/{base_branch}:{remote_ref}"),
        ],
    )?;

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

fn call_git_merge_readiness(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let base_ref = validate_git_ref_expression(required_string(arguments, "base_ref")?)?;
    let target_ref = validate_git_ref_expression(required_string(arguments, "target_ref")?)?;
    let fetch = optional_bool(arguments, "fetch")?.unwrap_or(false);
    let root = canonical_repo_root(repo_root, tools::git_merge_readiness::NAME)?;
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

fn optional_string_array(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    match arguments.get(key) {
        Some(value) => {
            let values = value
                .as_array()
                .ok_or_else(|| format!("invalid string array argument: {key}"))?;
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
        None => Ok(Vec::new()),
    }
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
    let entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    for entry in entries {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(format!(
                "git_commit_exact refused: unexpected {label} entry"
            ));
        }
        if matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C') {
            return Err(
                "git_commit_exact refused: rename/copy entries require a future dedicated tool"
                    .to_string(),
            );
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

fn parse_nul_paths_for_tool(
    tool_name: &str,
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            std::str::from_utf8(entry)
                .map(|path| path.to_string())
                .map_err(|error| format!("{tool_name} refused: {label} path is not UTF-8: {error}"))
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

fn current_branch(tool_name: &str, root: &Path) -> Result<String, String> {
    let branch = git_stdout_for_tool(tool_name, root, &["branch", "--show-current"])?;
    let branch = branch.trim();
    if branch.is_empty() {
        Err(format!(
            "{tool_name} refused: repository is in detached HEAD state"
        ))
    } else {
        Ok(branch.to_string())
    }
}

fn local_branch_exists(root: &Path, branch: &str) -> Result<bool, String> {
    let branch_ref = format!("refs/heads/{branch}");
    let status = git_status_code(root, &["show-ref", "--verify", "--quiet", &branch_ref])?;
    match status {
        0 => Ok(true),
        1 => Ok(false),
        code => Err(format!(
            "git workflow refused: failed to check local branch `{branch}` (exit code {code})"
        )),
    }
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

fn validate_git_ref_expression(value: &str) -> Result<String, String> {
    if value == "HEAD" {
        return Ok(value.to_string());
    }
    if (7..=40).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(value.to_ascii_lowercase());
    }

    validate_git_ref_component("ref", value)?;
    if value.contains("..")
        || value.contains('@')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with(".lock")
    {
        return Err(format!("git workflow refused: invalid ref `{value}`"));
    }

    let status = Command::new("git")
        .args(["check-ref-format", "--allow-onelevel", value])
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .map_err(|error| format!("git workflow refused: failed to validate ref: {error}"))?;
    if !status.status.success() {
        return Err(format!("git workflow refused: invalid ref `{value}`"));
    }
    Ok(value.to_string())
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

fn infer_fetch_branch(remote: &str, target_ref: &str) -> Result<String, String> {
    let refs_remotes_prefix = format!("refs/remotes/{remote}/");
    let remote_prefix = format!("{remote}/");
    let branch = if let Some(branch) = target_ref.strip_prefix(&refs_remotes_prefix) {
        branch
    } else if let Some(branch) = target_ref.strip_prefix(&remote_prefix) {
        branch
    } else {
        return Err(format!(
            "git_merge_readiness refused: fetch=true requires target_branch unless target_ref starts with `{remote}/` or `refs/remotes/{remote}/`"
        ));
    };
    validate_git_branch(branch)
}

fn resolve_commit(tool_name: &str, root: &Path, ref_name: &str) -> Result<String, String> {
    let commit_ref = format!("{ref_name}^{{commit}}");
    let commit = git_stdout_for_tool(tool_name, root, &["rev-parse", "--verify", &commit_ref])?;
    Ok(commit.trim().to_string())
}

fn changed_files_between(
    tool_name: &str,
    root: &Path,
    from_ref: &str,
    to_ref: &str,
) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(
        tool_name,
        root,
        &["diff", "--name-only", "-z", from_ref, to_ref, "--"],
    )?;
    parse_nul_paths_for_tool(tool_name, &output.stdout, "git diff --name-only")
}

fn validate_required_file_path(tool_name: &str, root: &Path, raw: &str) -> Result<String, String> {
    validate_required_file_path_syntax(tool_name, raw)?;
    let path = Path::new(raw);
    let candidate = root.join(path);
    if !candidate.is_file() {
        return Err(format!(
            "{tool_name} refused: required file `{raw}` is missing after branch preparation"
        ));
    }
    let resolved = candidate.canonicalize().map_err(|error| {
        format!("{tool_name} refused: failed to resolve required file `{raw}`: {error}")
    })?;
    if !resolved.starts_with(root) {
        return Err(format!(
            "{tool_name} refused: required file `{raw}` resolves outside repository root"
        ));
    }
    Ok(raw.to_string())
}

fn validate_required_files_in_ref(
    tool_name: &str,
    root: &Path,
    ref_name: &str,
    paths: &[String],
) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| {
            validate_required_file_path_syntax(tool_name, path)?;
            let object = format!("{ref_name}:{path}");
            let object_type = git_stdout_for_tool(tool_name, root, &["cat-file", "-t", &object])
                .map_err(|_| {
                    format!(
                        "{tool_name} refused: required file `{path}` is missing from `{ref_name}`"
                    )
                })?;
            if object_type.trim() != "blob" {
                return Err(format!(
                    "{tool_name} refused: required path `{path}` in `{ref_name}` is not a file"
                ));
            }
            Ok(path.to_string())
        })
        .collect()
}

fn validate_required_file_path_syntax(tool_name: &str, raw: &str) -> Result<(), String> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(format!(
            "{tool_name} refused: required file path must not be empty or contain NUL"
        ));
    }
    if raw.contains(':') || raw.contains('*') || raw.contains('?') || raw.contains('[') {
        return Err(format!(
            "{tool_name} refused: required file path `{raw}` contains Git pathspec metacharacters"
        ));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!(
            "{tool_name} refused: required file path `{raw}` must be repository-relative"
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "{tool_name} refused: required file path `{raw}` must be a normalized relative path"
                ))
            }
        }
    }
    Ok(())
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

fn executable_is_available(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn gradlew_available(root: &Path) -> bool {
    root.join("gradlew").is_file()
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
