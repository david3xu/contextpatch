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

use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
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
use serde_json::{json, Value};

use crate::tools::common::{
    normalize_repo_relative_path, optional_bool, optional_string, optional_string_array,
    optional_u64, required_string, required_string_array,
};

pub(crate) const MAX_ACTIVE_BACKGROUND_JOBS: usize = 2;
static ACTIVE_BACKGROUND_JOBS: AtomicUsize = AtomicUsize::new(0);
static COMMAND_LOG_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct BackgroundJobPermit;

impl BackgroundJobPermit {
    fn try_acquire(tool_name: &str) -> Result<Self, String> {
        let mut active = ACTIVE_BACKGROUND_JOBS.load(Ordering::Acquire);
        loop {
            if active >= MAX_ACTIVE_BACKGROUND_JOBS {
                return Err(format!(
                    "{tool_name} refused: at most {MAX_ACTIVE_BACKGROUND_JOBS} Harbor, task-image, \
                     or validation-profile jobs may run at once; poll existing log_ids before \
                     starting another job"
                ));
            }
            match ACTIVE_BACKGROUND_JOBS.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Self),
                Err(observed) => active = observed,
            }
        }
    }
}

impl Drop for BackgroundJobPermit {
    fn drop(&mut self) {
        ACTIVE_BACKGROUND_JOBS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct BackgroundJobOutcome {
    status: &'static str,
    exit_code: i32,
    timed_out: bool,
    log: String,
}

impl BackgroundJobOutcome {
    fn failed(message: impl Into<String>) -> Self {
        Self {
            status: "failed",
            exit_code: -1,
            timed_out: false,
            log: json!({
                "status": "failed",
                "error": message.into()
            })
            .to_string(),
        }
    }
}

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

fn start_background_job<F>(
    tool_name: &'static str,
    log_prefix: &str,
    initial_log: &str,
    worker: F,
) -> Result<String, String>
where
    F: FnOnce(&str) -> Result<BackgroundJobOutcome, String> + Send + 'static,
{
    let permit = BackgroundJobPermit::try_acquire(tool_name)?;
    let log_id =
        new_command_log_id(log_prefix).map_err(|error| format!("{tool_name} refused: {error}"))?;
    write_command_log_with_id(&log_id, initial_log)
        .and_then(|_| write_command_status(&log_id, "running", None, Some(false)))
        .map_err(|error| format!("{tool_name} refused: {error}"))?;

    let worker_log_id = log_id.clone();
    let spawn = thread::Builder::new()
        .name(format!("contextpatch-{log_id}"))
        .spawn(move || {
            let _permit = permit;
            let outcome = match catch_unwind(AssertUnwindSafe(|| worker(&worker_log_id))) {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(error)) => BackgroundJobOutcome::failed(error),
                Err(payload) => BackgroundJobOutcome::failed(format!(
                    "{tool_name} worker panicked: {}",
                    panic_payload(payload)
                )),
            };

            if let Err(error) = write_command_log_with_id(&worker_log_id, &outcome.log) {
                eprintln!("contextpatch: failed to write {worker_log_id} result: {error}");
                let _ = write_command_status(&worker_log_id, "failed", None, None);
                return;
            }
            if let Err(error) = write_command_status(
                &worker_log_id,
                outcome.status,
                Some(outcome.exit_code),
                Some(outcome.timed_out),
            ) {
                eprintln!("contextpatch: failed to finalize {worker_log_id}: {error}");
            }
        });

    if let Err(error) = spawn {
        let failure = format!("{tool_name} failed to spawn background worker: {error}");
        let _ = write_command_log_with_id(&log_id, &failure);
        let _ = write_command_status(&log_id, "failed", None, None);
        return Err(format!("{tool_name} refused: {failure}"));
    }
    Ok(log_id)
}

fn panic_payload(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

pub(crate) fn call_task_image_python_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let script = required_string(arguments, "script")?;
    let program = optional_string(arguments, "program")?.unwrap_or("python3");
    let args = optional_string_array(arguments, "args")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;
    let build_timeout_secs = optional_u64(arguments, "build_timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    let plan = contextpatch_core::process::task_image::plan_task_image_python_run(
        repo_root,
        script,
        program,
        &args,
        timeout_secs,
        build_timeout_secs,
    )
    .map_err(|error| format!("task_image_python_run refused: {error}"))?;

    if dry_run {
        return serde_json::to_string_pretty(&serde_json::json!({
            "tool": crate::tools::task_image_python_run::NAME,
            "dry_run": true,
            "script": plan.script(),
            "cache_image": plan.cache_image(),
            "image": plan.image(),
            "container": plan.container(),
            "build": {
                "program": "docker",
                "args": plan.build_args(),
                "timeout_secs": plan.build_timeout().as_secs()
            },
            "run": {
                "program": "docker",
                "args": plan.run_args(),
                "timeout_secs": plan.run_timeout().as_secs()
            },
            "cleanup": {
                "container_after_timeout": {
                    "program": "docker",
                    "args": plan.container_cleanup_args()
                },
                "execution_image": {
                    "program": "docker",
                    "args": plan.image_cleanup_args()
                }
            },
            "repository_mount": "read-only",
            "network": "none",
            "required_confirm_for_run": contextpatch_core::process::task_image::CONFIRMATION
        }))
        .map_err(|error| format!("task_image_python_run refused: {error}"));
    }

    if confirm != Some(contextpatch_core::process::task_image::CONFIRMATION) {
        return Err(format!(
            "task_image_python_run refused: dry_run=false requires confirm: {:?}",
            contextpatch_core::process::task_image::CONFIRMATION
        ));
    }
    let initial_log = serde_json::to_string_pretty(&json!({
        "tool": crate::tools::task_image_python_run::NAME,
        "status": "running",
        "script": plan.script(),
        "image": plan.image(),
        "container": plan.container()
    }))
    .map_err(|error| format!("task_image_python_run refused: {error}"))?;
    let worker_plan = plan.clone();
    let log_id = start_background_job(
        crate::tools::task_image_python_run::NAME,
        "task-image",
        &initial_log,
        move |_| {
            let result = contextpatch_core::process::task_image::run_task_image_python(
                &worker_plan,
                Some(contextpatch_core::process::task_image::CONFIRMATION),
            )
            .map_err(|error| format!("task_image_python_run failed: {error}"))?;
            let timed_out = result.build.timed_out
                || result.run.as_ref().is_some_and(|command| command.timed_out)
                || result
                    .container_cleanup
                    .as_ref()
                    .is_some_and(|command| command.timed_out)
                || result
                    .image_cleanup
                    .as_ref()
                    .is_some_and(|command| command.timed_out);
            let terminal_status = if timed_out {
                "timed_out"
            } else if result.success() {
                "completed"
            } else {
                "failed"
            };
            let result_value = json!({
                "tool": crate::tools::task_image_python_run::NAME,
                "status": terminal_status,
                "dry_run": false,
                "script": worker_plan.script(),
                "cache_image": worker_plan.cache_image(),
                "image": worker_plan.image(),
                "container": worker_plan.container(),
                "success": result.success(),
                "build": task_image_command_value(&result.build),
                "run": result.run.as_ref().map(task_image_command_value),
                "container_cleanup": result
                    .container_cleanup
                    .as_ref()
                    .map(task_image_command_value),
                "image_cleanup": result.image_cleanup.as_ref().map(task_image_command_value),
                "repository_mount": "read-only",
                "network": "none"
            });
            let last_command = (!result.build.success())
                .then_some(&result.build)
                .or_else(|| result.run.as_ref().filter(|command| !command.success()))
                .or_else(|| {
                    result
                        .container_cleanup
                        .as_ref()
                        .filter(|command| !command.success())
                })
                .or_else(|| {
                    result
                        .image_cleanup
                        .as_ref()
                        .filter(|command| !command.success())
                })
                .or(result.run.as_ref())
                .unwrap_or(&result.build);
            Ok(BackgroundJobOutcome {
                status: terminal_status,
                exit_code: last_command.exit_code,
                timed_out,
                log: serde_json::to_string_pretty(&result_value)
                    .map_err(|error| format!("task_image_python_run failed: {error}"))?,
            })
        },
    )?;
    serde_json::to_string_pretty(&json!({
        "tool": crate::tools::task_image_python_run::NAME,
        "status": "running",
        "log_id": log_id,
        "poll_with": {
            "action": crate::tools::read_command_log::NAME,
            "arguments": {"log_id": log_id}
        },
        "restart_semantics": "Polling never restarts work. If the MCP server restarts while status is running, read_command_log reports unknown; inspect current Docker state before retrying."
    }))
    .map_err(|error| format!("task_image_python_run refused: {error}"))
}

pub(crate) fn call_harbor_run_start(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let project = optional_string(arguments, "project")?.unwrap_or("task");
    let agent = required_string(arguments, "agent")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(3600);
    if timeout_secs == 0 || timeout_secs > 3600 {
        return Err(
            "harbor_run_start refused: timeout_secs must be between 1 and 3600".to_string(),
        );
    }
    if agent.is_empty()
        || agent.len() > 128
        || agent.starts_with('-')
        || !agent
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(
            "harbor_run_start refused: agent must not start with `-` and may contain only ASCII letters, digits, `.`, `_`, or `-`"
                .to_string(),
        );
    }
    if project.contains('\\') {
        return Err(
            "harbor_run_start refused: project must use `/` separators and contain no backslashes"
                .to_string(),
        );
    }
    let project = normalize_repo_relative_path(crate::tools::harbor_run_start::NAME, project)?;
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("harbor_run_start refused: {error}"))?;
    validate_harbor_project_directory(&root, Path::new(&project))?;
    let command_args = vec![
        "run".to_string(),
        "-p".to_string(),
        project.clone(),
        "--agent".to_string(),
        agent.to_string(),
    ];
    let initial_log = format!(
        "Harbor run is active.\ncommand: harbor run -p {} --agent {}\n",
        shell_display_arg(&project),
        shell_display_arg(agent)
    );
    let worker_root = root;
    let log_id = start_background_job(
        crate::tools::harbor_run_start::NAME,
        "harbor",
        &initial_log,
        move |worker_log_id| {
            let output = run_guarded_command(
                &worker_root,
                None,
                "harbor",
                &command_args,
                Some(timeout_secs),
            )
            .map_err(|error| format!("harbor_run_start failed: {error}"))?;
            let exit_code = extract_field(&output, "exit_code")
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or(-1);
            let timed_out = extract_field(&output, "timed_out") == Some("true");
            let status = if timed_out {
                "timed_out"
            } else if exit_code == 0 {
                "completed"
            } else {
                "failed"
            };
            let document = json!({
                "tool": crate::tools::harbor_run_start::NAME,
                "log_id": worker_log_id,
                "status": status,
                "command_output": output,
                "harbor": crate::tools::harbor::structured_evidence(&worker_root, &output)
            });
            Ok(BackgroundJobOutcome {
                status,
                exit_code,
                timed_out,
                log: serde_json::to_string_pretty(&document)
                    .map_err(|error| format!("harbor_run_start failed: {error}"))?,
            })
        },
    )?;

    serde_json::to_string_pretty(&json!({
        "tool": crate::tools::harbor_run_start::NAME,
        "status": "running",
        "log_id": log_id,
        "poll_with": {
            "action": crate::tools::read_command_log::NAME,
            "arguments": {"log_id": log_id}
        },
        "restart_semantics": "Polling never restarts work. If the MCP server restarts while status is running, read_command_log reports unknown; inspect current Harbor job state before retrying."
    }))
    .map_err(|error| format!("harbor_run_start refused: {error}"))
}

