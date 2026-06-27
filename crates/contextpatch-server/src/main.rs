mod protocol;
mod tools;

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use contextpatch_core::fs::read_range::read_range_in_root;
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
        tools::read_range::NAME => call_read_range(repo_root, &arguments),
        tools::replace_exact::NAME => call_replace_exact(repo_root, &arguments),
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

fn required_string<'a>(
    arguments: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or invalid string argument: {key}"))
}

fn required_usize(arguments: &serde_json::Map<String, Value>, key: &str) -> Result<usize, String> {
    let value = arguments
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing or invalid integer argument: {key}"))?;

    usize::try_from(value).map_err(|_| format!("integer argument out of range: {key}"))
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
