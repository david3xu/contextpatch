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

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let started = std::time::Instant::now();
    let mut child = command.spawn().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to run guarded command `{}` in {}: {error}",
            display_command(program, args),
            cwd.display()
        ))
    })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ContextPatchError::new("failed to capture guarded command stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ContextPatchError::new("failed to capture guarded command stderr pipe"))?;
    let stdout_reader = spawn_stream_reader("stdout", stdout);
    let stderr_reader = spawn_stream_reader("stderr", stderr);

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
                let stdout = join_stream_reader(stdout_reader)?;
                let stderr = join_stream_reader(stderr_reader)?;
                return Ok(format!(
                    "command: {}\ncwd: {}\nallowlist: {}\nexit_code: -1\ntimed_out: true\nduration_ms: {}\nstdout:\n{}\nstderr:\n{}",
                    display_command(program, args),
                    cwd.display(),
                    allowlist_label(program, args),
                    started.elapsed().as_millis(),
                    redact_and_truncate(&stdout),
                    redact_and_truncate(&stderr)
                ));
            }
            None => thread::sleep(Duration::from_millis(25)),
        }
    };

    let stdout = join_stream_reader(stdout_reader)?;
    let stderr = join_stream_reader(stderr_reader)?;

    Ok(format!(
        "command: {}\ncwd: {}\nallowlist: {}\nexit_code: {}\ntimed_out: false\nduration_ms: {}\nstdout:\n{}\nstderr:\n{}",
        display_command(program, args),
        cwd.display(),
        allowlist_label(program, args),
        status.code().unwrap_or(-1),
        started.elapsed().as_millis(),
        redact_and_truncate(&stdout),
        redact_and_truncate(&stderr)
    ))
}

fn spawn_stream_reader(
    label: &'static str,
    mut stream: impl Read + Send + 'static,
) -> thread::JoinHandle<Result<String, ContextPatchError>> {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        stream.read_to_end(&mut buffer).map_err(|error| {
            ContextPatchError::new(format!("failed to read guarded command {label}: {error}"))
        })?;
        Ok(String::from_utf8_lossy(&buffer).into_owned())
    })
}

fn join_stream_reader(
    reader: thread::JoinHandle<Result<String, ContextPatchError>>,
) -> Result<String, ContextPatchError> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = reader
            .join()
            .unwrap_or_else(|_| Err(ContextPatchError::new("guarded command reader panicked")));
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| ContextPatchError::new("timed out reading guarded command output"))?
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
            Some("status" | "diff" | "log" | "show" | "rev-parse" | "ls-tree")
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
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{redact_line, run_guarded_command};

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

    #[test]
    fn drains_stdout_and_stderr_without_hanging() {
        let root = git_root("drains_stdout_and_stderr_without_hanging");
        fs::write(root.join("tracked.txt"), "one\ntwo\n").unwrap();
        run_git(&root, &["add", "tracked.txt"]);
        run_git(&root, &["commit", "--quiet", "-m", "initial"]);

        let stdout = run_guarded_command(
            &root,
            None,
            "git",
            &["ls-tree".to_string(), "-r".to_string(), "HEAD".to_string()],
            Some(30),
        )
        .unwrap();
        assert!(stdout.contains("allowlist: git/ls-tree"));
        assert!(stdout.contains("timed_out: false"));
        assert!(stdout.contains("tracked.txt"));

        let stderr = run_guarded_command(
            &root,
            None,
            "git",
            &[
                "status".to_string(),
                "--definitely-not-a-real-option".to_string(),
            ],
            Some(30),
        )
        .unwrap();
        assert!(stderr.contains("timed_out: false"));
        assert!(stderr.contains("stderr:"));
        assert!(stderr.contains("definitely-not-a-real-option"));
    }

    #[test]
    fn redaction_keeps_secret_adjacent_paths_and_docs_readable() {
        assert_eq!(
            redact_line("clients/vscode/src/commands/ask-datacore.ts"),
            "clients/vscode/src/commands/ask-datacore.ts"
        );
        assert_eq!(
            redact_line("docs mention token discovery without showing a value"),
            "docs mention token discovery without showing a value"
        );
        assert_eq!(
            redact_line("docs/migration/roadmaps/product-readiness-task-list.md"),
            "docs/migration/roadmaps/product-readiness-task-list.md"
        );
        assert_eq!(
            redact_line("clients/vscode/src/chat/linked-task-store.ts"),
            "clients/vscode/src/chat/linked-task-store.ts"
        );
        assert_eq!(
            redact_line("DATACORE_GATEWAY_HTTP_API_KEY=REPLACE_ME"),
            "DATACORE_GATEWAY_HTTP_API_KEY=REPLACE_ME"
        );
        assert_eq!(
            redact_line("| API key | Use DATACORE_GATEWAY_HTTP_API_KEY in the runtime env |"),
            "| API key | Use DATACORE_GATEWAY_HTTP_API_KEY in the runtime env |"
        );
        assert_eq!(
            redact_line("DATACORE_TOKEN=super-secret-value"),
            "[redacted potential secret line]"
        );
        assert_eq!(
            redact_line("Authorization: Bearer abc123"),
            "[redacted potential secret line]"
        );
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
