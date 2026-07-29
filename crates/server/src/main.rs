#![recursion_limit = "256"]

mod protocol;
mod tools;

use std::collections::{hash_map::DefaultHasher, BTreeMap, BTreeSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use contextpatch_core::fs::create_directory::create_directory_in_root;
use contextpatch_core::fs::read_range::read_range_in_root;
use contextpatch_core::fs::write_new_file::{write_new_file_bytes_in_root, write_new_file_in_root};
use contextpatch_core::git::status::{status_summary, status_summary_for_path};
use contextpatch_core::native_build::{native_build_run, NativeBuildParams};
use contextpatch_core::native_device::{native_device_run, NativeDeviceParams};
use contextpatch_core::patch::diff::preview_exact_replacement_in_root;
use contextpatch_core::process::guarded_command::run_guarded_command;
use contextpatch_core::replace::exact::replace_exact_in_root;
use contextpatch_core::setup::profile::{setup_profile_run, CapacitorPlatform, SetupActionParams};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

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
            "name": tools::write_new_file_base64::NAME,
            "description": "Create a new binary file from base64 only when the destination does not already exist.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path relative to the configured repository root. Parent directory must already exist."
                    },
                    "content_base64": {
                        "type": "string",
                        "description": "Base64-encoded file content to write."
                    },
                    "expected_bytes": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 20971520,
                        "description": "Optional decoded byte count guard."
                    }
                },
                "required": ["path", "content_base64"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::write_existing_file_exact_hash::NAME,
            "description": "Overwrite an existing repository file only when the current SHA-256 hash matches the caller's expectation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Existing file path relative to the configured repository root."
                    },
                    "content": {
                        "type": "string",
                        "description": "Full UTF-8 file content to write."
                    },
                    "expected_sha256": {
                        "type": "string",
                        "description": "Lowercase SHA-256 hex digest of the current file content."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Validate and preview without writing. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `write exact hash` when dry_run is false."
                    }
                },
                "required": ["path", "content", "expected_sha256"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::artifact_write_text::NAME,
            "description": "Create a new UTF-8 text artifact outside the repository under the fixed contextpatch artifact directory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Artifact path relative to the fixed artifact root. Parent directory must already exist unless parents is true."
                    },
                    "content": {
                        "type": "string",
                        "description": "Full artifact content to write."
                    },
                    "parents": {
                        "type": "boolean",
                        "description": "When true, create missing parent directories under the artifact root. Defaults to false."
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::artifact_write_base64::NAME,
            "description": "Create a new binary artifact outside the repository under the fixed contextpatch artifact directory.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Artifact path relative to the fixed artifact root. Parent directory must already exist unless parents is true."
                    },
                    "content_base64": {
                        "type": "string",
                        "description": "Base64-encoded artifact content to write."
                    },
                    "expected_bytes": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 20971520,
                        "description": "Optional decoded byte count guard."
                    },
                    "parents": {
                        "type": "boolean",
                        "description": "When true, create missing parent directories under the artifact root. Defaults to false."
                    }
                },
                "required": ["path", "content_base64"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::bulk_write_new_files_base64::NAME,
            "description": "Create many new repository files from base64 entries in one bounded, create-only fixture import.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entries": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 500,
                        "items": {
                            "type": "object",
                            "properties": {
                                "path": {
                                    "type": "string",
                                    "description": "Repository-relative destination path. Existing files are refused."
                                },
                                "content_base64": {
                                    "type": "string",
                                    "description": "Base64-encoded file content."
                                },
                                "expected_bytes": {
                                    "type": "integer",
                                    "minimum": 0,
                                    "maximum": 20971520,
                                    "description": "Optional decoded byte count guard for this entry."
                                }
                            },
                            "required": ["path", "content_base64"],
                            "additionalProperties": false
                        },
                        "description": "Files to create. Total decoded size is limited to 20 MiB."
                    },
                    "parents": {
                        "type": "boolean",
                        "description": "When true, create missing parent directories inside the repository root. Defaults to false."
                    }
                },
                "required": ["entries"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::create_directory::NAME,
            "description": "Create a new directory only when the destination does not already exist.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path relative to the configured repository root."
                    },
                    "parents": {
                        "type": "boolean",
                        "description": "When true, create missing parent directories inside the repository root. Defaults to false."
                    }
                },
                "required": ["path"],
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
                        "description": "Allowlisted executable name: git, cargo, bun, npm, pnpm, python/python3, pytest, harbor, bash for the exact base-image script, or rg."
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
            "name": tools::fixture_generator_run::NAME,
            "description": "Plan or run a repo-relative Python fixture generator that may mutate only declared output paths or prefixes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "script_path": {
                        "type": "string",
                        "description": "Repository-relative .py script to run with python3. The script itself may be the only pre-existing dirty path unless allowed_existing_dirty_paths is provided."
                    },
                    "args": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Arguments passed after the script path. Use only explicit argv strings; no shell interpolation is performed."
                    },
                    "cwd": {
                        "type": "string",
                        "description": "Optional working directory relative to the repository root. Defaults to the repository root."
                    },
                    "expected_output_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Exact repository-relative paths the generator may create or modify."
                    },
                    "expected_output_prefixes": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Repository-relative directory prefixes under which the generator may create or modify files."
                    },
                    "allowed_existing_dirty_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Pre-existing dirty paths allowed before the run. Defaults to the script path only, supporting temporary untracked generators that are deleted before commit."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "description": "Optional timeout in seconds. Defaults to 120, maximum 600."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Plan only without running Python. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `run fixture generator` when dry_run is false."
                    }
                },
                "required": ["script_path"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::base_image_check_run::NAME,
            "description": "Plan or run the exact repository base-image check script references/check-base-image.sh without enabling arbitrary shell access.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_path": {
                        "type": "string",
                        "description": "Optional repository-relative project directory argument. For Dynamo/Harbor tasks this is exactly `task`."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "description": "Optional timeout in seconds. Defaults to 120, maximum 600."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Plan only without running the script. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `run base image check` when dry_run is false."
                    }
                },
                "additionalProperties": false
            }
        },
        {
            "name": tools::fixture_manifest_verify::NAME,
            "description": "Verify a fixture manifest's exact file set and SHA-256 digests against declared repository fixture paths or prefixes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "manifest_path": {
                        "type": "string",
                        "description": "Repository-relative manifest JSON path."
                    },
                    "fixture_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Exact fixture files to include in the file-set check."
                    },
                    "fixture_prefixes": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Repository-relative directories or file prefixes whose regular files must exactly match the manifest."
                    }
                },
                "required": ["manifest_path"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::fixture_manifest_refresh::NAME,
            "description": "Regenerate a fixture manifest JSON from declared repository fixture paths or prefixes, guarded by dry-run, confirmation, and existing manifest hash.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "manifest_path": {
                        "type": "string",
                        "description": "Repository-relative manifest JSON path to create or overwrite."
                    },
                    "fixture_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Exact fixture files to include."
                    },
                    "fixture_prefixes": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Repository-relative directories or file prefixes whose regular files are included."
                    },
                    "expected_manifest_sha256": {
                        "type": "string",
                        "description": "Required when overwriting an existing manifest; must match its current SHA-256."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Preview the generated manifest metadata without writing. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `refresh fixture manifest` when dry_run is false."
                    }
                },
                "required": ["manifest_path"],
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
                        "description": "Validation profile name: repo-basic, rust-workspace, datacore-vscode, datacore-m6-vscode, or dynamo-harbor-task."
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
                        "description": "Profile action, such as install_capacitor_dependencies, install_capacitor_filesystem, cap_init, cap_add_ios, cap_add_android, cap_sync, or ios_pod_install."
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
                        "description": "Typed action parameters. iOS uses workspace, scheme, optional configuration/sdk/destination/derived_data_path. Android accepts optional gradlew path."
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
                        "description": "Native device action such as ios_list_simulators, ios_create_simulator, ios_boot_simulator, ios_cap_run, ios_install_app, ios_launch_app, ios_read_logs, android_list_devices, android_install_app, android_launch_app, or android_read_logcat."
                    },
                    "params": {
                        "type": "object",
                        "description": "Typed action parameters such as device, serial, app_id, app_path, apk_path, Android lines, or iOS log duration."
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
            "name": tools::git_commit_scoped::NAME,
            "description": "Dry-run or create one local Git commit from an explicit subset of dirty paths while preserving unrelated dirty files. Never pushes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Repository-relative dirty paths to stage and commit. Other dirty paths are preserved."
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
                        "description": "Required literal value `commit scoped paths` when dry_run is false."
                    }
                },
                "required": ["paths", "subject"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::git_restore_exact::NAME,
            "description": "Dry-run or restore exact dirty repository paths from HEAD. Use for generated noise cleanup before exact commits; never resets the whole worktree.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Repository-relative dirty paths to restore from HEAD. Every path must currently be dirty."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Validate and preview without restoring. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `restore exact paths` when dry_run is false."
                    }
                },
                "required": ["paths"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::delete_untracked_exact::NAME,
            "description": "Dry-run or delete explicit untracked files only. Refuses tracked files, directories, globs, and broad cleanup.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": {
                            "type": "string"
                        },
                        "minItems": 1,
                        "description": "Repository-relative untracked file paths to delete."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Validate and preview without deleting. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `delete untracked files` when dry_run is false."
                    }
                },
                "required": ["paths"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::git_remote_list::NAME,
            "description": "Read-only Git remote inspection, equivalent to a parsed git remote -v.",
            "inputSchema": {
                "type": "object",
                "properties": {},
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
        },
        {
            "name": tools::github_pr_run::NAME,
            "description": "Run narrow GitHub PR workflows through gh: auth status, PR view/checks, or confirmation-gated PR creation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["auth_status", "pr_view", "pr_checks", "pr_create"],
                        "description": "GitHub workflow action."
                    },
                    "number": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Pull request number for pr_view or pr_checks."
                    },
                    "base": {
                        "type": "string",
                        "description": "Base branch for pr_create."
                    },
                    "head": {
                        "type": "string",
                        "description": "Head branch for pr_create."
                    },
                    "title": {
                        "type": "string",
                        "description": "Pull request title for pr_create."
                    },
                    "body": {
                        "type": "string",
                        "description": "Pull request body for pr_create."
                    },
                    "draft": {
                        "type": "boolean",
                        "description": "Create a draft pull request. Defaults to false."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "For pr_create, default true. When true, returns the gh command plan without mutating GitHub."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `create pull request` when dry_run is false for pr_create."
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            }
        },
        {
            "name": tools::github_fork_prepare::NAME,
            "description": "Plan or run a guarded gh repo fork workflow for the current repository.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "remote": {
                        "type": "boolean",
                        "description": "Pass --remote to add fork remotes. Defaults to true."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "Plan only without mutating GitHub or remotes. Defaults to true."
                    },
                    "confirm": {
                        "type": "string",
                        "description": "Required literal value `prepare github fork` when dry_run is false."
                    }
                },
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
        tools::write_new_file_base64::NAME => call_write_new_file_base64(repo_root, &arguments),
        tools::write_existing_file_exact_hash::NAME => {
            call_write_existing_file_exact_hash(repo_root, &arguments)
        }
        tools::artifact_write_text::NAME => call_artifact_write_text(repo_root, &arguments),
        tools::artifact_write_base64::NAME => call_artifact_write_base64(repo_root, &arguments),
        tools::bulk_write_new_files_base64::NAME => {
            call_bulk_write_new_files_base64(repo_root, &arguments)
        }
        tools::create_directory::NAME => call_create_directory(repo_root, &arguments),
        tools::run_guarded_command::NAME => call_run_guarded_command(repo_root, &arguments),
        tools::fixture_generator_run::NAME => call_fixture_generator_run(repo_root, &arguments),
        tools::base_image_check_run::NAME => call_base_image_check_run(repo_root, &arguments),
        tools::fixture_manifest_verify::NAME => call_fixture_manifest_verify(repo_root, &arguments),
        tools::fixture_manifest_refresh::NAME => {
            call_fixture_manifest_refresh(repo_root, &arguments)
        }
        tools::read_command_log::NAME => call_read_command_log(&arguments),
        tools::validation_profile_run::NAME => call_validation_profile_run(repo_root, &arguments),
        tools::setup_profile_run::NAME => call_setup_profile_run(repo_root, &arguments),
        tools::native_build_run::NAME => call_native_build_run(repo_root, &arguments),
        tools::native_device_run::NAME => call_native_device_run(repo_root, &arguments),
        tools::git_commit_exact::NAME => call_git_commit_exact(repo_root, &arguments),
        tools::git_commit_scoped::NAME => call_git_commit_scoped(repo_root, &arguments),
        tools::git_restore_exact::NAME => call_git_restore_exact(repo_root, &arguments),
        tools::delete_untracked_exact::NAME => call_delete_untracked_exact(repo_root, &arguments),
        tools::git_remote_list::NAME => call_git_remote_list(repo_root),
        tools::git_remote_check::NAME => call_git_remote_check(repo_root, &arguments),
        tools::git_branch_prepare::NAME => call_git_branch_prepare(repo_root, &arguments),
        tools::git_merge_readiness::NAME => call_git_merge_readiness(repo_root, &arguments),
        tools::git_push_exact::NAME => call_git_push_exact(repo_root, &arguments),
        tools::github_pr_run::NAME => call_github_pr_run(repo_root, &arguments),
        tools::github_fork_prepare::NAME => call_github_fork_prepare(repo_root, &arguments),
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
            "write_new_file_base64": true,
            "write_existing_file_exact_hash": true,
            "artifact_write_text": true,
            "artifact_write_base64": true,
            "create_directory": true,
            "status_guard": true,
            "read_command_log": true,
            "setup_profile_run": true,
            "native_build_run": true,
            "native_device_run": true,
            "git_commit_exact": true,
            "git_commit_scoped": true,
            "git_restore_exact": true,
            "delete_untracked_exact": true,
            "git_remote_list": true,
            "git_remote_check": true,
            "git_branch_prepare": true,
            "git_merge_readiness": true,
            "git_push_exact": true,
            "github_pr_run": true,
            "github_fork_prepare": true
            ,
            "bulk_write_new_files_base64": true,
            "fixture_generator_run": true,
            "base_image_check_run": true
            ,
            "fixture_manifest_verify": true,
            "fixture_manifest_refresh": true
        },
        "git_workflows": {
            "local_commit_exact_paths": true,
            "local_commit_scoped_paths": true,
            "restore_exact_paths": true,
            "delete_untracked_exact_paths": true,
            "remote_list": true,
            "remote_check": true,
            "branch_prepare": true,
            "merge_readiness": true,
            "push_exact_commit": true,
            "reset_checkout_clean_stash": false,
            "commit_attribution": "Do not include Claude, Anthropic, AI, or assistant attribution in commit messages. Use the repository user's authorship and project-owned commit message only.",
            "guards": [
                "requires exact complete dirty-path set",
                "scoped commits require requested dirty paths and a clean index before staging",
                "defaults to dry_run",
                "requires confirm literal for mutation",
                "restores only explicit dirty paths from HEAD",
                "stages only explicit paths",
                "creates at most one local commit",
                "fetches only explicit remote branch",
                "branch preparation requires clean worktree and validates required files",
                "merge readiness is read-only and reports changed-on-both-sides candidates",
                "pushes only expected HEAD to matching remote branch",
                "never force pushes"
            ],
            "examples": [
                {
                    "tool": "git_restore_exact",
                    "description": "Restore generated dirty tracked paths before making an exact scoped commit.",
                    "arguments": {
                        "paths": ["data/processed/example.json"],
                        "dry_run": true
                    }
                }
            ]
        },
        "github_workflows": {
            "available": true,
            "tools": [tools::github_pr_run::NAME, tools::github_fork_prepare::NAME],
            "actions": ["auth_status", "pr_view", "pr_checks", "pr_create"],
            "fork_actions": ["dry_run fork plan", "confirmed gh repo fork"],
            "guards": [
                "uses gh with explicit argv only",
                "pr_create defaults to dry_run",
                "pr_create requires confirm literal when mutating",
                "fork prepare defaults to dry_run",
                "fork prepare requires confirm literal when mutating",
                "no arbitrary gh passthrough"
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
                "pnpm": ["run", "test"],
                "python": ["repo-relative .py script"],
                "python3": ["repo-relative .py script"],
                "pytest": ["validation invocation"],
                "harbor": ["run"],
                "bash": ["references/check-base-image.sh", "references/check-base-image.sh task"],
                "rg": ["search"]
            },
            "typed_workflows": {
                "fixture_generator_run": "Runs repo-relative Python generators after planning, confirmation, and declared-output verification.",
                "base_image_check_run": "Runs only references/check-base-image.sh, optionally with the exact task project argument; arbitrary shell scripts remain unsupported.",
                "bulk_write_new_files_base64": "Imports many create-only binary/text fixture files with per-file and total size bounds.",
                "write_existing_file_exact_hash": "Overwrites an existing file only when the current SHA-256 matches.",
                "fixture_manifest_verify": "Verifies exact fixture file sets and SHA-256 digests.",
                "fixture_manifest_refresh": "Regenerates fixture manifests with dry-run, confirmation, and existing-manifest hash guard."
            },
            "validation_profiles": ["repo-basic", "rust-workspace", "datacore-vscode", "datacore-m6-vscode", "dynamo-harbor-task"],
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
                        "install_capacitor_filesystem",
                        "cap_init",
                        "cap_add_ios",
                        "cap_add_android",
                        "cap_sync",
                        "ios_pod_install"
                    ],
                    "external_mutator": true,
                    "caller_supplies_raw_command": false,
                    "ios_pod_install_requirement": "requires Podfile in cwd; Swift Package Manager based Capacitor projects do not need this action",
                    "supported_package_managers": ["npm", "pnpm"],
                    "package_manager_selection": "uses pnpm when pnpm-lock.yaml exists in cwd; otherwise npm",
                    "dependency_layout": {
                        "dependencies": ["@capacitor/core", "@capacitor/ios", "@capacitor/android"],
                        "optional_runtime_dependencies_by_action": {
                            "install_capacitor_filesystem": ["@capacitor/filesystem"]
                        },
                        "dev_dependencies": ["@capacitor/cli"]
                    },
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
                    "description": "Install the Capacitor Filesystem plugin needed to pass fetched local files into native plugins.",
                    "arguments": {
                        "profile": "node-capacitor-shell",
                        "action": "install_capacitor_filesystem",
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
                "package-manager commands stay inside setup profiles, not run_guarded_command",
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
                "ios_build": {
                    "program": "xcodebuild",
                    "caller_supplies_raw_command": false,
                    "supports_repo_relative_derived_data_path": true
                },
                "ios_test": {
                    "program": "xcodebuild",
                    "caller_supplies_raw_command": false,
                    "supports_repo_relative_derived_data_path": true
                },
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
                            "scheme": "App",
                            "derived_data_path": ".contextpatch-derived-data"
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
                "ios_create_simulator",
                "ios_boot_simulator",
                "ios_cap_run",
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
                    "description": "Plan an iOS app install from repo-relative Xcode DerivedData output.",
                    "arguments": {
                        "action": "ios_install_app",
                        "params": {
                            "device": "booted",
                            "app_path": ".contextpatch-derived-data/Build/Products/Debug-iphonesimulator/App.app"
                        },
                        "dry_run": true
                    }
                },
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
                    "description": "Create a named iOS simulator when runtimes exist but no devices are configured.",
                    "arguments": {
                        "action": "ios_create_simulator",
                        "params": {
                            "name": "ContextPatch iPhone",
                            "device_type": "iPhone 16",
                            "runtime": "iOS 26.4"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_device_run",
                    "description": "Run a synced Capacitor iOS app on a specific simulator target without raw npx/pnpm/xcrun commands.",
                    "arguments": {
                        "action": "ios_cap_run",
                        "params": {
                            "target": "00000000-0000-0000-0000-000000000000"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_device_run",
                    "description": "Read a timed iOS simulator log stream without starting an unbounded stream.",
                    "arguments": {
                        "action": "ios_read_logs",
                        "params": {
                            "device": "booted",
                            "duration": "5"
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
                "no arbitrary xcrun, adb, or package-manager exec",
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
                "available": executable_is_available("npm") || executable_is_available("pnpm"),
                "package_managers": {
                    "npm": executable_available("npm"),
                    "pnpm": executable_available("pnpm")
                },
                "optional_tools": {
                    "pod": executable_available("pod")
                },
                "mutation_enabled": true
            }
        },
        "native_build": {
            "available": xcodebuild_is_available() || gradlew_available(&root),
            "required_tools": {
                "xcodebuild": xcodebuild_available(),
                "xcode_select": xcode_select_available(),
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

fn call_write_new_file_base64(
    repo_root: &Path,
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

    let summary = write_new_file_bytes_in_root(repo_root, Path::new(path), &bytes)
        .map_err(|error| format!("write_new_file_base64 refused: {error}"))?;

    Ok(format!(
        "created {} ({} bytes written from base64)",
        summary.path.display(),
        summary.bytes_written
    ))
}

fn call_write_existing_file_exact_hash(
    repo_root: &Path,
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

    let root = canonical_repo_root(repo_root, tools::write_existing_file_exact_hash::NAME)?;
    let normalized =
        normalize_repo_relative_path(tools::write_existing_file_exact_hash::NAME, path)?;
    let target = root.join(&normalized);
    if !target.is_file() {
        return Err(format!(
            "write_existing_file_exact_hash refused: `{normalized}` is not an existing regular file"
        ));
    }
    let current = fs::read(&target).map_err(|error| {
        format!("write_existing_file_exact_hash refused: failed to read `{normalized}`: {error}")
    })?;
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

    let temporary = target.with_extension(format!(
        "contextpatch-tmp-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("write_existing_file_exact_hash refused: {error}"))?
            .as_nanos()
    ));
    fs::write(&temporary, new_bytes).map_err(|error| {
        format!("write_existing_file_exact_hash refused: failed to write temporary file: {error}")
    })?;
    fs::rename(&temporary, &target).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!("write_existing_file_exact_hash refused: failed to replace `{normalized}`: {error}")
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
}

fn call_artifact_write_text(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let content = required_string(arguments, "content")?;
    let parents = optional_bool(arguments, "parents")?.unwrap_or(false);
    write_artifact(
        repo_root,
        path,
        content.as_bytes(),
        parents,
        tools::artifact_write_text::NAME,
    )
}

fn call_artifact_write_base64(
    repo_root: &Path,
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
        repo_root,
        path,
        &bytes,
        parents,
        tools::artifact_write_base64::NAME,
    )
}

fn call_bulk_write_new_files_base64(
    repo_root: &Path,
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
    let root = canonical_repo_root(repo_root, tools::bulk_write_new_files_base64::NAME)?;

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
        let target = root.join(&normalized);
        if target.exists() {
            return Err(format!(
                "bulk_write_new_files_base64 refused: target already exists: {normalized}"
            ));
        }
        if !parents {
            let parent = target.parent().ok_or_else(|| {
                format!("bulk_write_new_files_base64 refused: path `{normalized}` has no parent")
            })?;
            if !parent.is_dir() {
                return Err(format!(
                    "bulk_write_new_files_base64 refused: parent does not exist for `{normalized}`"
                ));
            }
        }
        decoded_entries.push((normalized, bytes));
    }

    for (path, _) in &decoded_entries {
        if parents {
            let target = root.join(path);
            let parent = target.parent().ok_or_else(|| {
                format!("bulk_write_new_files_base64 refused: path `{path}` has no parent")
            })?;
            fs::create_dir_all(parent).map_err(|error| {
                format!("bulk_write_new_files_base64 refused: failed to create parents: {error}")
            })?;
        }
    }

    let mut created = Vec::with_capacity(decoded_entries.len());
    for (path, bytes) in decoded_entries {
        let summary = write_new_file_bytes_in_root(&root, Path::new(&path), &bytes)
            .map_err(|error| format!("bulk_write_new_files_base64 refused: {error}"))?;
        created.push(json!({
            "path": summary.path.display().to_string(),
            "bytes_written": summary.bytes_written
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

fn write_artifact(
    repo_root: &Path,
    path: &str,
    bytes: &[u8],
    parents: bool,
    tool_name: &str,
) -> Result<String, String> {
    let artifact_root = artifact_root(repo_root, tool_name)?;
    let relative = validate_relative_path(tool_name, path)?;
    let target = artifact_root.join(&relative);
    if parents {
        let parent = target
            .parent()
            .ok_or_else(|| format!("{tool_name} refused: artifact path has no parent"))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("{tool_name} refused: failed to create parents: {error}"))?;
    }
    let summary = write_new_file_bytes_in_root(&artifact_root, &relative, bytes)
        .map_err(|error| format!("{tool_name} refused: {error}"))?;

    serde_json::to_string_pretty(&json!({
        "tool": tool_name,
        "created": true,
        "artifact_root": artifact_root.display().to_string(),
        "path": summary.path.display().to_string(),
        "bytes_written": summary.bytes_written,
        "repo_mutation": false
    }))
    .map_err(|error| format!("{tool_name} refused: {error}"))
}

fn call_fixture_generator_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "run fixture generator";

    let script_path = required_string(arguments, "script_path")?;
    if !script_path.ends_with(".py") {
        return Err("fixture_generator_run refused: script_path must end with .py".to_string());
    }
    let args = optional_string_array(arguments, "args")?;
    let exact_outputs = optional_string_array(arguments, "expected_output_paths")?;
    let output_prefixes = optional_string_array(arguments, "expected_output_prefixes")?;
    if exact_outputs.is_empty() && output_prefixes.is_empty() {
        return Err("fixture_generator_run refused: at least one expected output path or prefix is required".to_string());
    }
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(120);
    let cwd = optional_string(arguments, "cwd")?;
    let root = canonical_repo_root(repo_root, tools::fixture_generator_run::NAME)?;
    let script = normalize_git_path(tools::fixture_generator_run::NAME, &root, script_path)?;
    if !root.join(&script).is_file() {
        return Err(format!(
            "fixture_generator_run refused: script_path `{script}` is not a file"
        ));
    }
    let exact_outputs =
        normalize_repo_relative_paths(tools::fixture_generator_run::NAME, &exact_outputs)?
            .into_iter()
            .collect::<BTreeSet<_>>();
    let output_prefixes =
        normalize_repo_relative_paths(tools::fixture_generator_run::NAME, &output_prefixes)?;
    let allowed_existing_dirty = if arguments.contains_key("allowed_existing_dirty_paths") {
        normalize_git_paths(
            tools::fixture_generator_run::NAME,
            &root,
            &required_string_array(arguments, "allowed_existing_dirty_paths")?,
        )?
    } else {
        vec![script.clone()]
    }
    .into_iter()
    .collect::<BTreeSet<_>>();

    let before = git_status_paths_for_tool(tools::fixture_generator_run::NAME, &root)?;
    let unexpected_before = before
        .difference(&allowed_existing_dirty)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !unexpected_before.is_empty() {
        return Err(format!(
            "fixture_generator_run refused: worktree has dirty paths outside allowed_existing_dirty_paths before generator run\n{}",
            format_set(&unexpected_before)
        ));
    }

    let mut command_args = Vec::with_capacity(args.len() + 1);
    command_args.push(script.clone());
    command_args.extend(args);

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::fixture_generator_run::NAME,
            "dry_run": true,
            "would_run": {
                "program": "python3",
                "args": command_args,
                "cwd": cwd.unwrap_or("."),
                "timeout_secs": timeout_secs
            },
            "allowed_existing_dirty_paths": allowed_existing_dirty,
            "expected_output_paths": exact_outputs,
            "expected_output_prefixes": output_prefixes,
            "confirm_required": CONFIRMATION
        }))
        .map_err(|error| format!("fixture_generator_run refused: {error}"));
    }
    let confirm = optional_string(arguments, "confirm")?;
    let Some(confirm) = confirm else {
        return Err(format!(
            "fixture_generator_run refused: confirm must be {CONFIRMATION:?}"
        ));
    };
    if confirm != CONFIRMATION {
        return Err(format!(
            "fixture_generator_run refused: confirm must be {CONFIRMATION:?}"
        ));
    }

    let output = run_guarded_command(
        &root,
        cwd.map(Path::new),
        "python3",
        &command_args,
        Some(timeout_secs),
    )
    .map_err(|error| format!("fixture_generator_run refused: {error}"))?;
    if !guarded_output_succeeded(&output) {
        return Err(format!(
            "fixture_generator_run refused: generator command failed\n{output}"
        ));
    }

    let after = git_status_paths_for_tool(tools::fixture_generator_run::NAME, &root)?;
    let unauthorized = after
        .iter()
        .filter(|path| {
            !allowed_existing_dirty.contains(*path)
                && !exact_outputs.contains(*path)
                && !matches_any_prefix(path, &output_prefixes)
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    if !unauthorized.is_empty() {
        return Err(format!(
            "fixture_generator_run refused: generator changed paths outside declared outputs\n{}",
            format_set(&unauthorized)
        ));
    }

    let output_changes = after
        .iter()
        .filter(|path| exact_outputs.contains(*path) || matches_any_prefix(path, &output_prefixes))
        .cloned()
        .collect::<BTreeSet<_>>();

    serde_json::to_string_pretty(&json!({
        "tool": tools::fixture_generator_run::NAME,
        "ran": true,
        "program": "python3",
        "script_path": script,
        "changed_output_paths": output_changes,
        "status_paths_after": after,
        "command_output": output
    }))
    .map_err(|error| format!("fixture_generator_run refused: {error}"))
}

fn call_base_image_check_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "run base image check";
    const SCRIPT: &str = "references/check-base-image.sh";

    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(120);
    let project_path = optional_string(arguments, "project_path")?;
    let command_args = base_image_check_args(project_path)?;
    let root = canonical_repo_root(repo_root, tools::base_image_check_run::NAME)?;
    if !root.join(SCRIPT).is_file() {
        return Err(format!(
            "base_image_check_run refused: required script `{SCRIPT}` is not present"
        ));
    }
    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::base_image_check_run::NAME,
            "dry_run": true,
            "would_run": {
                "program": "bash",
                "args": command_args,
                "cwd": ".",
                "timeout_secs": timeout_secs
            },
            "scope": "exact base-image check script only, optionally with the exact task project argument; arbitrary shell remains unsupported",
            "confirm_required": CONFIRMATION
        }))
        .map_err(|error| format!("base_image_check_run refused: {error}"));
    }
    let confirm = optional_string(arguments, "confirm")?;
    let Some(confirm) = confirm else {
        return Err(format!(
            "base_image_check_run refused: confirm must be {CONFIRMATION:?}"
        ));
    };
    if confirm != CONFIRMATION {
        return Err(format!(
            "base_image_check_run refused: confirm must be {CONFIRMATION:?}"
        ));
    }

    let output = run_guarded_command(&root, None, "bash", &command_args, Some(timeout_secs))
        .map_err(|error| format!("base_image_check_run refused: {error}"))?;
    if !guarded_output_succeeded(&output) {
        return Err(format!(
            "base_image_check_run refused: check command failed\n{output}"
        ));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::base_image_check_run::NAME,
        "ran": true,
        "script": SCRIPT,
        "args": command_args,
        "command_output": output
    }))
    .map_err(|error| format!("base_image_check_run refused: {error}"))
}

