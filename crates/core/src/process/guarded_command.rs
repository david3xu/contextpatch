use std::path::Path;

use crate::error::ContextPatchError;
use crate::process::runner::{
    checked_timeout_with_max, display_command, resolve_child_cwd, resolve_program,
    run_bounded_command, validate_common_command_shape,
};

const DEFAULT_MAX_TIMEOUT_SECS: u64 = 600;
const HARBOR_RUN_MAX_TIMEOUT_SECS: u64 = 3600;

/// Run one allowlisted command inside the repository, through the repository's own authority.
///
/// The working directory is opened relative to the root descriptor and held open until the child has been
/// spawned, so the directory the child changes into is the directory that was checked. Scratch token
/// expansion still resolves to a path, because a child process receives argv strings and there is no
/// descriptor to hand it; that path is derived from repository identity rather than from the root's name.
pub fn run_guarded_command<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    cwd: Option<&Path>,
    program: &str,
    args: &[String],
    timeout_secs: Option<u64>,
) -> Result<String, ContextPatchError> {
    let root = repo_root.into();
    let cwd = resolve_child_cwd(root, cwd)?;

    validate_command(program, args)?;
    let timeout = checked_command_timeout(program, args, timeout_secs)?;
    let expanded_args = crate::fs::scratch::expand_scratch_tokens(root, args)?;
    let output = run_bounded_command(
        cwd.command_cwd(),
        program,
        &expanded_args,
        timeout,
        "guarded command",
    )?;

    Ok(format!(
        "command: {}\ncwd: {}\nallowlist: {}\nexit_code: {}\ntimed_out: {}\nduration_ms: {}\nstdout:\n{}\nstderr:\n{}",
        display_command(program, &expanded_args),
        cwd.logical_path().display(),
        allowlist_label(program, args),
        output.exit_code,
        output.timed_out,
        output.duration_ms,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn checked_command_timeout(
    program: &str,
    args: &[String],
    timeout_secs: Option<u64>,
) -> Result<std::time::Duration, ContextPatchError> {
    let max_timeout_secs = if program == "harbor" && args.first().is_some_and(|arg| arg == "run") {
        HARBOR_RUN_MAX_TIMEOUT_SECS
    } else {
        DEFAULT_MAX_TIMEOUT_SECS
    };
    checked_timeout_with_max(timeout_secs, max_timeout_secs)
}

pub fn resolve_guarded_program(program: &str) -> Option<std::path::PathBuf> {
    if !is_allowlisted_program(program) {
        return None;
    }
    resolve_program(program)
}

pub fn redact_and_truncate_output(text: &str, max_chars: usize) -> (String, bool) {
    let redacted = redact_output(text);
    let mut chars = redacted.chars();
    let output: String = chars.by_ref().take(max_chars).collect();
    let truncated = chars.next().is_some();
    if truncated {
        (format!("{output}\n[truncated]"), true)
    } else {
        (output, false)
    }
}

pub fn redact_and_truncate_output_tail(text: &str, max_chars: usize) -> (String, bool) {
    let redacted = redact_output(text);
    let total_chars = redacted.chars().count();
    if total_chars <= max_chars {
        return (redacted, false);
    }

    let output = redacted
        .chars()
        .skip(total_chars - max_chars)
        .collect::<String>();
    (format!("[truncated leading output]\n{output}"), true)
}

fn redact_output(text: &str) -> String {
    text.lines()
        .map(crate::process::runner::redact_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn validate_command(program: &str, args: &[String]) -> Result<(), ContextPatchError> {
    validate_common_command_shape(program, args)?;
    if matches!(program, "python" | "python3")
        && args
            .first()
            .is_some_and(|script| script.contains(crate::fs::scratch::SCRATCH_TOKEN))
    {
        return Err(ContextPatchError::new(
            "guarded command refused: Python code must be repository-relative; use \
             artifact_python_run for a script stored outside the repository",
        ));
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
        "pnpm" => matches!(subcommand, Some("run" | "test")),
        "python" | "python3" => {
            subcommand.is_some_and(|script| script.ends_with(".py") && !script.starts_with('-'))
        }
        "pytest" => true,
        "harbor" => matches!(subcommand, Some("run")),
        "bash" => {
            args == ["references/check-base-image.sh"]
                || args == ["references/check-base-image.sh", "task"]
        }
        "rg" => subcommand.is_some(),
        _ => false,
    };

    if !allowed {
        return Err(ContextPatchError::new(format!(
            "guarded command refused: `{}` is not allowlisted ({})",
            display_command(program, args),
            crate::process::guidance::refusal_suffix(program, args)
        )));
    }

    Ok(())
}

fn is_allowlisted_program(program: &str) -> bool {
    matches!(
        program,
        "git"
            | "cargo"
            | "bun"
            | "npm"
            | "pnpm"
            | "python"
            | "python3"
            | "pytest"
            | "harbor"
            | "bash"
            | "rg"
    )
}

fn allowlist_label(program: &str, args: &[String]) -> String {
    match args.first() {
        Some(subcommand) => format!("{program}/{subcommand}"),
        None => program.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        checked_command_timeout, redact_and_truncate_output, redact_and_truncate_output_tail,
        run_guarded_command, validate_command,
    };
    use crate::process::runner::redact_line;

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
    fn redacts_and_bounds_external_workflow_output() {
        let (output, truncated) =
            redact_and_truncate_output("safe\nDATACORE_TOKEN=super-secret-value\ntail", 60);
        assert!(output.contains("safe"));
        assert!(output.contains("[redacted potential secret line]"));
        assert!(!output.contains("super-secret-value"));
        assert!(!truncated);

        let (output, truncated) = redact_and_truncate_output("éééé", 3);
        assert_eq!(output, "ééé\n[truncated]");
        assert!(truncated);

        let long_log = format!(
            "{}\nDATACORE_TOKEN=super-secret-value\nterminal failure",
            "prefix".repeat(20)
        );
        let (output, truncated) = redact_and_truncate_output_tail(&long_log, 60);
        assert!(output.starts_with("[truncated leading output]\n"));
        assert!(output.contains("[redacted potential secret line]"));
        assert!(output.ends_with("terminal failure"));
        assert!(!output.contains("super-secret-value"));
        assert!(truncated);
    }

    #[test]
    fn refuses_disallowed_git_mutation() {
        let root = git_root("refuses_disallowed_git_mutation");

        let error =
            run_guarded_command(&root, None, "git", &["reset".to_string()], Some(30)).unwrap_err();

        assert!(error.to_string().contains("not allowlisted"));
    }

    #[test]
    fn extends_timeout_only_for_harbor_run() {
        let harbor_args = ["run".to_string()];
        assert!(checked_command_timeout("harbor", &harbor_args, Some(3600)).is_ok());
        assert!(checked_command_timeout("harbor", &harbor_args, Some(3601))
            .unwrap_err()
            .to_string()
            .contains("between 1 and 3600"));

        let git_args = ["status".to_string()];
        assert!(checked_command_timeout("git", &git_args, Some(601))
            .unwrap_err()
            .to_string()
            .contains("between 1 and 600"));
    }

    #[test]
    fn expands_scratch_only_after_the_original_command_is_allowlisted() {
        let root = git_root("expands_scratch_only_after_the_original_command_is_allowlisted");
        fs::write(
            root.join("write_scratch.py"),
            "import pathlib, sys\npathlib.Path(sys.argv[1]).write_text('outside repo\\n')\n",
        )
        .unwrap();

        let output = run_guarded_command(
            &root,
            None,
            "python3",
            &[
                "write_scratch.py".to_string(),
                "{scratch}/result.txt".to_string(),
            ],
            Some(30),
        )
        .unwrap();

        let scratch = crate::fs::scratch::scratch_root(&root);
        assert_eq!(
            fs::read_to_string(scratch.as_ref().unwrap().join("result.txt")).unwrap(),
            "outside repo\n"
        );
        assert!(output.contains(&scratch.as_ref().unwrap().display().to_string()));
        assert!(!root.join("result.txt").exists());
    }

    #[test]
    fn scratch_may_be_an_output_but_not_the_python_script() {
        validate_command(
            "python3",
            &[
                "write_scratch.py".to_string(),
                "{scratch}/result.txt".to_string(),
            ],
        )
        .unwrap();

        let error =
            validate_command("python3", &["{scratch}/outside-script.py".to_string()]).unwrap_err();
        assert!(error.to_string().contains("artifact_python_run"));
    }

    #[test]
    fn allows_pnpm_scripts_but_not_package_install() {
        validate_command(
            "pnpm",
            &[
                "run".to_string(),
                "cap:sync:ios:prod".to_string(),
                "--reporter=silent".to_string(),
            ],
        )
        .unwrap();

        let error = validate_command("pnpm", &["add".to_string(), "@capacitor/core".to_string()])
            .unwrap_err();

        assert!(error.to_string().contains("not allowlisted"));
    }

    #[test]
    fn allows_project_python_pytest_and_harbor_but_not_pip_or_docker() {
        validate_command(
            "python3",
            &[
                "scripts/generate_fixtures.py".to_string(),
                "--verify".to_string(),
            ],
        )
        .unwrap();
        validate_command("pytest", &["tests".to_string(), "-q".to_string()]).unwrap();
        validate_command(
            "harbor",
            &[
                "run".to_string(),
                "-p".to_string(),
                "task".to_string(),
                "--agent".to_string(),
                "oracle".to_string(),
            ],
        )
        .unwrap();
        validate_command("bash", &["references/check-base-image.sh".to_string()]).unwrap();
        validate_command(
            "bash",
            &[
                "references/check-base-image.sh".to_string(),
                "task".to_string(),
            ],
        )
        .unwrap();

        assert!(
            validate_command("pip", &["install".to_string(), "pytest".to_string()])
                .unwrap_err()
                .to_string()
                .contains("not allowlisted")
        );
        assert!(
            validate_command("docker", &["run".to_string(), "image".to_string()])
                .unwrap_err()
                .to_string()
                .contains("not allowlisted")
        );
        assert!(
            validate_command("python3", &["-m".to_string(), "pip".to_string()])
                .unwrap_err()
                .to_string()
                .contains("not allowlisted")
        );
        assert!(validate_command("bash", &["scripts/other.sh".to_string()])
            .unwrap_err()
            .to_string()
            .contains("not allowlisted"));
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
