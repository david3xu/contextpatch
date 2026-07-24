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

pub(crate) fn run_no_shell_command(
    cwd: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    operation_label: &str,
) -> Result<ProcessOutput, ContextPatchError> {
    let mut command = Command::new(program);
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
                let _ = child.kill();
                let _ = child.wait();
                let stdout = join_stream_reader(stdout_reader, operation_label)?;
                let stderr = join_stream_reader(stderr_reader, operation_label)?;
                return Ok(ProcessOutput {
                    cwd: cwd.to_path_buf(),
                    exit_code: -1,
                    timed_out: true,
                    duration_ms: started.elapsed().as_millis(),
                    stdout: redact_and_truncate(&stdout),
                    stderr: redact_and_truncate(&stderr),
                });
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    };

    let stdout = join_stream_reader(stdout_reader, operation_label)?;
    let stderr = join_stream_reader(stderr_reader, operation_label)?;

    Ok(ProcessOutput {
        cwd: cwd.to_path_buf(),
        exit_code: status.code().unwrap_or(-1),
        timed_out: false,
        duration_ms: started.elapsed().as_millis(),
        stdout: redact_and_truncate(&stdout),
        stderr: redact_and_truncate(&stderr),
    })
}

fn spawn_stream_reader(
    label: &'static str,
    mut stream: impl Read + Send + 'static,
    operation_label: &str,
) -> thread::JoinHandle<Result<String, ContextPatchError>> {
    let operation_label = operation_label.to_string();
    thread::spawn(move || {
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).map_err(|error| {
            ContextPatchError::new(format!("failed to read {operation_label} {label}: {error}"))
        })?;
        Ok(String::from_utf8_lossy(&buffer).into_owned())
    })
}

fn join_stream_reader(
    reader: thread::JoinHandle<Result<String, ContextPatchError>>,
    operation_label: &str,
) -> Result<String, ContextPatchError> {
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
    let timeout_secs = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    if timeout_secs == 0 || timeout_secs > MAX_TIMEOUT_SECS {
        return Err(ContextPatchError::new(format!(
            "timeout_secs must be between 1 and {MAX_TIMEOUT_SECS}"
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
