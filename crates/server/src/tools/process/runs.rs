use std::path::Path;

use serde_json::{json, Value};

use super::*;
use crate::tools::common::{
    optional_bool, optional_string, optional_string_array, optional_u64, required_string,
};

/// The longest Harbor agent identifier accepted, advertised and enforced from here.
///
/// Server-side because nothing in `core` bounds an agent name. The run timeout is the opposite case:
/// `core` already names it as `HARBOR_RUN_MAX_TIMEOUT_SECS`, so this module reads that rather than
/// declaring a second name for one bound.
pub(crate) const MAX_HARBOR_AGENT_LEN: usize = 128;

pub(crate) fn call_harbor_run_start<'a>(
    repository_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let project = optional_string(arguments, "project")?.unwrap_or("task");
    let agent = required_string(arguments, "agent")?;
    let max_timeout_secs = contextpatch_core::process::guarded_command::HARBOR_RUN_MAX_TIMEOUT_SECS;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(max_timeout_secs);
    if timeout_secs == 0 || timeout_secs > max_timeout_secs {
        return Err(format!(
            "harbor_run_start refused: timeout_secs must be between 1 and {max_timeout_secs}"
        ));
    }
    if agent.is_empty()
        || agent.len() > MAX_HARBOR_AGENT_LEN
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
    let root = repository_root.into();
    validate_harbor_project_directory(root, Path::new(&project))?;
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
    // Retained before scheduling and carried for the job's lifetime, so the directory Harbor runs in and the
    // directory its evidence is read from are both the one that was validated here, however long the run
    // takes and whatever happens to the name meanwhile.
    let worker_authority = contextpatch_core::git::OwnedRepositoryRoot::retain(root)
        .map_err(|error| format!("harbor_run_start refused: {error}"))?;
    let log_id = start_background_job(
        crate::tools::harbor_run_start::NAME,
        "harbor",
        &initial_log,
        move |worker_log_id| {
            let output = run_guarded_command(
                worker_authority.borrow(),
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
                "harbor": crate::tools::harbor::structured_evidence(worker_authority.borrow(), &output)
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

/// Confirm the Harbor project directory through the repository's own authority.
///
/// The component walk survives because it enforces something the rooted primitives do not: a component may
/// not begin with `-`, since Harbor receives the project as an argv string and a leading dash would be read
/// as an option. What no longer happens is joining each component onto a pathname and inspecting it, then
/// canonicalizing the leaf to test containment: the rooted lookup refuses a symlink at any component and
/// cannot leave the repository, so containment is structural.
pub(super) fn validate_harbor_project_directory(
    root: contextpatch_core::git::RepositoryRoot<'_>,
    project: &Path,
) -> Result<(), String> {
    let mut parts = Vec::new();
    for component in project.components() {
        let Component::Normal(component) = component else {
            return Err(
                "harbor_run_start refused: project must be a normalized repository-relative path"
                    .to_string(),
            );
        };
        let component = component.to_string_lossy();
        if component.starts_with('-') {
            return Err(
                "harbor_run_start refused: project path components must not start with `-`"
                    .to_string(),
            );
        }
        parts.push(component.into_owned());
    }
    let relative = parts.join("/");
    match contextpatch_core::fs::rooted::entry_kind(root, &relative) {
        Ok(Some(contextpatch_core::fs::rooted::RootedEntryKind::Directory)) => Ok(()),
        Ok(Some(contextpatch_core::fs::rooted::RootedEntryKind::Symlink)) => Err(format!(
            "harbor_run_start refused: project `{}` must not contain symlink components",
            project.display()
        )),
        Ok(_) => Err(format!(
            "harbor_run_start refused: project `{}` is not an existing repository directory",
            project.display()
        )),
        Err(error) => Err(format!(
            "harbor_run_start refused: failed to inspect project `{}`: {error}",
            project.display()
        )),
    }
}

pub(crate) fn call_artifact_python_run<'a>(
    repository_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
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
    let artifact_root = crate::tools::files::artifact_root(
        repository_root,
        crate::tools::artifact_python_run::NAME,
    )?;
    let relative = crate::tools::common::normalize_repo_relative_path(
        crate::tools::artifact_python_run::NAME,
        script,
    )?;
    // The artifact directory is the authority for both the script check and the child's working directory.
    // The script is confirmed to be a regular file beneath that directory without following a symlink at any
    // component, which replaces canonicalizing the leaf and comparing prefixes afterwards.
    let authority = contextpatch_core::git::RepositoryRoot::from_path(&artifact_root);
    if !contextpatch_core::fs::rooted::is_regular_file(authority, &relative)
        .map_err(|error| format!("artifact_python_run refused: {error}"))?
    {
        return Err(format!(
            "artifact_python_run refused: `{script}` is not an existing artifact file"
        ));
    }
    // Opened and held until the child has been spawned, so the directory the child changes into is the
    // directory that was checked rather than a name resolved again at spawn time.
    let working_directory = contextpatch_core::fs::rooted::open_directory(authority, "")
        .map_err(|error| format!("artifact_python_run refused: {error}"))?;
    // The interpreter receives argv strings and there is no descriptor form of a script argument, so the
    // script is named relative to the working directory it is about to run in.
    let mut command_args = vec![relative.clone()];
    command_args.extend(args);
    let output = run_bounded_command(
        crate::tools::artifact_python_run::NAME,
        program,
        &command_args,
        contextpatch_core::process::runner::CommandCwd::Anchored {
            directory: &working_directory,
            logical_path: &artifact_root,
        },
        timeout_secs,
    )?;
    let text = format_command_output(program, &command_args, &artifact_root, &output);
    let log_id = write_command_log(&text)
        .map_err(|error| format!("artifact_python_run log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{text}"))
}

pub(crate) fn call_validation_profile_run<'a>(
    repository_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
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

    // Retained before the job is scheduled, so the worker carries the repository's authority rather than its
    // name. Capturing a path here would mean the directory each command runs in is resolved after this call
    // has already returned, which is the longest possible gap between validating a repository and acting on
    // it.
    let worker_authority =
        contextpatch_core::git::OwnedRepositoryRoot::retain(repository_root.into())
            .map_err(|error| format!("validation_profile_run refused: {error}"))?;
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
            let log = run_validation_profile_sync(worker_authority.borrow(), &worker_arguments)?;
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

pub(super) fn run_validation_profile_sync(
    root: contextpatch_core::git::RepositoryRoot<'_>,
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
            root,
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
            let rewards = crate::tools::harbor::rewards_for_run(root, &output)
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

pub(super) struct ProfileCommand {
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

/// The validation profiles this server knows, as one declared list.
///
/// The five names were written out in five places: these arms, the refusal below, the capability
/// manifest's list, the preflight document's keys, and the advertised description of the `profile`
/// argument. Three of those now derive from here.
///
/// Two do not, for different reasons. The arms cannot, because each carries its own commands, so the
/// tie between this list and them is asserted by test instead: every name here must resolve, and a
/// name absent from here must not. The preflight document's keys stay hand-written because each entry
/// also carries that profile's availability and required tools, which is a different fact from the
/// name and wants its own change rather than being folded into this one.
///
/// Without the assertion this const would be the sixth copy rather than the single source.
pub(crate) const VALIDATION_PROFILE_NAMES: &[&str] = &[
    "repo-basic",
    "rust-workspace",
    "datacore-vscode",
    "datacore-m6-vscode",
    "dynamo-harbor-task",
];

pub(super) fn validation_profile(profile: &str) -> Result<Vec<ProfileCommand>, String> {
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
            "validation_profile_run refused: unknown profile `{profile}`; expected one of: {}",
            VALIDATION_PROFILE_NAMES.join(", ")
        )),
    }
}

pub(super) fn harbor_agent<'a>(args: &'a [&'static str]) -> Option<&'a str> {
    args.windows(2)
        .find_map(|window| (window[0] == "--agent").then_some(window[1]))
}

pub(super) fn rewards_deterministic(rewards: &[f64]) -> bool {
    match rewards.split_first() {
        Some((first, rest)) if !rest.is_empty() => rest
            .iter()
            .all(|reward| (*reward - *first).abs() <= f64::EPSILON),
        _ => false,
    }
}