fn validate_harbor_project_directory(root: &Path, project: &Path) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in project.components() {
        let Component::Normal(component) = component else {
            return Err(
                "harbor_run_start refused: project must be a normalized repository-relative path"
                    .to_string(),
            );
        };
        if component.to_string_lossy().starts_with('-') {
            return Err(
                "harbor_run_start refused: project path components must not start with `-`"
                    .to_string(),
            );
        }
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            format!(
                "harbor_run_start refused: failed to inspect project `{}`: {error}",
                project.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "harbor_run_start refused: project `{}` must not contain symlink components",
                project.display()
            ));
        }
    }
    let resolved = current.canonicalize().map_err(|error| {
        format!(
            "harbor_run_start refused: failed to resolve project `{}`: {error}",
            project.display()
        )
    })?;
    if !resolved.starts_with(root) || !resolved.is_dir() {
        return Err(format!(
            "harbor_run_start refused: project `{}` is not an existing repository directory",
            project.display()
        ));
    }
    Ok(())
}

fn task_image_command_value(
    result: &contextpatch_core::process::task_image::TaskImageCommandResult,
) -> Value {
    serde_json::json!({
        "exit_code": result.exit_code,
        "timed_out": result.timed_out,
        "success": result.success(),
        "duration_ms": result.duration_ms,
        "stdout": result.stdout,
        "stdout_truncated": result.stdout_truncated,
        "stderr": result.stderr,
        "stderr_truncated": result.stderr_truncated
    })
}