fn base_image_check_args(project_path: Option<&str>) -> Result<Vec<String>, String> {
    const SCRIPT: &str = "references/check-base-image.sh";
    match project_path {
        Some("task") => Ok(vec![SCRIPT.to_string(), "task".to_string()]),
        Some(other) => Err(format!(
            "base_image_check_run refused: project_path must be exactly `task` when provided, got `{other}`"
        )),
        None => Ok(vec![SCRIPT.to_string()]),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_sha256_hex(tool_name: &str, value: &str) -> Result<String, String> {
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!(
            "{tool_name} refused: SHA-256 digest must be 64 hexadecimal characters"
        ));
    }
    if value.chars().any(|ch| ch.is_ascii_uppercase()) {
        return Err(format!(
            "{tool_name} refused: SHA-256 digest must be lowercase hex"
        ));
    }
    Ok(value.to_string())
}

fn parse_fixture_manifest(
    tool_name: &str,
    value: &Value,
) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    match value {
        Value::Object(object) => {
            if let Some(files) = object.get("files") {
                let files = files.as_array().ok_or_else(|| {
                    format!("{tool_name} refused: manifest files must be an array")
                })?;
                for (index, entry) in files.iter().enumerate() {
                    let entry = entry.as_object().ok_or_else(|| {
                        format!("{tool_name} refused: manifest files[{index}] must be an object")
                    })?;
                    let path = entry.get("path").and_then(Value::as_str).ok_or_else(|| {
                        format!("{tool_name} refused: manifest files[{index}].path is required")
                    })?;
                    let sha = entry
                        .get("sha256")
                        .or_else(|| entry.get("digest"))
                        .or_else(|| entry.get("sha256_digest"))
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            format!(
                                "{tool_name} refused: manifest files[{index}] needs sha256 or digest"
                            )
                        })?;
                    let normalized = normalize_repo_relative_path(tool_name, path)?;
                    let sha = validate_sha256_hex(tool_name, sha)?;
                    if entries.insert(normalized.clone(), sha).is_some() {
                        return Err(format!(
                            "{tool_name} refused: manifest has duplicate path `{normalized}`"
                        ));
                    }
                }
            } else {
                for (path, digest) in object {
                    let digest = digest.as_str().ok_or_else(|| {
                        format!(
                            "{tool_name} refused: manifest object values must be SHA-256 strings"
                        )
                    })?;
                    let normalized = normalize_repo_relative_path(tool_name, path)?;
                    let digest = validate_sha256_hex(tool_name, digest)?;
                    if entries.insert(normalized.clone(), digest).is_some() {
                        return Err(format!(
                            "{tool_name} refused: manifest has duplicate path `{normalized}`"
                        ));
                    }
                }
            }
        }
        _ => {
            return Err(format!(
                "{tool_name} refused: manifest root must be an object"
            ))
        }
    }
    if entries.is_empty() {
        return Err(format!(
            "{tool_name} refused: manifest must contain at least one file"
        ));
    }
    Ok(entries)
}

