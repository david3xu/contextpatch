use std::path::Path;

use serde_json::{json, Value};

use super::*;
use crate::tools::common::{
    optional_bool, optional_string, optional_string_array, optional_u64, required_string,
};

/// Plan or start one named Compose stack proof.
///
/// The argv is derived in core from the action name alone, so no caller-supplied Docker arguments
/// reach the child. Execution is asynchronous for the same reason Harbor runs are: a stack proof
/// routinely outlives any reply deadline, and the 600-second guarded-command cap does not apply to
/// this path because it never passes through the guarded-command allowlist.
pub(crate) fn call_compose_stack_run<'a>(
    repository_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let repository_root = repository_root.into();
    // Checked before any argument is read, so planning against a selected repository is refused
    // exactly where executing would be.
    contextpatch_core::process::compose_stack::ensure_compose_root_is_addressable(repository_root)
        .map_err(|error| format!("compose_stack_run refused: {error}"))?;

    let action = required_string(arguments, "action")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    let plan = contextpatch_core::process::compose_stack::plan_compose_stack_run(
        repository_root,
        action,
        timeout_secs,
    )
    .map_err(|error| format!("compose_stack_run refused: {error}"))?;

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": crate::tools::compose_stack_run::NAME,
            "dry_run": true,
            "action": plan.action(),
            "compose_file": plan.compose_file(),
            "project_name": plan.project_name(),
            "up": {
                "program": "docker",
                "args": plan.up_args(),
                "timeout_secs": plan.up_timeout().as_secs()
            },
            "teardown": {
                "program": "docker",
                "args": plan.down_args(),
                "timeout_secs": plan.down_timeout().as_secs()
            },
            "network": "enabled",
            "required_confirm_for_run": contextpatch_core::process::compose_stack::CONFIRMATION
        }))
        .map_err(|error| format!("compose_stack_run refused: {error}"));
    }

    if confirm != Some(contextpatch_core::process::compose_stack::CONFIRMATION) {
        return Err(format!(
            "compose_stack_run refused: dry_run=false requires confirm: {:?}",
            contextpatch_core::process::compose_stack::CONFIRMATION
        ));
    }

    let initial_log = serde_json::to_string_pretty(&json!({
        "tool": crate::tools::compose_stack_run::NAME,
        "status": "running",
        "action": plan.action(),
        "compose_file": plan.compose_file(),
        "project_name": plan.project_name()
    }))
    .map_err(|error| format!("compose_stack_run refused: {error}"))?;

    let worker_plan = plan.clone();
    let log_id = start_background_job(
        crate::tools::compose_stack_run::NAME,
        "compose-stack",
        &initial_log,
        move |_| {
            let result = contextpatch_core::process::compose_stack::run_compose_stack(
                &worker_plan,
                Some(contextpatch_core::process::compose_stack::CONFIRMATION),
            )
            .map_err(|error| format!("compose_stack_run failed: {error}"))?;

            let terminal_status = if result.up.timed_out {
                "timed_out"
            } else if result.success() {
                "completed"
            } else {
                "failed"
            };
            let result_value = json!({
                "tool": crate::tools::compose_stack_run::NAME,
                "status": terminal_status,
                "dry_run": false,
                "action": worker_plan.action(),
                "compose_file": worker_plan.compose_file(),
                "project_name": worker_plan.project_name(),
                "up": compose_command_value(&result.up),
                "teardown": result.teardown.as_ref().map(compose_command_value),
                // Surfaced separately because a leaked stack must not be hidden by a passing proof.
                "teardown_clean": result.teardown_clean()
            });
            let log = serde_json::to_string_pretty(&result_value)
                .map_err(|error| format!("compose_stack_run failed: {error}"))?;
            Ok(BackgroundJobOutcome {
                log,
                status: terminal_status,
                exit_code: result.up.exit_code,
                timed_out: result.up.timed_out,
            })
        },
    )?;

    Ok(format!(
        "log_id: {log_id}\n{}",
        serde_json::to_string_pretty(&json!({
            "tool": crate::tools::compose_stack_run::NAME,
            "status": "running",
            "action": plan.action(),
            "project_name": plan.project_name(),
            "poll_with": crate::tools::read_command_log::NAME
        }))
        .map_err(|error| format!("compose_stack_run refused: {error}"))?
    ))
}

