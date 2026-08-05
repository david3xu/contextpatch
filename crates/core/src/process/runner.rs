use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::ContextPatchError;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
const MAX_ARGS: usize = 64;
const MAX_ARG_LEN: usize = 4096;
const MAX_OUTPUT_CHARS: usize = 12_000;
const CAPTURE_HEAD_BYTES: usize = 4 * 1024 * 1024;
const CAPTURE_TAIL_BYTES: usize = 4 * 1024 * 1024;
const CAPTURE_OMISSION_MARKER: &[u8] = b"\n[... process output omitted ...]\n";
const STREAM_READ_POLL_INTERVAL: Duration = Duration::from_millis(10);
const STREAM_CANCEL_DRAIN_LIMIT: Duration = Duration::from_millis(100);

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
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
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
        stdout: redact_captured_output(&output.stdout, output.stdout_truncated),
        stderr: redact_captured_output(&output.stderr, output.stderr_truncated),
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
    run_bounded_command_with_wait(
        cwd,
        program,
        args,
        timeout,
        operation_label,
        std::process::Child::try_wait,
    )
}

fn run_bounded_command_with_wait(
    cwd: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    operation_label: &str,
    mut try_wait: impl FnMut(
        &mut std::process::Child,
    ) -> std::io::Result<Option<std::process::ExitStatus>>,
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

    let Some((stdout, stderr)) = child.stdout.take().zip(child.stderr.take()) else {
        let error =
            ContextPatchError::new(format!("failed to capture {operation_label} output pipes"));
        if let Err(cleanup_error) = terminate_child(&mut child, operation_label) {
            return Err(ContextPatchError::new(format!(
                "{error}; cleanup also failed: {cleanup_error}"
            )));
        }
        return Err(error);
    };
    #[cfg(unix)]
    {
        let nonblocking = set_stream_nonblocking(&stdout, "stdout", operation_label)
            .and_then(|()| set_stream_nonblocking(&stderr, "stderr", operation_label));
        if let Err(error) = nonblocking {
            if let Err(cleanup_error) = terminate_child(&mut child, operation_label) {
                return Err(ContextPatchError::new(format!(
                    "{error}; cleanup also failed: {cleanup_error}"
                )));
            }
            return Err(error);
        }
    }
    let readers = StreamReaders::spawn(stdout, stderr, operation_label);

    let status = loop {
        match try_wait(&mut child) {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let termination = terminate_child(&mut child, operation_label);
                let captured = readers.finish(operation_label);
                termination?;
                let (stdout, stderr) = captured?;
                return Ok(BoundedProcessOutput {
                    cwd: cwd.to_path_buf(),
                    exit_code: -1,
                    timed_out: true,
                    duration_ms: started.elapsed().as_millis(),
                    stdout: stdout.bytes,
                    stderr: stderr.bytes,
                    stdout_truncated: stdout.truncated,
                    stderr_truncated: stderr.truncated,
                });
            }
            Ok(None) => thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                let mut message = format!(
                    "failed while waiting for {operation_label} `{}`: {error}",
                    display_command(program, args)
                );
                if let Err(cleanup_error) = terminate_child(&mut child, operation_label) {
                    message.push_str(&format!("; process cleanup also failed: {cleanup_error}"));
                }
                if let Err(cleanup_error) = readers.finish(operation_label) {
                    message.push_str(&format!("; output cleanup also failed: {cleanup_error}"));
                }
                return Err(ContextPatchError::new(message));
            }
        }
    };

    #[cfg(unix)]
    let descendant_cleanup = terminate_remaining_process_group(child.id(), operation_label);
    #[cfg(not(unix))]
    let descendant_cleanup: Result<(), ContextPatchError> = Ok(());
    let captured = readers.finish(operation_label);
    descendant_cleanup?;
    let (stdout, stderr) = captured?;

    Ok(BoundedProcessOutput {
        cwd: cwd.to_path_buf(),
        exit_code: status.code().unwrap_or(-1),
        timed_out: false,
        duration_ms: started.elapsed().as_millis(),
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        stdout_truncated: stdout.truncated,
        stderr_truncated: stderr.truncated,
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

#[cfg(unix)]
fn terminate_remaining_process_group(
    process_group_id: u32,
    operation_label: &str,
) -> Result<(), ContextPatchError> {
    let process_group = i32::try_from(process_group_id).map_err(|_| {
        ContextPatchError::new(format!(
            "failed to identify completed {operation_label} process group"
        ))
    })?;
    let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
    if result == 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(ContextPatchError::new(format!(
        "failed to terminate descendants after {operation_label} exited: {error}"
    )))
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

struct StreamReaders {
    cancellation: Arc<AtomicBool>,
    stdout: Option<thread::JoinHandle<Result<CapturedStream, ContextPatchError>>>,
    stderr: Option<thread::JoinHandle<Result<CapturedStream, ContextPatchError>>>,
}

impl StreamReaders {
    fn spawn(
        stdout: impl Read + Send + 'static,
        stderr: impl Read + Send + 'static,
        operation_label: &str,
    ) -> Self {
        let cancellation = Arc::new(AtomicBool::new(false));
        Self {
            stdout: Some(spawn_stream_reader(
                "stdout",
                stdout,
                operation_label,
                Arc::clone(&cancellation),
            )),
            stderr: Some(spawn_stream_reader(
                "stderr",
                stderr,
                operation_label,
                Arc::clone(&cancellation),
            )),
            cancellation,
        }
    }

    fn cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }

    fn finish(
        mut self,
        operation_label: &str,
    ) -> Result<(CapturedStream, CapturedStream), ContextPatchError> {
        self.cancel();
        let stdout = join_stream_reader(
            self.stdout
                .take()
                .expect("stdout reader is present until finish"),
            operation_label,
        );
        let stderr = join_stream_reader(
            self.stderr
                .take()
                .expect("stderr reader is present until finish"),
            operation_label,
        );
        Ok((stdout?, stderr?))
    }
}

impl Drop for StreamReaders {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(unix)]
fn set_stream_nonblocking(
    stream: &impl std::os::fd::AsRawFd,
    label: &str,
    operation_label: &str,
) -> Result<(), ContextPatchError> {
    let descriptor = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags < 0 {
        return Err(ContextPatchError::new(format!(
            "failed to inspect {operation_label} {label} pipe flags: {}",
            std::io::Error::last_os_error()
        )));
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(ContextPatchError::new(format!(
            "failed to make {operation_label} {label} pipe nonblocking: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

fn spawn_stream_reader(
    label: &'static str,
    mut stream: impl Read + Send + 'static,
    operation_label: &str,
    cancellation: Arc<AtomicBool>,
) -> thread::JoinHandle<Result<CapturedStream, ContextPatchError>> {
    let operation_label = operation_label.to_string();
    thread::spawn(move || {
        capture_stream_with_cancellation(&mut stream, Some(&cancellation)).map_err(|error| {
            ContextPatchError::new(format!("failed to read {operation_label} {label}: {error}"))
        })
    })
}

#[derive(Debug)]
struct CapturedStream {
    bytes: Vec<u8>,
    truncated: bool,
}

#[cfg(test)]
fn capture_stream(mut stream: impl Read) -> Result<CapturedStream, std::io::Error> {
    capture_stream_with_cancellation(&mut stream, None)
}

fn capture_stream_with_cancellation(
    mut stream: impl Read,
    cancellation: Option<&AtomicBool>,
) -> Result<CapturedStream, std::io::Error> {
    let mut head = Vec::with_capacity(CAPTURE_HEAD_BYTES);
    let mut tail = std::collections::VecDeque::with_capacity(CAPTURE_TAIL_BYTES);
    let mut truncated = false;
    let mut chunk = [0_u8; 16 * 1024];
    let mut cancellation_deadline = None;

    loop {
        if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            let deadline = cancellation_deadline
                .get_or_insert_with(|| Instant::now() + STREAM_CANCEL_DRAIN_LIMIT);
            if Instant::now() >= *deadline {
                break;
            }
        }

        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                let mut bytes = &chunk[..count];
                if head.len() < CAPTURE_HEAD_BYTES {
                    let retained = (CAPTURE_HEAD_BYTES - head.len()).min(bytes.len());
                    head.extend_from_slice(&bytes[..retained]);
                    bytes = &bytes[retained..];
                }
                if !bytes.is_empty() {
                    truncated = true;
                    for byte in bytes {
                        if tail.len() == CAPTURE_TAIL_BYTES {
                            tail.pop_front();
                        }
                        tail.push_back(*byte);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if cancellation_deadline.is_some() {
                    break;
                }
                thread::sleep(STREAM_READ_POLL_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }

    if truncated {
        head.extend_from_slice(CAPTURE_OMISSION_MARKER);
        head.extend(tail);
    }

    Ok(CapturedStream {
        bytes: head,
        truncated,
    })
}

fn join_stream_reader(
    reader: thread::JoinHandle<Result<CapturedStream, ContextPatchError>>,
    operation_label: &str,
) -> Result<CapturedStream, ContextPatchError> {
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
    truncate_head_tail(redacted.join("\n"), MAX_OUTPUT_CHARS)
}

fn redact_captured_output(bytes: &[u8], truncated: bool) -> String {
    if !truncated {
        return redact_and_truncate(&String::from_utf8_lossy(bytes));
    }

    let marker_start = CAPTURE_HEAD_BYTES;
    let marker_end = marker_start.saturating_add(CAPTURE_OMISSION_MARKER.len());
    if bytes.get(marker_start..marker_end) != Some(CAPTURE_OMISSION_MARKER) {
        return "[redacted malformed truncated process output]".to_string();
    }

    let head = &bytes[..marker_start];
    let tail = &bytes[marker_end..];
    let complete_head_end = head
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let complete_tail_start = tail
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(tail.len(), |index| index + 1);

    let mut complete_lines = Vec::with_capacity(
        complete_head_end
            .saturating_add(CAPTURE_OMISSION_MARKER.len())
            .saturating_add(tail.len().saturating_sub(complete_tail_start)),
    );
    complete_lines.extend_from_slice(&head[..complete_head_end]);
    complete_lines.extend_from_slice(CAPTURE_OMISSION_MARKER);
    complete_lines.extend_from_slice(&tail[complete_tail_start..]);
    redact_and_truncate(&String::from_utf8_lossy(&complete_lines))
}

fn truncate_head_tail(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }

    let marker = "\n[... output omitted ...]\n";
    let retained = max_chars.saturating_sub(marker.chars().count());
    let head_chars = retained / 2;
    let tail_chars = retained - head_chars;
    let head = text.chars().take(head_chars).collect::<String>();
    let tail = text
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{head}{marker}{tail}")
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
    use super::{
        capture_stream, redact_captured_output, run_bounded_command, run_bounded_command_with_wait,
        truncate_head_tail, CAPTURE_HEAD_BYTES, CAPTURE_OMISSION_MARKER, CAPTURE_TAIL_BYTES,
        MAX_OUTPUT_CHARS,
    };
    #[cfg(unix)]
    use std::cell::Cell;
    #[cfg(unix)]
    use std::io::Write;
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

    #[cfg(unix)]
    #[test]
    fn terminates_descendants_that_retain_output_after_the_child_exits() {
        let started = Instant::now();
        let output = run_bounded_command(
            std::env::current_dir().unwrap().as_path(),
            "sh",
            &[
                "-c".to_string(),
                "sleep 30 & printf 'direct-child-finished\\n'".to_string(),
            ],
            Duration::from_secs(5),
            "post-exit descendant regression",
        )
        .unwrap();

        assert!(output.success());
        assert_eq!(output.stdout, b"direct-child-finished\n");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn stops_reading_when_a_new_session_descendant_retains_output() {
        let helper = std::env::current_exe().unwrap();
        let helper_test = "process::runner::tests::escaped_output_writer_helper";
        let ready = std::env::temp_dir().join(format!(
            "contextpatch-escaped-writer-{}-{}.ready",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let script = concat!(
            "CONTEXTPATCH_ESCAPED_WRITER_READY=\"$3\" ",
            "\"$1\" --ignored --exact \"$2\" --nocapture & ",
            "while [ ! -f \"$3\" ]; do sleep 0.01; done; ",
            "printf 'direct-child-finished\\n'"
        );

        let started = Instant::now();
        let output = run_bounded_command(
            std::env::current_dir().unwrap().as_path(),
            "sh",
            &[
                "-c".to_string(),
                script.to_string(),
                "contextpatch-test".to_string(),
                helper.display().to_string(),
                helper_test.to_string(),
                ready.display().to_string(),
            ],
            Duration::from_secs(5),
            "escaped output writer regression",
        )
        .unwrap();
        let _ = std::fs::remove_file(&ready);

        assert!(output.success());
        assert!(output
            .stdout
            .windows(b"direct-child-finished\n".len())
            .any(|window| window == b"direct-child-finished\n"));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[cfg(unix)]
    #[test]
    fn terminates_and_reaps_the_child_when_waiting_fails() {
        let child_id = Cell::new(None);
        let started = Instant::now();
        let result = run_bounded_command_with_wait(
            std::env::current_dir().unwrap().as_path(),
            "sh",
            &["-c".to_string(), "sleep 30".to_string()],
            Duration::from_secs(5),
            "wait failure regression",
            |child| {
                child_id.set(Some(child.id()));
                Err(std::io::Error::other("injected wait failure"))
            },
        );
        let error = match result {
            Ok(_) => panic!("injected wait failure unexpectedly succeeded"),
            Err(error) => error,
        };

        let child_id = child_id.get().expect("wait hook observed the child");
        assert!(error
            .to_string()
            .contains("failed while waiting for wait failure regression"));
        assert!(started.elapsed() < Duration::from_secs(3));

        let mut status = 0;
        let waited = unsafe { libc::waitpid(child_id as i32, &mut status, libc::WNOHANG) };
        assert_eq!(waited, -1, "child {child_id} was left waitable");
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD),
            "child {child_id} was not reaped"
        );
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess helper for escaped output writer regression"]
    fn escaped_output_writer_helper() {
        let ready = std::env::var_os("CONTEXTPATCH_ESCAPED_WRITER_READY")
            .expect("escaped writer helper requires a ready path");
        assert_ne!(unsafe { libc::setsid() }, -1);
        std::fs::write(ready, std::process::id().to_string()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stdout = std::io::stdout().lock();
        while Instant::now() < deadline {
            if stdout
                .write_all(b"escaped-writer-retains-pipe\n")
                .and_then(|()| stdout.flush())
                .is_err()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn bounded_capture_retains_head_and_tail() {
        let mut input = vec![b'h'; CAPTURE_HEAD_BYTES];
        input.extend(std::iter::repeat_n(b'm', 1024));
        input.extend(std::iter::repeat_n(b't', CAPTURE_TAIL_BYTES));

        let captured = capture_stream(input.as_slice()).unwrap();

        assert!(captured.truncated);
        assert!(captured.bytes.starts_with(b"hhhh"));
        assert!(captured.bytes.ends_with(b"tttt"));
        assert!(captured
            .bytes
            .windows(CAPTURE_OMISSION_MARKER.len())
            .any(|window| window == CAPTURE_OMISSION_MARKER));
        assert!(captured.bytes.len() <= CAPTURE_HEAD_BYTES + CAPTURE_TAIL_BYTES + 64);
    }

    #[test]
    fn truncates_multibyte_output_on_character_boundaries() {
        let text = "prefix ".to_string() + &"🧪".repeat(MAX_OUTPUT_CHARS) + " result marker";
        let truncated = truncate_head_tail(text, MAX_OUTPUT_CHARS);

        assert!(truncated.contains("output omitted"));
        assert!(truncated.ends_with(" result marker"));
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn discards_partial_lines_at_raw_capture_boundaries_before_redaction() {
        let mut input = b"visible-before\nTOKEN=".to_vec();
        input.extend(std::iter::repeat_n(
            b'x',
            CAPTURE_HEAD_BYTES + CAPTURE_TAIL_BYTES + 1024,
        ));
        input.extend_from_slice(b"LEAKME\nvisible-after\n");
        let captured = capture_stream(input.as_slice()).unwrap();

        let rendered = redact_captured_output(&captured.bytes, captured.truncated);

        assert!(rendered.contains("visible-before"));
        assert!(rendered.contains("visible-after"));
        assert!(rendered.contains("process output omitted"));
        assert!(!rendered.contains("LEAKME"));
        assert!(!rendered.contains("TOKEN="));
    }
}