fn collect_fixture_files_from_args(
    tool_name: &str,
    root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<BTreeMap<String, String>, String> {
    let fixture_paths = optional_string_array(arguments, "fixture_paths")?;
    let fixture_prefixes = optional_string_array(arguments, "fixture_prefixes")?;
    if fixture_paths.is_empty() && fixture_prefixes.is_empty() {
        return Err(format!(
            "{tool_name} refused: at least one fixture_paths or fixture_prefixes entry is required"
        ));
    }

    let mut paths = BTreeSet::new();
    for path in normalize_repo_relative_paths(tool_name, &fixture_paths)? {
        let target = root.join(&path);
        if !target.is_file() {
            return Err(format!(
                "{tool_name} refused: fixture path `{path}` is not a regular file"
            ));
        }
        paths.insert(path);
    }
    for prefix in normalize_repo_relative_paths(tool_name, &fixture_prefixes)? {
        let target = root.join(&prefix);
        if target.is_file() {
            paths.insert(prefix);
        } else if target.is_dir() {
            collect_regular_files(tool_name, root, &target, &mut paths)?;
        } else {
            return Err(format!(
                "{tool_name} refused: fixture prefix `{prefix}` is not a file or directory"
            ));
        }
    }

    let mut result = BTreeMap::new();
    for path in paths {
        let target = root.join(&path);
        let bytes = fs::read(&target)
            .map_err(|error| format!("{tool_name} refused: failed to read `{path}`: {error}"))?;
        result.insert(path, sha256_hex(&bytes));
    }
    Ok(result)
}

fn collect_regular_files(
    tool_name: &str,
    root: &Path,
    directory: &Path,
    paths: &mut BTreeSet<String>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("{tool_name} refused: failed to read directory: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("{tool_name} refused: failed to read directory entry: {error}"))?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "{tool_name} refused: failed to inspect `{}`: {error}",
                path.display()
            )
        })?;
        if file_type.is_symlink() {
            return Err(format!(
                "{tool_name} refused: fixture path `{}` is a symlink",
                path.display()
            ));
        }
        if file_type.is_dir() {
            collect_regular_files(tool_name, root, &path, paths)?;
        } else if file_type.is_file() {
            paths.insert(repo_relative_string(tool_name, root, &path)?);
        }
    }
    Ok(())
}

