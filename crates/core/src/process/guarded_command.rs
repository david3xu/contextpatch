use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::ContextPatchError;

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;
const MAX_ARGS: usize = 64;
const MAX_ARG_LEN: usize = 4096;
const MAX_OUTPUT_CHARS: usize = 12_000;

pub fn run_guarded_command(
    repo_root: &Path,
    cwd: Option<&Path>,
    program: &str,
    args: &[String],
    timeout_secs: Option<u64>,
) -> Result<String, ContextPatchError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    let cwd = resolve_cwd(&root, cwd)?;
    let timeout = checked_timeout(timeout_secs)?;

    validate_command(program, args)?;

    let mut command = Command::new(program);
    command.current_dir(&cwd);
    if program == "git" {
        command.arg("--no-pager");
    }
    command.args(args);
    command.env("GIT_PAGER", "cat");
    command.env("NO_COLOR", "1");
    command.stdin(Stdio::null());

    let output_paths = OutputPaths::new()?;
    let stdout = fs::File::create(&output_paths.stdout).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to create stdout log {}: {error}",
            output_paths.stdout.display()
        ))
    })?;
    let stderr = fs::File::create(&output_paths.stderr).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to create stderr log {}: {error}",
            output_paths.stderr.display()
        ))
    })?;
    command.stdout(Stdio::from(stdout));
    command.stderr(Stdio::from(stderr));

    let started = std::time::Instant::now();
    let mut child = command.spawn().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to run guarded command `{}` in {}: {error}",
            display_command(program, args),
            cwd.display()
        ))
    })?;

    let status = loop {
        match child.try_wait().map_err(|error| {
            ContextPatchError::new(format!(
                "failed while waiting for guarded command `{}`: {error}",
                display_command(program, args)
            ))
        })? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ContextPatchError::new(format!(
                    "guarded command timed out after {}s: {}",
                    timeout.as_secs(),
                    display_command(program, args)
                )));
            }
            None => thread::sleep(Duration::from_millis(100)),
        }
    };

    let stdout = read_log(&output_paths.stdout)?;
    let stderr = read_log(&output_paths.stderr)?;
    let _ = fs::remove_file(&output_paths.stdout);
    let _ = fs::remove_file(&output_paths.stderr);

    Ok(format!(
        "command: {}\ncwd: {}\nallowlist: {}\nexit_code: {}\nduration_ms: {}\nstdout:\n{}\nstderr:\n{}",
        display_command(program, args),
        cwd.display(),
        allowlist_label(program, args),
        status.code().unwrap_or(-1),
        started.elapsed().as_millis(),
        redact_and_truncate(&stdout),
        redact_and_truncate(&stderr)
    ))
}

fn resolve_cwd(root: &Path, cwd: Option<&Path>) -> Result<PathBuf, ContextPatchError> {
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

fn checked_timeout(timeout_secs: Option<u64>) -> Result<Duration, ContextPatchError> {
    let timeout_secs = timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
    if timeout_secs == 0 || timeout_secs > MAX_TIMEOUT_SECS {
        return Err(ContextPatchError::new(format!(
            "timeout_secs must be between 1 and {MAX_TIMEOUT_SECS}"
        )));
    }
    Ok(Duration::from_secs(timeout_secs))
}

fn validate_command(program: &str, args: &[String]) -> Result<(), ContextPatchError> {
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

    let subcommand = args.first().map(String::as_str);
    let allowed = match program {
        "git" => matches!(
            subcommand,
            Some("status" | "diff" | "log" | "show" | "rev-parse")
        ),
        "cargo" => matches!(subcommand, Some("check" | "test" | "build" | "clippy")),
        "bun" => matches!(subcommand, Some("run" | "test")),
        "npm" => matches!(subcommand, Some("run" | "test")),
        "rg" => subcommand.is_some(),
        _ => false,
    };

    if !allowed {
        return Err(ContextPatchError::new(format!(
            "guarded command refused: `{}` is not allowlisted",
            display_command(program, args)
        )));
    }

    Ok(())
}

fn allowlist_label(program: &str, args: &[String]) -> String {
    match args.first() {
        Some(subcommand) => format!("{program}/{subcommand}"),
        None => program.to_string(),
    }
}

fn display_command(program: &str, args: &[String]) -> String {
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

fn read_log(path: &Path) -> Result<String, ContextPatchError> {
    fs::read_to_string(path).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to read command log {}: {error}",
            path.display()
        ))
    })
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

fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    if lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("token")
        || lower.contains("secret")
        || lower.contains("password")
        || line.contains("sk-")
    {
        "[redacted potential secret line]".to_string()
    } else {
        line.to_string()
    }
}

struct OutputPaths {
    stdout: PathBuf,
    stderr: PathBuf,
}

impl OutputPaths {
    fn new() -> Result<Self, ContextPatchError> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| ContextPatchError::new(format!("system clock error: {error}")))?
            .as_nanos();
        let pid = std::process::id();
        let base = std::env::temp_dir().join(format!("contextpatch-command-{pid}-{unique}"));
        Ok(Self {
            stdout: base.with_extension("stdout.log"),
            stderr: base.with_extension("stderr.log"),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::run_guarded_command;

    #[test]
    fn runs_allowlisted_git_status() {
        let root = git_root("runs_allowlisted_git_status");

        let output = run_guarded_command(
            &root,
            None,
            "git",
            &["status".to_string(), "--porcelain=v1".to_string()],
            Some(30),
        )
        .unwrap();

        assert!(output.contains("allowlist: git/status"));
        assert!(output.contains("exit_code: 0"));
    }

    #[test]
    fn refuses_disallowed_git_mutation() {
        let root = git_root("refuses_disallowed_git_mutation");

        let error =
            run_guarded_command(&root, None, "git", &["reset".to_string()], Some(30)).unwrap_err();

        assert!(error.to_string().contains("not allowlisted"));
    }

    #[test]
    fn refuses_cwd_outside_root() {
        let root = git_root("refuses_cwd_outside_root");
        let outside = temp_root("outside-cwd");

        let error = run_guarded_command(
            &root,
            Some(&outside),
            "git",
            &["status".to_string()],
            Some(30),
        )
        .unwrap_err();

        assert!(error.to_string().contains("outside repository root"));
    }

    fn git_root(name: &str) -> PathBuf {
        let root = temp_root(name);
        run_git(&root, &["init", "--quiet"]);
        root
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
}
