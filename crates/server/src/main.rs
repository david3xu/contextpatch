mod protocol;
mod tools;

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

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
            "status_guard": true
        },
        "process_execution": {
            "available": true,
            "mode": "allowlisted_no_shell",
            "programs": {
                "git": ["status", "diff", "log", "show", "rev-parse"],
                "cargo": ["check", "test", "build", "clippy"],
                "bun": ["run", "test"],
                "npm": ["run", "test"],
                "rg": ["search"]
            },
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
                "git reset/checkout/stash/clean",
                "automatic commits",
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

    run_guarded_command(repo_root, cwd.map(Path::new), program, &args, timeout_secs)
        .map_err(|error| format!("run_guarded_command refused: {error}"))
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