/// Plan or start one artifact build plus its import smoke check.
///
/// The caller names a Dockerfile and the arguments for the built image; every Docker flag, the tag,
/// and the networkless smoke invocation are derived here.
pub(crate) fn call_artifact_build_check_run<'a>(
    repository_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let repository_root = repository_root.into();
    contextpatch_core::process::artifact_build::ensure_artifact_root_is_addressable(
        repository_root,
    )
    .map_err(|error| format!("artifact_build_check_run refused: {error}"))?;

    let dockerfile = required_string(arguments, "dockerfile")?;
    let context = optional_string(arguments, "context")?;
    let smoke_args = optional_string_array(arguments, "smoke_args")?;
    let build_timeout_secs = optional_u64(arguments, "build_timeout_secs")?;
    let smoke_timeout_secs = optional_u64(arguments, "smoke_timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    let plan = contextpatch_core::process::artifact_build::plan_artifact_build_check(
        repository_root,
        dockerfile,
        context,
        &smoke_args,
        build_timeout_secs,
        smoke_timeout_secs,
    )
    .map_err(|error| format!("artifact_build_check_run refused: {error}"))?;

    if dry_run {
        return serde_json::to_string_pretty(&json!({
            "tool": crate::tools::artifact_build_check_run::NAME,
            "dry_run": true,
            "dockerfile": plan.dockerfile(),
            "context": plan.context(),
            "tag": plan.tag(),
            "build": {
                "program": "docker",
                "args": plan.build_args(),
                "timeout_secs": plan.build_timeout().as_secs()
            },
            "smoke": {
                "program": "docker",
                "args": plan.smoke_args(),
                "timeout_secs": plan.smoke_timeout().as_secs()
            },
            "image_cleanup": {
                "program": "docker",
                "args": plan.image_cleanup_args()
            },
            "build_network": "enabled",
            "smoke_network": "none",
            "required_confirm_for_run":
                contextpatch_core::process::artifact_build::CONFIRMATION
        }))
        .map_err(|error| format!("artifact_build_check_run refused: {error}"));
    }

    if confirm != Some(contextpatch_core::process::artifact_build::CONFIRMATION) {
        return Err(format!(
            "artifact_build_check_run refused: dry_run=false requires confirm: {:?}",
            contextpatch_core::process::artifact_build::CONFIRMATION
        ));
    }

    let initial_log = serde_json::to_string_pretty(&json!({
        "tool": crate::tools::artifact_build_check_run::NAME,
        "status": "running",
        "dockerfile": plan.dockerfile(),
        "tag": plan.tag()
    }))
    .map_err(|error| format!("artifact_build_check_run refused: {error}"))?;

    let worker_plan = plan.clone();
    let log_id = start_background_job(
        crate::tools::artifact_build_check_run::NAME,
        "artifact-build",
        &initial_log,
        move |_| {
            let result = contextpatch_core::process::artifact_build::run_artifact_build_check(
                &worker_plan,
                Some(contextpatch_core::process::artifact_build::CONFIRMATION),
            )
            .map_err(|error| format!("artifact_build_check_run failed: {error}"))?;

            let timed_out = result.build.timed_out
                || result.smoke.as_ref().is_some_and(|smoke| smoke.timed_out);
            let terminal_status = if timed_out {
                "timed_out"
            } else if result.success() {
                "completed"
            } else {
                "failed"
            };
            let result_value = json!({
                "tool": crate::tools::artifact_build_check_run::NAME,
                "status": terminal_status,
                "dry_run": false,
                "dockerfile": worker_plan.dockerfile(),
                "tag": worker_plan.tag(),
                "build": artifact_command_value(&result.build),
                "smoke": result.smoke.as_ref().map(artifact_command_value),
                "image_cleanup": result.image_cleanup.as_ref().map(artifact_command_value),
                // A leaked image is reported rather than hidden behind a passing gate.
                "cleanup_clean": result.cleanup_clean()
            });
            let log = serde_json::to_string_pretty(&result_value)
                .map_err(|error| format!("artifact_build_check_run failed: {error}"))?;
            Ok(BackgroundJobOutcome {
                log,
                status: terminal_status,
                exit_code: result.build.exit_code,
                timed_out,
            })
        },
    )?;

    Ok(format!(
        "log_id: {log_id}\n{}",
        serde_json::to_string_pretty(&json!({
            "tool": crate::tools::artifact_build_check_run::NAME,
            "status": "running",
            "dockerfile": plan.dockerfile(),
            "tag": plan.tag(),
            "poll_with": crate::tools::read_command_log::NAME
        }))
        .map_err(|error| format!("artifact_build_check_run refused: {error}"))?
    ))
}

pub(super) fn artifact_command_value(
    result: &contextpatch_core::process::artifact_build::ArtifactBuildCommandResult,
) -> Value {
    json!({
        "exit_code": result.exit_code,
        "timed_out": result.timed_out,
        "duration_ms": result.duration_ms,
        "stdout": result.stdout,
        "stdout_truncated": result.stdout_truncated,
        "stderr": result.stderr,
        "stderr_truncated": result.stderr_truncated
    })
}

pub(super) fn compose_command_value(
    result: &contextpatch_core::process::compose_stack::ComposeStackCommandResult,
) -> Value {
    json!({
        "exit_code": result.exit_code,
        "timed_out": result.timed_out,
        "duration_ms": result.duration_ms,
        "stdout": result.stdout,
        "stdout_truncated": result.stdout_truncated,
        "stderr": result.stderr,
        "stderr_truncated": result.stderr_truncated
    })
}

pub(crate) fn call_task_image_python_run<'a>(
    repository_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let repository_root = repository_root.into();
    // Checked before any argument is read, so a selected repository receives the same refusal whether it
    // asked to plan or to execute. Core refuses again during planning; two independent gates because a
    // caller must not be able to plan against a selection and then execute that plan.
    contextpatch_core::process::task_image::ensure_task_image_root_is_addressable(repository_root)
        .map_err(|error| format!("task_image_python_run refused: {error}"))?;

    let script = required_string(arguments, "script")?;
    let program = optional_string(arguments, "program")?.unwrap_or("python3");
    let args = optional_string_array(arguments, "args")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;
    let build_timeout_secs = optional_u64(arguments, "build_timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;
    let plan = contextpatch_core::process::task_image::plan_task_image_python_run(
        repository_root,
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

pub(super) fn task_image_command_value(
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

pub(super) fn validate_docker_image_ref(image: &str, tool_name: &str) -> Result<(), String> {
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

pub(super) fn validate_find_filename(filename: &str, tool_name: &str) -> Result<(), String> {
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

pub(super) fn run_bounded_docker(
    tool_name: &str,
    args: &[String],
    timeout_secs: u64,
) -> Result<BoundedProcessOutput, String> {
    run_bounded_command(tool_name, "docker", args, Path::new("/"), timeout_secs)
}
