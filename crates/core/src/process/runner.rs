use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::error::ContextPatchError;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
const MAX_ARGS: usize = 64;
const MAX_ARG_LEN: usize = 4096;
const MAX_OUTPUT_CHARS: usize = 12_000;

pub(crate) struct ProcessOutput {
    pub(crate) cwd: PathBuf,
    pub(crate) exit_code: i32,
    pub(crate) timed_out: bool,
    pub(crate) duration_ms: u128,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

/// Raw output from one no-shell child process with a bounded execution time.
///
/// This lower-level result intentionally preserves stdout and stderr bytes. Git callers use
/// NUL-delimited output and must not pass through the redaction and truncation applied to
/// user-facing validation command output.
pub struct BoundedProcessOutput {
    pub cwd: PathBuf,
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl BoundedProcessOutput {
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit_code == 0
    }
}

pub(crate) fn run_no_shell_command(
    cwd: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    operation_label: &str,
) -> Result<ProcessOutput, ContextPatchError> {
    let output = run_bounded_command(cwd, program, args, timeout, operation_label)?;
    Ok(ProcessOutput {
        cwd: output.cwd,
        exit_code: output.exit_code,
        timed_out: output.timed_out,
        duration_ms: output.duration_ms,
        stdout: redact_and_truncate(&String::from_utf8_lossy(&output.stdout)),
        stderr: redact_and_truncate(&String::from_utf8_lossy(&output.stderr)),
    })
}

/// Run a program directly, with null stdin, captured output, and a hard child-process timeout.
///
/// The caller owns command policy. This function supplies execution mechanics only: no shell is
/// involved, Git paging is disabled, output pipes are drained concurrently, and the child is
/// killed when the timeout expires.
pub fn run_bounded_command(
    cwd: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    operation_label: &str,
) -> Result<BoundedProcessOutput, ContextPatchError> {
    let resolved_program = resolve_program(program);
    let mut command = Command::new(
        resolved_program
            .as_deref()
            .unwrap_or_else(|| Path::new(program)),
    );
    command.current_dir(cwd);
    if program == "git" {
        command.arg("--no-pager");
    }

    command.args(args);
    command.env("GIT_PAGER", "cat");
    command.env("NO_COLOR", "1");
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let started = std::time::Instant::now();
    let mut child = command.spawn().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to run {operation_label} `{}` in {}: {error}",
            display_command(program, args),
            cwd.display()
        ))
    })?;

    let stdout = child.stdout.take().ok_or_else(|| {
        ContextPatchError::new(format!("failed to capture {operation_label} stdout pipe"))
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        ContextPatchError::new(format!("failed to capture {operation_label} stderr pipe"))
    })?;
    let stdout_reader = spawn_stream_reader("stdout", stdout, operation_label);
    let stderr_reader = spawn_stream_reader("stderr", stderr, operation_label);

    let status = loop {
        match child.try_wait().map_err(|error| {
            ContextPatchError::new(format!(
                "failed while waiting for {operation_label} `{}`: {error}",
                display_command(program, args)
            ))
        })? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                terminate_child(&mut child, operation_label)?;
                let stdout = join_stream_reader(stdout_reader, operation_label)?;
                let stderr = join_stream_reader(stderr_reader, operation_label)?;
                return Ok(BoundedProcessOutput {
                    cwd: cwd.to_path_buf(),
                    exit_code: -1,
                    timed_out: true,
                    duration_ms: started.elapsed().as_millis(),
                    stdout,
                    stderr,
                });
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    };

    let stdout = join_stream_reader(stdout_reader, operation_label)?;
    let stderr = join_stream_reader(stderr_reader, operation_label)?;

    Ok(BoundedProcessOutput {
        cwd: cwd.to_path_buf(),
        exit_code: status.code().unwrap_or(-1),
        timed_out: false,
        duration_ms: started.elapsed().as_millis(),
        stdout,
        stderr,
    })
}