fn repo_relative_string(tool_name: &str, root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(root).map_err(|error| {
        format!(
            "{tool_name} refused: path `{}` is outside repository root: {error}",
            path.display()
        )
    })?;
    let parts = relative
        .components()
        .map(|component| match component {
            std::path::Component::Normal(value) => Ok(value.to_string_lossy().into_owned()),
            _ => Err(format!(
                "{tool_name} refused: collected path must be normalized relative"
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

fn call_fixture_manifest_verify(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let manifest_path = required_string(arguments, "manifest_path")?;
    let root = canonical_repo_root(repo_root, tools::fixture_manifest_verify::NAME)?;
    let manifest_normalized =
        normalize_repo_relative_path(tools::fixture_manifest_verify::NAME, manifest_path)?;
    let manifest_target = root.join(&manifest_normalized);
    if !manifest_target.is_file() {
        return Err(format!(
            "fixture_manifest_verify refused: manifest `{manifest_normalized}` is not an existing file"
        ));
    }
    let manifest_bytes = fs::read(&manifest_target).map_err(|error| {
        format!("fixture_manifest_verify refused: failed to read `{manifest_normalized}`: {error}")
    })?;
    let manifest_value: Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        format!("fixture_manifest_verify refused: manifest JSON is invalid: {error}")
    })?;
    let expected = parse_fixture_manifest(tools::fixture_manifest_verify::NAME, &manifest_value)?;
    let actual =
        collect_fixture_files_from_args(tools::fixture_manifest_verify::NAME, &root, arguments)?;
    let expected_paths = expected.keys().cloned().collect::<BTreeSet<_>>();
    let actual_paths = actual.keys().cloned().collect::<BTreeSet<_>>();
    let missing_files = expected_paths
        .difference(&actual_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    let unlisted_files = actual_paths
        .difference(&expected_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut modified_files = Vec::new();
    for path in expected_paths.intersection(&actual_paths) {
        let expected_sha = expected.get(path).expect("path from expected set");
        let actual_sha = actual.get(path).expect("path from actual set");
        if expected_sha != actual_sha {
            modified_files.push(json!({
                "path": path,
                "expected_sha256": expected_sha,
                "actual_sha256": actual_sha
            }));
        }
    }
    let verified =
        missing_files.is_empty() && unlisted_files.is_empty() && modified_files.is_empty();

    let report = serde_json::to_string_pretty(&json!({
        "tool": tools::fixture_manifest_verify::NAME,
        "verified": verified,
        "manifest_path": manifest_normalized,
        "manifest_sha256": sha256_hex(&manifest_bytes),
        "file_count": expected.len(),
        "actual_file_count": actual.len(),
        "missing_files": missing_files,
        "unlisted_files": unlisted_files,
        "modified_files": modified_files
    }))
    .map_err(|error| format!("fixture_manifest_verify refused: {error}"))?;
    if !verified {
        return Err(format!(
            "fixture_manifest_verify refused: fixture file set or digest mismatch\n{report}"
        ));
    }
    Ok(report)
}

fn call_fixture_manifest_refresh(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "refresh fixture manifest";

    let manifest_path = required_string(arguments, "manifest_path")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "fixture_manifest_refresh refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }
    let root = canonical_repo_root(repo_root, tools::fixture_manifest_refresh::NAME)?;
    let manifest_normalized =
        normalize_repo_relative_path(tools::fixture_manifest_refresh::NAME, manifest_path)?;
    let manifest_target = root.join(&manifest_normalized);
    let current_manifest = if manifest_target.exists() {
        if !manifest_target.is_file() {
            return Err(format!(
                "fixture_manifest_refresh refused: manifest path `{manifest_normalized}` is not a regular file"
            ));
        }
        let bytes = fs::read(&manifest_target).map_err(|error| {
            format!(
                "fixture_manifest_refresh refused: failed to read `{manifest_normalized}`: {error}"
            )
        })?;
        Some((sha256_hex(&bytes), bytes))
    } else {
        None
    };
    if !dry_run {
        if let Some((current_sha, _)) = &current_manifest {
            let expected = optional_string(arguments, "expected_manifest_sha256")?.ok_or_else(|| {
                "fixture_manifest_refresh refused: expected_manifest_sha256 is required when overwriting an existing manifest".to_string()
            })?;
            let expected = validate_sha256_hex(tools::fixture_manifest_refresh::NAME, expected)?;
            if &expected != current_sha {
                return Err(format!(
                    "fixture_manifest_refresh refused: manifest hash mismatch; current_sha256={current_sha}, expected_manifest_sha256={expected}"
                ));
            }
        }
    }

    let actual =
        collect_fixture_files_from_args(tools::fixture_manifest_refresh::NAME, &root, arguments)?;
    let files = actual
        .iter()
        .map(|(path, sha256)| {
            let bytes = fs::metadata(root.join(path))
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            json!({
                "path": path,
                "sha256": sha256,
                "bytes": bytes
            })
        })
        .collect::<Vec<_>>();
    let manifest = json!({
        "version": 1,
        "algorithm": "sha256",
        "files": files
    });
    let manifest_text = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("fixture_manifest_refresh refused: {error}"))?
        + "\n";
    let new_sha256 = sha256_hex(manifest_text.as_bytes());

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": tools::fixture_manifest_refresh::NAME,
            "dry_run": true,
            "would_write": true,
            "manifest_path": manifest_normalized,
            "file_count": actual.len(),
            "current_manifest_sha256": current_manifest.as_ref().map(|(sha, _)| sha),
            "new_manifest_sha256": new_sha256,
            "confirm_required": CONFIRMATION
        }))
        .map_err(|error| format!("fixture_manifest_refresh refused: {error}"));
    }

    if let Some(parent) = manifest_target.parent() {
        if !parent.is_dir() {
            return Err(format!(
                "fixture_manifest_refresh refused: parent directory for `{manifest_normalized}` does not exist"
            ));
        }
    }
    let temporary = manifest_target.with_extension(format!(
        "contextpatch-tmp-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("fixture_manifest_refresh refused: {error}"))?
            .as_nanos()
    ));
    fs::write(&temporary, manifest_text).map_err(|error| {
        format!(
            "fixture_manifest_refresh refused: failed to write temporary manifest for `{manifest_normalized}`: {error}"
        )
    })?;
    fs::rename(&temporary, &manifest_target).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        format!(
            "fixture_manifest_refresh refused: failed to replace `{manifest_normalized}`: {error}"
        )
    })?;

    serde_json::to_string_pretty(&json!({
        "tool": tools::fixture_manifest_refresh::NAME,
        "dry_run": false,
        "refreshed": true,
        "manifest_path": manifest_normalized,
        "file_count": actual.len(),
        "sha256": new_sha256
    }))
    .map_err(|error| format!("fixture_manifest_refresh refused: {error}"))
}

