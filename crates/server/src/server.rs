use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{json, Value};

use crate::protocol::response::{error_response, success_response};
use crate::tools::ToolSurface;

#[derive(Debug)]
pub(crate) struct ServerOptions {
    pub(crate) repo_root: PathBuf,
    pub(crate) tool_surface: ToolSurface,
}

pub(crate) fn parse_server_options(args: Vec<String>) -> Result<ServerOptions, String> {
    let mut repo_root = std::env::current_dir()
        .map_err(|error| format!("failed to read current directory: {error}"))?;
    let mut tool_surface = ToolSurface::Full;
    let mut tool_surface_seen = false;
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
            "--tool-surface" => {
                if tool_surface_seen {
                    return Err("--tool-surface may be provided only once".to_string());
                }
                tool_surface_seen = true;
                index += 1;
                tool_surface = ToolSurface::parse(
                    args.get(index)
                        .ok_or_else(|| "--tool-surface requires a value".to_string())?,
                )?;
            }
            "--help" | "-h" => {
                return Err("usage: contextpatch-server [--repo-root <path>] \
                     [--tool-surface <full|project>]\n\nRuns the stdio MCP server."
                    .to_string());
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
        index += 1;
    }

    Ok(ServerOptions {
        repo_root,
        tool_surface,
    })
}

pub(crate) fn run_stdio_server(options: &ServerOptions) -> ExitCode {
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

        let response = handle_line(&options.repo_root, options.tool_surface, &line);
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

fn handle_line(repo_root: &Path, surface: ToolSurface, line: &str) -> Option<String> {
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
                "instructions": crate::protocol::instructions::client_instructions(surface)
            }),
        )),
        ("tools/list", Some(id)) => Some(success_response(
            id,
            json!({
                "tools": crate::tools::schema::tool_definitions(surface)
            }),
        )),
        ("tools/call", Some(id)) => Some(crate::tools::dispatch::handle_tool_call(
            repo_root, surface, id, &request,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_options_default_to_full_surface() {
        let options = parse_server_options(vec!["--repo-root".into(), "/tmp/repo".into()]).unwrap();
        assert_eq!(options.repo_root, PathBuf::from("/tmp/repo"));
        assert_eq!(options.tool_surface, ToolSurface::Full);
    }

    #[test]
    fn server_options_accept_project_surface() {
        let options = parse_server_options(vec![
            "--repo-root".into(),
            "/tmp/repo".into(),
            "--tool-surface".into(),
            "project".into(),
        ])
        .unwrap();
        assert_eq!(options.tool_surface, ToolSurface::Project);
    }

    #[test]
    fn server_options_reject_unknown_surface() {
        let error = parse_server_options(vec!["--tool-surface".into(), "wide".into()]).unwrap_err();
        assert!(error.contains("full"));
        assert!(error.contains("project"));
    }

    #[test]
    fn server_options_reject_duplicate_surface() {
        let error = parse_server_options(vec![
            "--tool-surface".into(),
            "project".into(),
            "--tool-surface".into(),
            "full".into(),
        ])
        .unwrap_err();
        assert!(error.contains("only once"));
    }
}
