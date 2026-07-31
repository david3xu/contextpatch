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

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use contextpatch_core::process::guarded_command::run_guarded_command;
use serde_json::Value;

use crate::tools::common::{
    optional_bool, optional_string, optional_string_array, optional_u64, required_string,
    required_string_array,
};

pub(crate) fn call_run_guarded_command(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let program = required_string(arguments, "program")?;
    let args = required_string_array(arguments, "args")?;
    let cwd = optional_string(arguments, "cwd")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;

    let output = run_guarded_command(repo_root, cwd.map(Path::new), program, &args, timeout_secs)
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
        "log_id: {log_id}\noffset: {offset}\nchars_returned: {}\ntotal_chars: {}\n{slice}",
        end - start,
        chars.len()
    ))
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
    let clean = output.status.success() && matches.is_empty();
    serde_json::to_string_pretty(&serde_json::json!({
        "tool": crate::tools::image_cleanliness_check_run::NAME,
        "dry_run": false,
        "ran": true,
        "image": image,
        "filename": filename,
        "exit_code": output.status.code().unwrap_or(-1),
        "clean": clean,
        "matches": matches,
        "stdout": stdout,
        "stderr": stderr
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
    let stdout = truncate_string(String::from_utf8_lossy(&output.stdout).to_string(), 120_000);
    let stderr = truncate_string(String::from_utf8_lossy(&output.stderr).to_string(), 20_000);
    serde_json::to_string_pretty(&serde_json::json!({
        "tool": crate::tools::docker_image_inspect::NAME,
        "dry_run": false,
        "ran": true,
        "image": image,
        "exit_code": output.status.code().unwrap_or(-1),
        "success": output.status.success(),
        "stdout": stdout,
        "stderr": stderr
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
            match extract_harbor_reward(&output) {
                Some(reward) if agent == "oracle" => harbor_oracle_rewards.push(reward),
                Some(reward) if agent == "nop" => harbor_nop_rewards.push(reward),
                Some(_) => harbor_missing_rewards.push(format!(
                    "{}. {} | unrecognized_agent: {agent}",
                    index + 1,
                    command.display()
                )),
                None => harbor_missing_rewards.push(format!(
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

    lines.insert(3, format!("commands_run: {ran}"));
    lines.insert(4, format!("failed: {failed}"));
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
) -> Result<std::process::Output, String> {
    run_bounded_command(tool_name, "docker", args, Path::new("/"), timeout_secs)
}

fn run_bounded_command(
    tool_name: &str,
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout_secs: u64,
) -> Result<std::process::Output, String> {
    let started = std::time::Instant::now();
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("{tool_name} refused: failed to run {program}: {error}"))?;
    let timeout = Duration::from_secs(timeout_secs);
    loop {
        match child.try_wait().map_err(|error| {
            format!("{tool_name} refused: failed while waiting for {program}: {error}")
        })? {
            Some(_) => {
                return child.wait_with_output().map_err(|error| {
                    format!("{tool_name} refused: failed to collect {program} output: {error}")
                });
            }
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let output = child.wait_with_output().map_err(|error| {
                    format!("{tool_name} refused: failed to collect timed-out {program} output: {error}")
                })?;
                return Err(format!(
                    "{tool_name} refused: {program} timed out after {timeout_secs}s\nstdout:\n{}\nstderr:\n{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    }
}

fn format_command_output(
    program: &str,
    args: &[String],
    cwd: &Path,
    output: &std::process::Output,
) -> String {
    format!(
        "command: {}\ncwd: {}\nexit_code: {}\nsuccess: {}\nstdout:\n{}\nstderr:\n{}",
        std::iter::once(program.to_string())
            .chain(args.iter().map(|arg| shell_display_arg(arg)))
            .collect::<Vec<_>>()
            .join(" "),
        cwd.display(),
        output.status.code().unwrap_or(-1),
        output.status.success(),
        truncate_string(String::from_utf8_lossy(&output.stdout).to_string(), 120_000),
        truncate_string(String::from_utf8_lossy(&output.stderr).to_string(), 20_000)
    )
}

fn truncate_string(mut text: String, max_chars: usize) -> String {
    if text.len() > max_chars {
        text.truncate(max_chars);
        text.push_str("\n[truncated]");
    }
    text
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

fn extract_harbor_reward(text: &str) -> Option<f64> {
    text.lines().filter_map(parse_reward_line).next_back()
}

fn parse_reward_line(line: &str) -> Option<f64> {
    let lower = line.to_ascii_lowercase();
    let reward_index = lower.find("reward");
    let score_index = lower.find("score");
    let index = match (reward_index, score_index) {
        (Some(reward), Some(score)) => reward.min(score),
        (Some(reward), None) => reward,
        (None, Some(score)) => score,
        (None, None) => return None,
    };
    line[index..]
        .split(|ch: char| !(ch.is_ascii_digit() || matches!(ch, '.' | '-' | '+' | 'e' | 'E')))
        .filter(|token| token.chars().any(|ch| ch.is_ascii_digit()))
        .filter_map(|token| token.parse::<f64>().ok())
        .next()
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
    let log_id = format!("cmd-{}-{unique}", std::process::id());
    fs::write(dir.join(format!("{log_id}.log")), text)
        .map_err(|error| format!("failed to write command log {log_id}: {error}"))?;
    Ok(log_id)
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

fn command_log_dir() -> PathBuf {
    std::env::temp_dir().join("contextpatch-command-logs")
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
