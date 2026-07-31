use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{json, Value};

use crate::protocol::response::{error_response, success_response};

pub(crate) fn parse_repo_root(args: Vec<String>) -> Result<PathBuf, String> {
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

pub(crate) fn run_stdio_server(repo_root: &Path) -> ExitCode {
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
                    "name": crate::protocol::metadata::PROTOCOL_NAME,
                    "version": contextpatch_core::VERSION
                },
                // Clients surface this to the model before its first tool call. It exists because the
                // natural inference from a missing tool is that the capability is absent, when the usual
                // cause is a binary older than the checkout.
                "instructions": crate::protocol::instructions::CLIENT_INSTRUCTIONS
            }),
        )),
        ("tools/list", Some(id)) => Some(success_response(
            id,
            json!({
                "tools": crate::tools::schema::tool_definitions()
            }),
        )),
        ("tools/call", Some(id)) => Some(crate::tools::dispatch::handle_tool_call(
            repo_root, id, &request,
        )),
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