fn artifact_root(repo_root: &Path, tool_name: &str) -> Result<PathBuf, String> {
    let root = canonical_repo_root(repo_root, tool_name)?;
    let mut hasher = DefaultHasher::new();
    root.display().to_string().hash(&mut hasher);
    let repo_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let artifact_root = std::env::temp_dir()
        .join("contextpatch-artifacts")
        .join(format!("{repo_name}-{:016x}", hasher.finish()));
    fs::create_dir_all(&artifact_root)
        .map_err(|error| format!("{tool_name} refused: failed to create artifact root: {error}"))?;
    artifact_root
        .canonicalize()
        .map_err(|error| format!("{tool_name} refused: failed to resolve artifact root: {error}"))
}

fn call_create_directory(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let path = required_string(arguments, "path")?;
    let parents = optional_bool(arguments, "parents")?.unwrap_or(false);

    let summary = create_directory_in_root(repo_root, Path::new(path), parents)
        .map_err(|error| format!("create_directory refused: {error}"))?;

    Ok(format!(
        "created directory {} ({} directories created)",
        summary.path.display(),
        summary.directories_created.len()
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
        | "install_capacitor_filesystem"
        | "cap_add_ios"
        | "cap_add_android"
        | "ios_pod_install"
            if params.is_empty() =>
        {
            Ok(SetupActionParams::None)
        }
        "install_capacitor_dependencies"
        | "install_capacitor_filesystem"
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
            derived_data_path: optional_string(params, "derived_data_path")?
                .map(ToString::to_string),
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
        "ios_boot_simulator" => Ok(NativeDeviceParams::IosDevice {
            device: required_string(params, "device")?.to_string(),
        }),
        "ios_read_logs" => Ok(NativeDeviceParams::IosLogs {
            device: required_string(params, "device")?.to_string(),
            duration: optional_string(params, "duration")?
                .or(optional_string(params, "last")?)
                .map(ToString::to_string),
        }),
        "ios_create_simulator" => Ok(NativeDeviceParams::IosCreate {
            name: required_string(params, "name")?.to_string(),
            device_type: required_string(params, "device_type")?.to_string(),
            runtime: optional_string(params, "runtime")?.map(ToString::to_string),
        }),
        "ios_install_app" => Ok(NativeDeviceParams::IosInstall {
            device: required_string(params, "device")?.to_string(),
            app_path: required_string(params, "app_path")?.to_string(),
        }),
        "ios_launch_app" => Ok(NativeDeviceParams::IosLaunch {
            device: required_string(params, "device")?.to_string(),
            app_id: required_string(params, "app_id")?.to_string(),
        }),
        "ios_cap_run" => Ok(NativeDeviceParams::IosCapRun {
            target: required_string(params, "target")?.to_string(),
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

fn call_git_commit_scoped(
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

    let root = canonical_repo_root(repo_root, tools::git_commit_scoped::NAME)?;
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
    git_success_for_tool(tools::git_commit_scoped::NAME, &root, commit_args).map_err(|error| {
        format!(
            "{error}\nindex may contain staged scoped paths because git commit failed after staging"
        )
    })?;

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
}

fn call_git_restore_exact(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "restore exact paths";
    let paths = required_string_array(arguments, "paths")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("git_restore_exact refused: paths must not be empty".to_string());
    }
    if paths.len() > 100 {
        return Err("git_restore_exact refused: at most 100 paths may be restored".to_string());
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "git_restore_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = repo_root.canonicalize().map_err(|error| {
        format!("git_restore_exact refused: failed to resolve repo root: {error}")
    })?;
    let normalized_paths = normalize_git_paths(tools::git_restore_exact::NAME, &root, &paths)?;
    let restore_paths: BTreeSet<String> = normalized_paths.iter().cloned().collect();
    if restore_paths.len() != normalized_paths.len() {
        return Err("git_restore_exact refused: duplicate paths are not allowed".to_string());
    }

    let dirty_paths = git_status_paths_for_tool(tools::git_restore_exact::NAME, &root)?;
    let missing_dirty = restore_paths
        .difference(&dirty_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing_dirty.is_empty() {
        return Err(format!(
            "git_restore_exact refused: every requested path must currently be dirty\nnot_dirty_paths:\n{}",
            format_set(&missing_dirty)
        ));
    }
    ensure_restore_paths_are_tracked(&root, &normalized_paths)?;

    if dry_run {
        let remaining_dirty_paths = dirty_paths
            .difference(&restore_paths)
            .cloned()
            .collect::<BTreeSet<_>>();
        return serde_json::to_string_pretty(&json!({
            "tool": tools::git_restore_exact::NAME,
            "dry_run": true,
            "would_restore_paths": normalized_paths,
            "remaining_dirty_paths_after_restore": remaining_dirty_paths,
            "required_confirm_for_restore": CONFIRMATION
        }))
        .map_err(|error| format!("git_restore_exact refused: {error}"));
    }

    git_success_for_tool(
        tools::git_restore_exact::NAME,
        &root,
        git_restore_args(&normalized_paths),
    )?;
    let dirty_after = git_status_paths_for_tool(tools::git_restore_exact::NAME, &root)?;
    let still_dirty = restore_paths
        .intersection(&dirty_after)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !still_dirty.is_empty() {
        return Err(format!(
            "git_restore_exact refused after restore: requested paths are still dirty\n{}",
            format_set(&still_dirty)
        ));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::git_restore_exact::NAME,
        "dry_run": false,
        "restored": true,
        "restored_paths": normalized_paths,
        "remaining_dirty_paths": dirty_after
    }))
    .map_err(|error| format!("git_restore_exact refused: {error}"))
}

fn call_delete_untracked_exact(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "delete untracked files";
    let paths = required_string_array(arguments, "paths")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    if paths.is_empty() {
        return Err("delete_untracked_exact refused: paths must not be empty".to_string());
    }
    if paths.len() > 100 {
        return Err("delete_untracked_exact refused: at most 100 paths may be deleted".to_string());
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "delete_untracked_exact refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let root = canonical_repo_root(repo_root, tools::delete_untracked_exact::NAME)?;
    let normalized_paths = normalize_git_paths(tools::delete_untracked_exact::NAME, &root, &paths)?;
    let requested_paths: BTreeSet<String> = normalized_paths.iter().cloned().collect();
    if requested_paths.len() != normalized_paths.len() {
        return Err("delete_untracked_exact refused: duplicate paths are not allowed".to_string());
    }

    let untracked_paths = git_untracked_paths_for_tool(tools::delete_untracked_exact::NAME, &root)?;
    let not_untracked = requested_paths
        .difference(&untracked_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !not_untracked.is_empty() {
        return Err(format!(
            "delete_untracked_exact refused: every requested path must be an untracked file\nnot_untracked_paths:\n{}",
            format_set(&not_untracked)
        ));
    }
    for path in &normalized_paths {
        let target = root.join(path);
        if !target.is_file() {
            return Err(format!(
                "delete_untracked_exact refused: `{path}` is not a regular file"
            ));
        }
    }

    if dry_run {
        let remaining_untracked_paths = untracked_paths
            .difference(&requested_paths)
            .cloned()
            .collect::<BTreeSet<_>>();
        return serde_json::to_string_pretty(&json!({
            "tool": tools::delete_untracked_exact::NAME,
            "dry_run": true,
            "would_delete_paths": normalized_paths,
            "remaining_untracked_paths_after_delete": remaining_untracked_paths,
            "required_confirm_for_delete": CONFIRMATION
        }))
        .map_err(|error| format!("delete_untracked_exact refused: {error}"));
    }

    for path in &normalized_paths {
        fs::remove_file(root.join(path)).map_err(|error| {
            format!("delete_untracked_exact refused: failed to delete `{path}`: {error}")
        })?;
    }
    let untracked_after = git_untracked_paths_for_tool(tools::delete_untracked_exact::NAME, &root)?;
    let still_untracked = requested_paths
        .intersection(&untracked_after)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !still_untracked.is_empty() {
        return Err(format!(
            "delete_untracked_exact refused after delete: requested paths are still untracked\n{}",
            format_set(&still_untracked)
        ));
    }

    serde_json::to_string_pretty(&json!({
        "tool": tools::delete_untracked_exact::NAME,
        "dry_run": false,
        "deleted": true,
        "deleted_paths": normalized_paths,
        "remaining_untracked_paths": untracked_after
    }))
    .map_err(|error| format!("delete_untracked_exact refused: {error}"))
}

fn call_git_remote_list(repo_root: &Path) -> Result<String, String> {
    let root = canonical_repo_root(repo_root, tools::git_remote_list::NAME)?;
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

fn call_github_pr_run(
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

fn call_github_fork_prepare(
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

fn nonempty_tool_string(tool: &str, key: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{tool} refused: {key} must not be empty"));
    }
    Ok(trimmed.to_string())
}

fn validate_relative_path(tool_name: &str, raw: &str) -> Result<PathBuf, String> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(format!(
            "{tool_name} refused: path must not be empty or contain NUL"
        ));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!("{tool_name} refused: path must be relative"));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "{tool_name} refused: path must be a normalized relative path"
                ))
            }
        }
    }
    Ok(path.to_path_buf())
}