pub(crate) fn call_image_cleanliness_check_run(
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "run image cleanliness check";

    let image = required_string(arguments, "image")?;
    let filename = optional_string(arguments, "filename")?.unwrap_or("solve.sh");
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(120);

    validate_docker_image_ref(image, crate::tools::image_cleanliness_check_run::NAME)?;
    validate_find_filename(filename, crate::tools::image_cleanliness_check_run::NAME)?;
    if timeout_secs == 0 || timeout_secs > 600 {
        return Err(
            "image_cleanliness_check_run refused: timeout_secs must be between 1 and 600"
                .to_string(),
        );
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "image_cleanliness_check_run refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }

    let args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--network".to_string(),
        "none".to_string(),
        "--entrypoint".to_string(),
        "find".to_string(),
        image.to_string(),
        "/".to_string(),
        "-name".to_string(),
        filename.to_string(),
    ];
    if dry_run {
        return serde_json::to_string_pretty(&serde_json::json!({
            "tool": crate::tools::image_cleanliness_check_run::NAME,
            "dry_run": true,
            "would_run": std::iter::once("docker".to_string()).chain(args.iter().cloned()).collect::<Vec<_>>(),
            "required_confirm_for_run": CONFIRMATION,
            "expected_clean_stdout": ""
        }))
        .map_err(|error| format!("image_cleanliness_check_run refused: {error}"));
    }

    let output = run_bounded_docker(
        crate::tools::image_cleanliness_check_run::NAME,
        &args,
        timeout_secs,
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let matches = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    let clean = output.success() && matches.is_empty() && !output.stdout_truncated;
    serde_json::to_string_pretty(&serde_json::json!({
        "tool": crate::tools::image_cleanliness_check_run::NAME,
        "dry_run": false,
        "ran": true,
        "image": image,
        "filename": filename,
        "exit_code": output.exit_code,
        "clean": clean,
        "matches": matches,
        "stdout": stdout,
        "stdout_truncated": output.stdout_truncated,
        "stderr": stderr,
        "stderr_truncated": output.stderr_truncated
    }))
    .map_err(|error| format!("image_cleanliness_check_run refused: {error}"))
}

pub(crate) fn call_docker_image_inspect(
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    const CONFIRMATION: &str = "inspect docker image";

    let image = required_string(arguments, "image")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(120);

    validate_docker_image_ref(image, crate::tools::docker_image_inspect::NAME)?;
    if timeout_secs == 0 || timeout_secs > 600 {
        return Err(
            "docker_image_inspect refused: timeout_secs must be between 1 and 600".to_string(),
        );
    }
    if !dry_run && confirm != Some(CONFIRMATION) {
        return Err(format!(
            "docker_image_inspect refused: dry_run=false requires confirm: {CONFIRMATION:?}"
        ));
    }
    let args = vec![
        "image".to_string(),
        "inspect".to_string(),
        image.to_string(),
    ];
    if dry_run {
        return serde_json::to_string_pretty(&serde_json::json!({
            "tool": crate::tools::docker_image_inspect::NAME,
            "dry_run": true,
            "would_run": std::iter::once("docker".to_string()).chain(args.iter().cloned()).collect::<Vec<_>>(),
            "required_confirm_for_run": CONFIRMATION
        }))
        .map_err(|error| format!("docker_image_inspect refused: {error}"));
    }
    let output = run_bounded_docker(
        crate::tools::docker_image_inspect::NAME,
        &args,
        timeout_secs,
    )?;
    let (stdout, stdout_truncated) =
        truncate_string(String::from_utf8_lossy(&output.stdout).to_string(), 120_000);
    let (stderr, stderr_truncated) =
        truncate_string(String::from_utf8_lossy(&output.stderr).to_string(), 20_000);
    serde_json::to_string_pretty(&serde_json::json!({
        "tool": crate::tools::docker_image_inspect::NAME,
        "dry_run": false,
        "ran": true,
        "image": image,
        "exit_code": output.exit_code,
        "success": output.success(),
        "stdout": stdout,
        "stdout_truncated": output.stdout_truncated || stdout_truncated,
        "stderr": stderr,
        "stderr_truncated": output.stderr_truncated || stderr_truncated
    }))
    .map_err(|error| format!("docker_image_inspect refused: {error}"))
}

pub(crate) fn call_artifact_python_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let program = optional_string(arguments, "program")?.unwrap_or("python3");
    if program != "python3" && program != "python" {
        return Err(
            "artifact_python_run refused: program must be `python3` or `python`".to_string(),
        );
    }
    let script = required_string(arguments, "script")?;
    let args = optional_string_array(arguments, "args")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(120);
    if timeout_secs == 0 || timeout_secs > 600 {
        return Err(
            "artifact_python_run refused: timeout_secs must be between 1 and 600".to_string(),
        );
    }
    for arg in &args {
        if arg.contains('\0') || arg.len() > 1000 {
            return Err("artifact_python_run refused: args must not contain NUL and must be at most 1000 bytes each".to_string());
        }
    }
    let artifact_root =
        crate::tools::files::artifact_root(repo_root, crate::tools::artifact_python_run::NAME)?;
    let relative = crate::tools::common::validate_relative_path(
        crate::tools::artifact_python_run::NAME,
        script,
    )?;
    let script_path = artifact_root.join(&relative);
    let resolved_script = script_path.canonicalize().map_err(|error| {
        format!(
            "artifact_python_run refused: failed to resolve artifact script `{script}`: {error}"
        )
    })?;
    if !resolved_script.starts_with(&artifact_root) {
        return Err(
            "artifact_python_run refused: script resolves outside artifact root".to_string(),
        );
    }
    if !resolved_script.is_file() {
        return Err(format!(
            "artifact_python_run refused: `{script}` is not an existing artifact file"
        ));
    }
    let mut command_args = vec![resolved_script.display().to_string()];
    command_args.extend(args);
    let output = run_bounded_command(
        crate::tools::artifact_python_run::NAME,
        program,
        &command_args,
        &artifact_root,
        timeout_secs,
    )?;
    let text = format_command_output(program, &command_args, &artifact_root, &output);
    let log_id = write_command_log(&text)
        .map_err(|error| format!("artifact_python_run log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{text}"))
}