fn terminate_child(
    child: &mut std::process::Child,
    operation_label: &str,
) -> Result<(), ContextPatchError> {
    let already_reaped;
    #[cfg(unix)]
    {
        let process_group = -(child.id() as i32);
        // The child starts a fresh process group, so this also stops hooks or credential helpers
        // that would otherwise keep inherited output pipes open after the direct child is killed.
        let result = unsafe { libc::kill(process_group, libc::SIGKILL) };
        if result != 0 {
            // The direct child can exit between try_wait and kill. Re-check before treating a
            // missing process group as a timeout-handling failure.
            already_reaped = kill_direct_child_if_running(child, operation_label)?;
        } else {
            already_reaped = false;
        }
    }
    #[cfg(not(unix))]
    {
        already_reaped = kill_direct_child_if_running(child, operation_label)?;
    }

    if !already_reaped {
        child.wait().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to reap timed-out {operation_label}: {error}"
            ))
        })?;
    }
    Ok(())
}

fn kill_direct_child_if_running(
    child: &mut std::process::Child,
    operation_label: &str,
) -> Result<bool, ContextPatchError> {
    match child.try_wait().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to inspect timed-out {operation_label}: {error}"
        ))
    })? {
        Some(_) => Ok(true),
        None => {
            child.kill().map_err(|error| {
                ContextPatchError::new(format!(
                    "failed to terminate timed-out {operation_label}: {error}"
                ))
            })?;
            Ok(false)
        }
    }
}

fn spawn_stream_reader(
    label: &'static str,
    mut stream: impl Read + Send + 'static,
    operation_label: &str,
) -> thread::JoinHandle<Result<Vec<u8>, ContextPatchError>> {
    let operation_label = operation_label.to_string();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).map_err(|error| {
            ContextPatchError::new(format!("failed to read {operation_label} {label}: {error}"))
        })?;
        Ok(buffer)
    })
}

fn join_stream_reader(
    reader: thread::JoinHandle<Result<Vec<u8>, ContextPatchError>>,
    operation_label: &str,
) -> Result<Vec<u8>, ContextPatchError> {
    let (sender, receiver) = mpsc::channel();
    let operation_label = operation_label.to_string();
    let reader_operation_label = operation_label.clone();
    thread::spawn(move || {
        let result = reader.join().unwrap_or_else(|_| {
            Err(ContextPatchError::new(format!(
                "{reader_operation_label} reader panicked"
            )))
        });
        let _ = sender.send(result);
    });
    receiver.recv_timeout(Duration::from_secs(5)).map_err(|_| {
        ContextPatchError::new(format!("timed out reading {operation_label} output"))
    })?
}

pub(crate) fn resolve_program(program: &str) -> Option<PathBuf> {
    configured_tool_paths()
        .into_iter()
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

fn configured_tool_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    if let Some(path) = std::env::var_os("CONTEXTPATCH_VALIDATION_PATHS") {
        paths.extend(std::env::split_paths(&path));
    }
    paths
}

pub(crate) fn resolve_cwd(root: &Path, cwd: Option<&Path>) -> Result<PathBuf, ContextPatchError> {
    let cwd = match cwd {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => root.join(path),
        None => root.to_path_buf(),
    };
    let resolved = cwd.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve command cwd {}: {error}",
            cwd.display()
        ))
    })?;
    if !resolved.starts_with(root) {
        return Err(ContextPatchError::new(format!(
            "command cwd {} is outside repository root {}",
            resolved.display(),
            root.display()
        )));
    }
    if !resolved.is_dir() {
        return Err(ContextPatchError::new(format!(
            "command cwd {} is not a directory",
            resolved.display()
        )));
    }
    Ok(resolved)
}

pub(crate) fn checked_timeout(timeout_secs: Option<u64>) -> Result<Duration, ContextPatchError> {
    checked_timeout_with_max(timeout_secs, MAX_TIMEOUT_SECS)
}

pub(crate) fn checked_timeout_with_max(
    timeout_secs: Option<u64>,
    max_timeout_secs: u64,
) -> Result<Duration, ContextPatchError> {
    let timeout_secs = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    if timeout_secs == 0 || timeout_secs > max_timeout_secs {
        return Err(ContextPatchError::new(format!(
            "timeout_secs must be between 1 and {max_timeout_secs}"
        )));
    }
    Ok(Duration::from_secs(timeout_secs))
}