fn normalize_repo_relative_paths(tool_name: &str, paths: &[String]) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| normalize_repo_relative_path(tool_name, path))
        .collect()
}

fn normalize_repo_relative_path(tool_name: &str, raw: &str) -> Result<String, String> {
    let path = validate_relative_path(tool_name, raw)?;
    let parts = path
        .components()
        .map(|component| match component {
            std::path::Component::Normal(value) => Ok(value.to_string_lossy().into_owned()),
            _ => Err(format!(
                "{tool_name} refused: path must be a normalized relative path"
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

fn matches_any_prefix(path: &str, prefixes: &[String]) -> bool {
    prefixes
        .iter()
        .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
}

fn guarded_output_succeeded(output: &str) -> bool {
    output.lines().any(|line| line.trim() == "exit_code: 0")
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    let mut values = Vec::new();
    let mut padding_started = false;
    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding_started = true;
                64
            }
            b'\r' | b'\n' | b'\t' | b' ' => continue,
            _ => return Err("content_base64 contains invalid characters".to_string()),
        };
        if padding_started && value != 64 {
            return Err("content_base64 has data after padding".to_string());
        }
        values.push(value);
    }
    if values.is_empty() {
        return Ok(Vec::new());
    }
    if values.len() % 4 != 0 {
        return Err("content_base64 length must be a multiple of 4".to_string());
    }

    let mut output = Vec::with_capacity(values.len() / 4 * 3);
    for chunk in values.chunks_exact(4) {
        let pad = chunk.iter().filter(|value| **value == 64).count();
        if pad > 2 || (pad > 0 && chunk[2] != 64 && chunk[3] == 64 && pad != 1) {
            return Err("content_base64 has invalid padding".to_string());
        }
        if chunk[0] == 64 || chunk[1] == 64 || (chunk[2] == 64 && chunk[3] != 64) {
            return Err("content_base64 has invalid padding".to_string());
        }
        let a = u32::from(chunk[0]);
        let b = u32::from(chunk[1]);
        let c = u32::from(if chunk[2] == 64 { 0 } else { chunk[2] });
        let d = u32::from(if chunk[3] == 64 { 0 } else { chunk[3] });
        let triple = (a << 18) | (b << 12) | (c << 6) | d;
        output.push(((triple >> 16) & 0xff) as u8);
        if chunk[2] != 64 {
            output.push(((triple >> 8) & 0xff) as u8);
        }
        if chunk[3] != 64 {
            output.push((triple & 0xff) as u8);
        }
    }

    Ok(output)
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

fn normalize_git_paths(
    tool_name: &str,
    root: &Path,
    paths: &[String],
) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| normalize_git_path(tool_name, root, path))
        .collect()
}

fn normalize_git_path(tool_name: &str, root: &Path, raw: &str) -> Result<String, String> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(format!(
            "{tool_name} refused: path must not be empty or contain NUL"
        ));
    }
    if raw.starts_with(':') || raw.contains('*') || raw.contains('?') || raw.contains('[') {
        return Err(format!(
            "{tool_name} refused: path `{raw}` contains Git pathspec metacharacters"
        ));
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!(
            "{tool_name} refused: path `{raw}` must be repository-relative"
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "{tool_name} refused: path `{raw}` must be a normalized relative path"
                ))
            }
        }
    }

    let candidate = root.join(path);
    let resolved = if candidate.exists() {
        candidate.canonicalize().map_err(|error| {
            format!("{tool_name} refused: failed to resolve path `{raw}`: {error}")
        })?
    } else {
        let parent = candidate
            .parent()
            .ok_or_else(|| format!("{tool_name} refused: path `{raw}` has no parent"))?;
        let parent = parent.canonicalize().map_err(|error| {
            format!("{tool_name} refused: failed to resolve parent for `{raw}`: {error}")
        })?;
        let file_name = candidate
            .file_name()
            .ok_or_else(|| format!("{tool_name} refused: path `{raw}` has no file name"))?;
        parent.join(file_name)
    };

    if !resolved.starts_with(root) {
        return Err(format!(
            "{tool_name} refused: path `{raw}` resolves outside repository root"
        ));
    }

    Ok(raw.to_string())
}