pub(crate) fn call_validation_profile_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let profile = required_string(arguments, "profile")?;
    if let Some(timeout_secs) = optional_u64(arguments, "timeout_secs")? {
        if timeout_secs == 0 || timeout_secs > 600 {
            return Err(
                "validation_profile_run refused: timeout_secs must be between 1 and 600"
                    .to_string(),
            );
        }
    }
    validation_profile(profile)?;

    let worker_root = repo_root.to_path_buf();
    let worker_arguments = arguments.clone();
    let profile_name = profile.to_string();
    let initial_log = json!({
        "tool": crate::tools::validation_profile_run::NAME,
        "status": "running",
        "profile": profile
    })
    .to_string();
    let log_id = start_background_job(
        crate::tools::validation_profile_run::NAME,
        "validation",
        &initial_log,
        move |_| {
            let log = run_validation_profile_sync(&worker_root, &worker_arguments)?;
            let failed = extract_field(&log, "failed") == Some("true");
            let timed_out = log.contains("| timed_out: true |");
            Ok(BackgroundJobOutcome {
                status: if timed_out {
                    "timed_out"
                } else if failed {
                    "failed"
                } else {
                    "completed"
                },
                exit_code: if failed { 1 } else { 0 },
                timed_out,
                log,
            })
        },
    )?;
    serde_json::to_string_pretty(&json!({
        "tool": crate::tools::validation_profile_run::NAME,
        "profile": profile_name,
        "status": "running",
        "log_id": log_id,
        "poll_with": {
            "action": crate::tools::read_command_log::NAME,
            "arguments": {"log_id": log_id}
        },
        "restart_semantics": "Polling never restarts work. If the MCP server restarts while status is running, read_command_log reports unknown; inspect repository and external job state before retrying."
    }))
    .map_err(|error| format!("validation_profile_run refused: {error}"))
}

