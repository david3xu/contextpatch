pub mod read_command_log {
    pub const NAME: &str = "read_command_log";
}

pub mod run_guarded_command {
    pub const NAME: &str = "run_guarded_command";
}

pub mod image_cleanliness_check_run {
    pub const NAME: &str = "image_cleanliness_check_run";
}

pub mod artifact_python_run {
    pub const NAME: &str = "artifact_python_run";
}

pub mod docker_image_inspect {
    pub const NAME: &str = "docker_image_inspect";
}

pub mod validation_profile_run {
    pub const NAME: &str = "validation_profile_run";
}

pub mod task_image_python_run {
    pub const NAME: &str = "task_image_python_run";
}

pub mod harbor_run_start {
    pub const NAME: &str = "harbor_run_start";
}

pub mod compose_stack_run {
    pub const NAME: &str = "compose_stack_run";
}

pub mod artifact_build_check_run {
    pub const NAME: &str = "artifact_build_check_run";
}

mod containers;
mod jobs;
mod runs;

pub(crate) use containers::{
    call_artifact_build_check_run, call_compose_stack_run, call_docker_image_inspect,
    call_image_cleanliness_check_run, call_task_image_python_run,
};
#[cfg(test)]
use jobs::ACTIVE_BACKGROUND_JOBS;
pub(crate) use jobs::MAX_ACTIVE_BACKGROUND_JOBS;
use jobs::{start_background_job, BackgroundJobOutcome};
pub(crate) use runs::{
    call_artifact_python_run, call_harbor_run_start, call_validation_profile_run,
    MAX_HARBOR_AGENT_LEN, MAX_HARBOR_TIMEOUT_SECS, VALIDATION_PROFILE_NAMES,
};

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use contextpatch_core::process::guarded_command::run_guarded_command;
use contextpatch_core::process::runner::{
    run_bounded_command as run_core_bounded_command, BoundedProcessOutput,
};
use serde_json::Value;

use crate::tools::common::{
    normalize_repo_relative_path, optional_string, optional_u64, required_string,
    required_string_array,
};

static COMMAND_LOG_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn call_run_guarded_command<'a>(
    repository_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let program = required_string(arguments, "program")?;
    let args = required_string_array(arguments, "args")?;
    let cwd = optional_string(arguments, "cwd")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;

    if program == "harbor" && args.first().is_some_and(|arg| arg == "run") {
        return Err(
            "run_guarded_command refused: direct `harbor run` is not available; use \
             harbor_run_start and poll its log_id with read_command_log"
                .to_string(),
        );
    }
    let output = run_guarded_command(
        repository_root.into(),
        cwd.map(Path::new),
        program,
        &args,
        timeout_secs,
    )
    .map_err(|error| format!("run_guarded_command refused: {error}"))?;
    let log_id = write_command_log(&output)
        .map_err(|error| format!("run_guarded_command log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{output}"))
}

pub(crate) fn call_read_command_log(
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let log_id = required_string(arguments, "log_id")?;
    let offset = optional_u64(arguments, "offset")?.unwrap_or(0);
    let max_chars = optional_u64(arguments, "max_chars")?.unwrap_or(12_000);
    if max_chars == 0 || max_chars > 200_000 {
        return Err("read_command_log refused: max_chars must be between 1 and 200000".to_string());
    }

    let status = command_log_status(log_id)?;
    let path = command_log_path(log_id)?;
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("read_command_log refused: failed to read {log_id}: {error}"))?;
    let start = usize::try_from(offset)
        .map_err(|_| "read_command_log refused: offset is too large".to_string())?;
    let chars = text.chars().collect::<Vec<_>>();
    if start > chars.len() {
        return Err(format!(
            "read_command_log refused: offset {offset} is past end of log ({}) characters",
            chars.len()
        ));
    }
    let end = start.saturating_add(max_chars as usize).min(chars.len());
    let mut slice = chars[start..end].iter().collect::<String>();
    if end < chars.len() {
        slice.push_str("\n[truncated]");
    }
    Ok(format!(
        "log_id: {log_id}\nstatus: {}\noffset: {offset}\nchars_returned: {}\ntotal_chars: {}\n{slice}",
        status,
        end - start,
        chars.len()
    ))
}