fn git_status_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    git_status_paths_for_tool(tools::git_commit_exact::NAME, root)
}

fn git_status_paths_for_tool(tool_name: &str, root: &Path) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(
        tool_name,
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_porcelain_paths_for_tool(tool_name, &output.stdout, "git status")
}

fn git_untracked_paths_for_tool(tool_name: &str, root: &Path) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(
        tool_name,
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_untracked_porcelain_paths_for_tool(tool_name, &output.stdout, "git status")
}

fn git_cached_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    git_cached_paths_for_tool(tools::git_commit_exact::NAME, root)
}

fn git_cached_paths_for_tool(tool_name: &str, root: &Path) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(tool_name, root, &["diff", "--cached", "--name-only", "-z"])?;
    parse_nul_paths_for_tool(tool_name, &output.stdout, "git diff --cached")
}

fn parse_porcelain_paths_for_tool(
    tool_name: &str,
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    let entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    for entry in entries {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(format!("{tool_name} refused: unexpected {label} entry"));
        }
        if matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C') {
            return Err(format!(
                "{tool_name} refused: rename/copy entries require a future dedicated tool"
            ));
        }
        let path = std::str::from_utf8(&entry[3..])
            .map_err(|error| format!("{tool_name} refused: {label} path is not UTF-8: {error}"))?;
        paths.insert(path.to_string());
    }
    Ok(paths)
}