fn run_validation_profile_sync(
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
    let mut harbor_oracle_rewards = Vec::new();
    let mut harbor_nop_rewards = Vec::new();
    let mut harbor_missing_rewards = Vec::new();

    for (index, command) in commands.iter().enumerate() {
        ran += 1;
        let timeout_secs = timeout_override.or(command.timeout_secs);
        let effective_timeout_secs = timeout_secs.unwrap_or(120);
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
        if profile == "dynamo-harbor-task" && command.program == "harbor" {
            let agent = harbor_agent(&command.args).unwrap_or("unknown");
            // Prefer the result file Harbor writes over its rendered table: the table splits the word
            // "reward" and its value across two lines, which no single-line scan can read.
            let rewards = crate::tools::harbor::rewards_for_run(repo_root, &output)
                .or_else(|| crate::tools::harbor::rewards_from_output(&output));
            match (rewards, agent) {
                (Some(values), "oracle") => harbor_oracle_rewards.extend(values),
                (Some(values), "nop") => harbor_nop_rewards.extend(values),
                (Some(_), _) => harbor_missing_rewards.push(format!(
                    "{}. {} | unrecognized_agent: {agent}",
                    index + 1,
                    command.display()
                )),
                (None, _) => harbor_missing_rewards.push(format!(
                    "{}. {} | reward: missing | log_id: {log_id}",
                    index + 1,
                    command.display()
                )),
            }
        }
        lines.push(format!(
            "{}. {} | timeout_secs: {effective_timeout_secs} | exit_code: {exit_code} | timed_out: {timed_out} | duration_ms: {duration_ms} | log_id: {log_id}",
            index + 1,
            command.display()
        ));
        if command_failed && stop_on_failure {
            lines.push(format!("stopped_after_failure: {}", index + 1));
            break;
        }
    }

    if profile == "dynamo-harbor-task" {
        let harbor_oracle_all_one = harbor_oracle_rewards.len() == 2
            && harbor_oracle_rewards
                .iter()
                .all(|reward| (*reward - 1.0).abs() <= f64::EPSILON);
        let harbor_nop_all_below_one =
            harbor_nop_rewards.len() == 2 && harbor_nop_rewards.iter().all(|reward| *reward < 1.0);
        let harbor_oracle_deterministic = rewards_deterministic(&harbor_oracle_rewards);
        let harbor_nop_deterministic = rewards_deterministic(&harbor_nop_rewards);
        let harbor_passed = !failed
            && harbor_oracle_all_one
            && harbor_nop_all_below_one
            && harbor_oracle_deterministic
            && harbor_nop_deterministic
            && harbor_missing_rewards.is_empty();
        failed |= !harbor_passed;
        let summary = serde_json::json!({
            "profile": "dynamo-harbor-task",
            "oracle_rewards": harbor_oracle_rewards,
            "nop_rewards": harbor_nop_rewards,
            "oracle_all_one": harbor_oracle_all_one,
            "nop_all_below_one": harbor_nop_all_below_one,
            "oracle_deterministic": harbor_oracle_deterministic,
            "nop_deterministic": harbor_nop_deterministic,
            "missing_rewards": harbor_missing_rewards,
            "passed": harbor_passed
        });
        lines.push(format!("harbor_summary: {summary}"));
    }
    lines.insert(3, format!("commands_run: {ran}"));
    lines.insert(4, format!("failed: {failed}"));
    lines.push(format!("duration_ms: {}", started.elapsed().as_millis()));
    Ok(lines.join("\n"))
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
                timeout_secs: Some(3600),
            },
            ProfileCommand {
                program: "harbor",
                args: vec!["run", "-p", "task", "--agent", "nop"],
                cwd: None,
                timeout_secs: Some(3600),
            },
            ProfileCommand {
                program: "harbor",
                args: vec!["run", "-p", "task", "--agent", "oracle"],
                cwd: None,
                timeout_secs: Some(3600),
            },
            ProfileCommand {
                program: "harbor",
                args: vec!["run", "-p", "task", "--agent", "nop"],
                cwd: None,
                timeout_secs: Some(3600),
            },
        ]),
        _ => Err(format!(
            "validation_profile_run refused: unknown profile `{profile}`; expected repo-basic, rust-workspace, datacore-vscode, datacore-m6-vscode, or dynamo-harbor-task"
        )),
    }
}

