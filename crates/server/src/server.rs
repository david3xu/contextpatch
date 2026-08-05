use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{json, Value};

use crate::protocol::response::{error_response, success_response};
use crate::tools::ToolSurface;

#[derive(Debug)]
pub(crate) struct ServerOptions {
    pub(crate) repo_root: PathBuf,
    pub(crate) tool_surface: ToolSurface,
}

pub(crate) const MAX_ACTIVE_TOOL_CALLS: usize = 16;
const MAX_BUFFERED_STDIN_LINES: usize = MAX_ACTIVE_TOOL_CALLS * 2;
const STDIN_POLL_INTERVAL: Duration = Duration::from_millis(25);

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

/// Resolve the configured repository root exactly once, at startup.
///
/// Every downstream guard, scratch path, and mutation lock derives from this
/// value, so resolving it here means a missing or misspelled root fails fast
/// with one clear message instead of surfacing later as a confusing per-tool
/// error, and two spellings of the same directory can no longer produce
/// different scratch or lock keys.
///
/// A non-Git workspace root stays valid on purpose: the wrapper `repository`
/// selector exists to reach descendant worktrees. Requiring a worktree root
/// belongs to the individual tools that need one, not to startup.
pub(crate) fn resolve_server_options(options: ServerOptions) -> Result<ServerOptions, String> {
    let repo_root = options.repo_root.canonicalize().map_err(|error| {
        format!(
            "--repo-root {} could not be resolved: {error}",
            options.repo_root.display()
        )
    })?;
    if !repo_root.is_dir() {
        return Err(format!(
            "--repo-root {} is not a directory",
            repo_root.display()
        ));
    }

    Ok(ServerOptions {
        repo_root,
        tool_surface: options.tool_surface,
    })
}

fn spawn_stdin_reader() -> io::Result<mpsc::Receiver<io::Result<String>>> {
    let (sender, receiver) = mpsc::sync_channel(MAX_BUFFERED_STDIN_LINES);
    thread::Builder::new()
        .name("contextpatch-mcp-stdin".to_string())
        .spawn(move || {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                if sender.send(line).is_err() {
                    break;
                }
            }
        })?;
    Ok(receiver)
}