fn parse_untracked_porcelain_paths_for_tool(
    tool_name: &str,
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, String> {
    let mut paths = BTreeSet::new();
    let entries = bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty());
    for entry in entries {
        if entry.len() < 4 || entry[2] != b' ' {
            return Err(format!("{tool_name} refused: unexpected {label} entry"));
        }
        if entry[0] == b'?' && entry[1] == b'?' {
            let path = std::str::from_utf8(&entry[3..]).map_err(|error| {
                format!("{tool_name} refused: {label} path is not UTF-8: {error}")
            })?;
            paths.insert(path.to_string());
        }
    }
    Ok(paths)
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

fn git_restore_args(paths: &[String]) -> Vec<String> {
    ["restore", "--staged", "--worktree", "--"]
        .into_iter()
        .map(ToString::to_string)
        .chain(paths.iter().cloned())
        .collect()
}

fn ensure_restore_paths_are_tracked(root: &Path, paths: &[String]) -> Result<(), String> {
    let untracked = paths
        .iter()
        .filter_map(|path| {
            let args = ["ls-files", "--error-unmatch", "--", path.as_str()];
            match git_output_for_tool(tools::git_restore_exact::NAME, root, &args) {
                Ok(_) => None,
                Err(_) => Some(path.clone()),
            }
        })
        .collect::<BTreeSet<_>>();

    if !untracked.is_empty() {
        return Err(format!(
            "git_restore_exact refused: untracked paths cannot be restored from HEAD\n{}",
            format_set(&untracked)
        ));
    }
    Ok(())
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
        "dynamo-harbor-task" => Ok(vec![
            ProfileCommand {
                program: "git",
                args: vec!["diff", "--check"],
                cwd: None,
                timeout_secs: Some(30),
            },
            ProfileCommand {
                program: "bash",
                args: vec!["references/check-base-image.sh", "task"],
                cwd: None,
                timeout_secs: Some(600),
            },
            ProfileCommand {
                program: "harbor",
                args: vec!["run", "-p", "task", "--agent", "oracle"],
                cwd: None,
                timeout_secs: Some(600),
            },
            ProfileCommand {
                program: "harbor",
                args: vec!["run", "-p", "task", "--agent", "nop"],
                cwd: None,
                timeout_secs: Some(600),
            },
            ProfileCommand {
                program: "harbor",
                args: vec!["run", "-p", "task", "--agent", "oracle"],
                cwd: None,
                timeout_secs: Some(600),
            },
            ProfileCommand {
                program: "harbor",
                args: vec!["run", "-p", "task", "--agent", "nop"],
                cwd: None,
                timeout_secs: Some(600),
            },
        ]),
        _ => Err(format!(
            "validation_profile_run refused: unknown profile `{profile}`; expected repo-basic, rust-workspace, datacore-vscode, datacore-m6-vscode, or dynamo-harbor-task"
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

fn xcodebuild_available() -> Value {
    let output = std::process::Command::new("xcodebuild")
        .arg("-version")
        .output();
    match output {
        Ok(output) => json!({
            "available": output.status.success(),
            "exit_code": output.status.code().unwrap_or(-1),
            "probe": "xcodebuild -version",
            "version": String::from_utf8_lossy(&output.stdout).trim(),
            "diagnostic": xcodebuild_diagnostic(&output)
        }),
        Err(error) => json!({
            "available": false,
            "probe": "xcodebuild -version",
            "error": error.to_string()
        }),
    }
}

fn xcodebuild_is_available() -> bool {
    std::process::Command::new("xcodebuild")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn xcodebuild_diagnostic(output: &std::process::Output) -> Value {
    if output.status.success() {
        return Value::Null;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}\n{stderr}");
    let lower = combined.to_ascii_lowercase();
    if lower.contains("license") {
        return json!(
            "xcodebuild is installed but requires accepting the Xcode license outside contextpatch"
        );
    }
    if lower.contains("requires xcode") || lower.contains("full xcode") {
        return json!("xcodebuild is present but full Xcode may not be installed or selected");
    }
    Value::Null
}

fn xcode_select_available() -> Value {
    let output = std::process::Command::new("xcode-select")
        .arg("-p")
        .output();
    match output {
        Ok(output) => json!({
            "available": output.status.success(),
            "exit_code": output.status.code().unwrap_or(-1),
            "path": String::from_utf8_lossy(&output.stdout).trim()
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