fn run_bounded_command<'a>(
    tool_name: &str,
    program: &str,
    args: &[String],
    cwd: impl Into<contextpatch_core::process::runner::CommandCwd<'a>>,
    timeout_secs: u64,
) -> Result<BoundedProcessOutput, String> {
    let output = run_core_bounded_command(
        cwd,
        program,
        args,
        Duration::from_secs(timeout_secs),
        tool_name,
    )
    .map_err(|error| format!("{tool_name} refused: {error}"))?;
    if output.timed_out {
        return Err(format!(
            "{tool_name} refused: {program} timed out after {timeout_secs}s\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output)
}

fn format_command_output(
    program: &str,
    args: &[String],
    cwd: &Path,
    output: &BoundedProcessOutput,
) -> String {
    let (stdout, stdout_truncated) =
        truncate_string(String::from_utf8_lossy(&output.stdout).to_string(), 120_000);
    let (stderr, stderr_truncated) =
        truncate_string(String::from_utf8_lossy(&output.stderr).to_string(), 20_000);
    format!(
        "command: {}\ncwd: {}\nexit_code: {}\nsuccess: {}\nstdout_truncated: {}\nstderr_truncated: {}\nstdout:\n{}\nstderr:\n{}",
        std::iter::once(program.to_string())
            .chain(args.iter().map(|arg| shell_display_arg(arg)))
            .collect::<Vec<_>>()
            .join(" "),
        cwd.display(),
        output.exit_code,
        output.success(),
        output.stdout_truncated || stdout_truncated,
        output.stderr_truncated || stderr_truncated,
        stdout,
        stderr
    )
}

fn truncate_string(text: String, max_chars: usize) -> (String, bool) {
    if text.chars().count() <= max_chars {
        return (text, false);
    }
    let retained = max_chars.saturating_sub("\n[truncated]\n".chars().count());
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
    (format!("{head}\n[truncated]\n{tail}"), true)
}

fn extract_field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}: ");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix).map(str::trim))
}

pub(crate) fn write_command_log(text: &str) -> Result<String, String> {
    let log_id = new_command_log_id("cmd")?;
    write_command_log_with_id(&log_id, text)?;
    write_command_status(&log_id, "completed", None, None)?;
    Ok(log_id)
}

fn new_command_log_id(prefix: &str) -> Result<String, String> {
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
    let sequence = COMMAND_LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!(
        "{prefix}-{}-{unique}-{sequence}",
        std::process::id()
    ))
}

fn write_command_log_with_id(log_id: &str, text: &str) -> Result<(), String> {
    let path = command_log_path(log_id)?;
    fs::create_dir_all(command_log_dir()).map_err(|error| {
        format!(
            "failed to create command log directory {}: {error}",
            command_log_dir().display()
        )
    })?;
    fs::write(&path, text)
        .map_err(|error| format!("failed to write command log {log_id}: {error}"))?;
    Ok(())
}

fn write_command_status(
    log_id: &str,
    status: &str,
    exit_code: Option<i32>,
    timed_out: Option<bool>,
) -> Result<(), String> {
    let path = command_status_path(log_id)?;
    let document = serde_json::to_vec(&serde_json::json!({
        "status": status,
        "owner_instance": server_instance_id(),
        "updated_unix_millis": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock error: {error}"))?
            .as_millis(),
        "exit_code": exit_code,
        "timed_out": timed_out
    }))
    .map_err(|error| format!("failed to serialize command status {log_id}: {error}"))?;
    let temporary = path.with_extension(format!("status-{}.tmp", std::process::id()));
    fs::write(&temporary, document)
        .map_err(|error| format!("failed to stage command status {log_id}: {error}"))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("failed to publish command status {log_id}: {error}"))
}