fn validate_docker_image_ref(image: &str, tool_name: &str) -> Result<(), String> {
    if image.is_empty()
        || image.len() > 300
        || image.starts_with('-')
        || image.contains("..")
        || !image
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '/' | '_' | '-' | ':' | '@'))
    {
        return Err(format!(
            "{tool_name} refused: image must be a Docker image reference, not a shell fragment"
        ));
    }
    Ok(())
}

fn validate_find_filename(filename: &str, tool_name: &str) -> Result<(), String> {
    if filename.is_empty()
        || filename.len() > 128
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
        || filename.starts_with('-')
    {
        return Err(format!(
            "{tool_name} refused: filename must be a simple file name"
        ));
    }
    Ok(())
}

fn run_bounded_docker(
    tool_name: &str,
    args: &[String],
    timeout_secs: u64,
) -> Result<BoundedProcessOutput, String> {
    run_bounded_command(tool_name, "docker", args, Path::new("/"), timeout_secs)
}

fn run_bounded_command(
    tool_name: &str,
    program: &str,
    args: &[String],
    cwd: &Path,
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

fn harbor_agent<'a>(args: &'a [&'static str]) -> Option<&'a str> {
    args.windows(2)
        .find_map(|window| (window[0] == "--agent").then_some(window[1]))
}

fn rewards_deterministic(rewards: &[f64]) -> bool {
    match rewards.split_first() {
        Some((first, rest)) if !rest.is_empty() => rest
            .iter()
            .all(|reward| (*reward - *first).abs() <= f64::EPSILON),
        _ => false,
    }
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