pub(crate) fn run_stdio_server(options: &ServerOptions) -> ExitCode {
    let stdin_lines = match spawn_stdin_reader() {
        Ok(stdin_lines) => stdin_lines,
        Err(error) => {
            eprintln!("failed to start stdin reader: {error}");
            return ExitCode::from(1);
        }
    };
    let stdout = Arc::new(Mutex::new(io::stdout()));
    let active = Arc::new(AtomicUsize::new(0));
    let write_failed = Arc::new(AtomicBool::new(false));
    let mut workers = Vec::new();

    loop {
        reap_finished_workers(&mut workers);
        if write_failed.load(Ordering::Acquire) {
            return ExitCode::from(1);
        }

        let line = match stdin_lines.recv_timeout(STDIN_POLL_INTERVAL) {
            Ok(Ok(line)) => line,
            Ok(Err(error)) => {
                eprintln!("failed to read stdin: {error}");
                return ExitCode::from(1);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                if let Err(error) = write_response(
                    &stdout,
                    &error_response(Value::Null, -32700, &format!("parse error: {error}")),
                ) {
                    eprintln!("failed to write stdout: {error}");
                    return ExitCode::from(1);
                }
                continue;
            }
        };

        if should_dispatch_concurrently(&request) {
            let id = request.get("id").cloned().unwrap_or(Value::Null);
            let permit = match ToolCallPermit::try_acquire(Arc::clone(&active)) {
                Ok(permit) => permit,
                Err(active) => {
                    let response = error_response(
                        id,
                        -32000,
                        &format!(
                            "server busy: {active} tool calls are active (maximum \
                             {MAX_ACTIVE_TOOL_CALLS}); this call was not started. Poll existing \
                             asynchronous work with read_command_log or retry after an in-flight \
                             call completes."
                        ),
                    );
                    if let Err(error) = write_response(&stdout, &response) {
                        eprintln!("failed to write stdout: {error}");
                        return ExitCode::from(1);
                    }
                    continue;
                }
            };
            let repo_root = options.repo_root.clone();
            let surface = options.tool_surface;
            let worker_stdout = Arc::clone(&stdout);
            let worker_write_failed = Arc::clone(&write_failed);
            let worker_id = id.clone();
            let spawn = thread::Builder::new()
                .name("contextpatch-mcp-call".to_string())
                .spawn(move || {
                    let _permit = permit;
                    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        handle_request(&repo_root, surface, &request)
                    }))
                    .unwrap_or_else(|_| {
                        Some(error_response(
                            worker_id,
                            -32603,
                            "tool worker failed unexpectedly",
                        ))
                    });
                    if let Some(response) = response {
                        if let Err(error) = write_response(&worker_stdout, &response) {
                            eprintln!("failed to write stdout: {error}");
                            worker_write_failed.store(true, Ordering::Release);
                        }
                    }
                });
            match spawn {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    let response =
                        error_response(id, -32603, &format!("tool call was not started: {error}"));
                    if let Err(error) = write_response(&stdout, &response) {
                        eprintln!("failed to write stdout: {error}");
                        return ExitCode::from(1);
                    }
                }
            }
            continue;
        }

        if let Some(response) = handle_request(&options.repo_root, options.tool_surface, &request) {
            if let Err(error) = write_response(&stdout, &response) {
                eprintln!("failed to write stdout: {error}");
                return ExitCode::from(1);
            }
        }
    }

    for worker in workers {
        if worker.join().is_err() {
            eprintln!("tool worker failed unexpectedly");
        }
    }
    if write_failed.load(Ordering::Acquire) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn handle_request(repo_root: &Path, surface: ToolSurface, request: &Value) -> Option<String> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str);
    let Some(method) = method else {
        return id.map(|id| error_response(id, -32600, "missing method"));
    };

    match (method, id) {
        ("initialize", Some(id)) => Some(success_response(
            id,
            json!({
                "protocolVersion": requested_protocol_version(request),
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
            repo_root, surface, id, request,
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

fn should_dispatch_concurrently(request: &Value) -> bool {
    request.get("id").is_some()
        && request.get("method").and_then(Value::as_str) == Some("tools/call")
}

fn write_response(stdout: &Arc<Mutex<io::Stdout>>, response: &str) -> io::Result<()> {
    let mut stdout = stdout.lock().unwrap_or_else(|error| error.into_inner());
    writeln!(stdout, "{response}")?;
    stdout.flush()
}

fn reap_finished_workers(workers: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            let worker = workers.swap_remove(index);
            if worker.join().is_err() {
                eprintln!("tool worker failed unexpectedly");
            }
        } else {
            index += 1;
        }
    }
}

struct ToolCallPermit {
    active: Arc<AtomicUsize>,
}

impl ToolCallPermit {
    fn try_acquire(active: Arc<AtomicUsize>) -> Result<Self, usize> {
        active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (current < MAX_ACTIVE_TOOL_CALLS).then_some(current + 1)
            })
            .map(|_| Self { active })
    }
}

impl Drop for ToolCallPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
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

    #[test]
    fn dispatches_every_id_bearing_tool_call_concurrently() {
        let guarded = json!({
            "id": 1,
            "method": "tools/call",
            "params": {"name": "run_guarded_command", "arguments": {}}
        });
        let wrapped = json!({
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "project_execute",
                "arguments": {"action": "native_build_run", "arguments": {}}
            }
        });
        let read = json!({
            "id": 3,
            "method": "tools/call",
            "params": {"name": "read_range", "arguments": {}}
        });
        let notification = json!({
            "method": "tools/call",
            "params": {"name": "read_range", "arguments": {}}
        });
        let listing = json!({
            "id": 4,
            "method": "tools/list"
        });

        assert!(should_dispatch_concurrently(&guarded));
        assert!(should_dispatch_concurrently(&wrapped));
        assert!(should_dispatch_concurrently(&read));
        assert!(!should_dispatch_concurrently(&notification));
        assert!(!should_dispatch_concurrently(&listing));
    }
}