fn command_log_status(log_id: &str) -> Result<String, String> {
    let path = command_status_path(log_id)?;
    if !path.exists() {
        return Ok("completed".to_string());
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("read_command_log refused: failed to read status: {error}"))?;
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("read_command_log refused: invalid status data: {error}"))?;
    let status = document
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "read_command_log refused: status data has no status".to_string())?;
    let owner = document
        .get("owner_instance")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if status == "running" && owner != server_instance_id() {
        return Ok("unknown".to_string());
    }
    Ok(status.to_string())
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

fn command_status_path(log_id: &str) -> Result<PathBuf, String> {
    command_log_path(log_id)?;
    Ok(command_log_dir().join(format!("{log_id}.status.json")))
}

fn command_log_dir() -> PathBuf {
    std::env::temp_dir().join("contextpatch-command-logs")
}

fn server_instance_id() -> &'static str {
    static INSTANCE: OnceLock<String> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        format!("{}-{started}", std::process::id())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The declared list and the match arms must agree in both directions.
    ///
    /// `VALIDATION_PROFILE_NAMES` exists so the refusal text, the capability manifest and the
    /// advertised description stop repeating the same five names. Nothing in the compiler ties it to
    /// the arms that actually resolve, though, so on its own it would be one more hand-written list
    /// claiming which profiles exist, which is the defect it was introduced to remove.
    #[test]
    fn every_declared_validation_profile_resolves_and_nothing_else_does() {
        for name in VALIDATION_PROFILE_NAMES {
            assert!(
                runs::validation_profile(name).is_ok(),
                "{name} is declared but does not resolve to any commands"
            );
        }

        let undeclared = "repo-basic-";
        assert!(
            !VALIDATION_PROFILE_NAMES.contains(&undeclared),
            "the negative case must name a profile that is genuinely undeclared"
        );
        let refusal = match runs::validation_profile(undeclared) {
            Ok(_) => panic!("an undeclared profile must not resolve"),
            Err(refusal) => refusal,
        };
        for name in VALIDATION_PROFILE_NAMES {
            assert!(
                refusal.contains(name),
                "the refusal must list {name}, since it is derived from the declared list"
            );
        }
    }

    #[test]
    fn command_log_ids_are_unique_under_concurrency() {
        let workers = (0..16)
            .map(|_| {
                thread::spawn(|| {
                    (0..128)
                        .map(|_| new_command_log_id("concurrent-test").unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let ids = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        let unique = ids.iter().collect::<std::collections::HashSet<_>>();

        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn command_format_reports_server_side_truncation() {
        let output = BoundedProcessOutput {
            cwd: PathBuf::from("."),
            exit_code: 0,
            timed_out: false,
            duration_ms: 1,
            stdout: vec![b'x'; 120_001],
            stderr: vec![b'y'; 20_001],
            stdout_truncated: false,
            stderr_truncated: false,
        };

        let formatted = format_command_output("python3", &[], Path::new("."), &output);

        assert!(formatted.contains("stdout_truncated: true"));
        assert!(formatted.contains("stderr_truncated: true"));
        assert_eq!(formatted.matches("[truncated]").count(), 2);
    }

    #[test]
    fn background_worker_panics_become_terminal_failures() {
        let log_id = start_background_job(
            "test_background_worker",
            "panic-test",
            "status: running",
            |_| -> Result<BackgroundJobOutcome, String> {
                panic!("synthetic background failure");
            },
        )
        .unwrap();
        let arguments = serde_json::json!({"log_id": log_id});
        let arguments = arguments.as_object().unwrap();

        for _ in 0..100 {
            let log = call_read_command_log(arguments).unwrap();
            if log.contains("status: failed") {
                assert!(log.contains("worker panicked: synthetic background failure"));
                for _ in 0..100 {
                    if ACTIVE_BACKGROUND_JOBS.load(Ordering::Acquire) == 0 {
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                panic!("background permit was not released after a worker panic");
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("background panic did not reach a terminal failure state");
    }
}