pub(crate) fn validate_common_command_shape(
    program: &str,
    args: &[String],
) -> Result<(), ContextPatchError> {
    if args.len() > MAX_ARGS {
        return Err(ContextPatchError::new(format!(
            "too many command arguments: maximum is {MAX_ARGS}"
        )));
    }
    if program.contains('/') || program.contains('\\') || program.is_empty() {
        return Err(ContextPatchError::new(
            "program must be an allowlisted executable name, not a path",
        ));
    }
    for arg in args {
        if arg.len() > MAX_ARG_LEN || arg.contains('\0') {
            return Err(ContextPatchError::new("command argument is invalid"));
        }
        if arg == ".." || arg.starts_with("../") || arg.contains("/../") || arg.starts_with('/') {
            return Err(ContextPatchError::new(format!(
                "command argument may not reference paths outside the repository root: {arg}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn display_command(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().map(|arg| shell_display_arg(arg)))
        .collect::<Vec<_>>()
        .join(" ")
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

fn redact_and_truncate(text: &str) -> String {
    let mut redacted = Vec::new();
    for line in text.lines() {
        redacted.push(redact_line(line));
    }
    let mut text = redacted.join("\n");
    if text.len() > MAX_OUTPUT_CHARS {
        text.truncate(MAX_OUTPUT_CHARS);
        text.push_str("\n[truncated]");
    }
    text
}

pub(crate) fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if contains_openai_style_key(line)
        || lower.contains("authorization: bearer ")
        || contains_secret_assignment(&lower, "api_key")
        || contains_secret_assignment(&lower, "apikey")
        || contains_secret_assignment(&lower, "token")
        || contains_secret_assignment(&lower, "secret")
        || contains_secret_assignment(&lower, "password")
    {
        "[redacted potential secret line]".to_string()
    } else {
        line.to_string()
    }
}

fn contains_openai_style_key(line: &str) -> bool {
    line.match_indices("sk-").any(|(index, _)| {
        let preceded_by_word = line[..index]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
        !preceded_by_word
    })
}

fn contains_secret_assignment(line: &str, name: &str) -> bool {
    let Some(index) = line.find(name) else {
        return false;
    };
    let tail = line[index + name.len()..].trim_start();
    let value = if let Some(value) = tail.strip_prefix('=') {
        value
    } else if let Some(value) = tail.strip_prefix(':') {
        value
    } else if let Some(value) = tail.strip_prefix("\":") {
        value
    } else {
        return false;
    };
    is_probable_secret_value(value)
}

fn is_probable_secret_value(value: &str) -> bool {
    let value = value
        .trim()
        .trim_matches(|ch| matches!(ch, '"' | '\'' | '`' | ',' | ';'));
    let lower = value.to_ascii_lowercase();
    if value.is_empty()
        || matches!(
            lower.as_str(),
            "replace_me" | "<redacted>" | "[redacted]" | "<secret>" | "<token>"
        )
        || value.starts_with('$')
        || value.starts_with("DATACORE_")
    {
        return false;
    }
    contains_openai_style_key(value)
        || value.len() >= 12 && value.chars().all(|ch| !ch.is_whitespace())
        || value
            .chars()
            .any(|ch| matches!(ch, '_' | '-' | '.' | '/' | '+' | '='))
}

#[cfg(test)]
mod tests {
    use super::run_bounded_command;
    use std::time::{Duration, Instant};

    #[cfg(unix)]
    #[test]
    fn terminates_a_git_child_that_never_reaches_eof() {
        let started = Instant::now();
        let output = run_bounded_command(
            std::env::current_dir().unwrap().as_path(),
            "git",
            &["hash-object".to_string(), "/dev/zero".to_string()],
            Duration::from_millis(100),
            "stalled Git regression",
        )
        .unwrap();

        assert!(output.timed_out);
        assert!(!output.success());
        assert_eq!(output.exit_code, -1);
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
