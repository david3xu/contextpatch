use std::path::Path;

use crate::error::ContextPatchError;
use crate::process::runner::{
    checked_timeout_with_max, display_command, resolve_child_cwd, resolve_program,
    run_bounded_command, validate_common_command_shape,
};

/// Cap for an allowlisted guarded command.
///
/// Equal to `runner::MAX_TIMEOUT_SECS` and `task_image::MAX_RUN_TIMEOUT_SECS` today, and separate
/// from both on purpose: the call sets are disjoint, so they bound different work and agree only by
/// coincidence.
const DEFAULT_MAX_TIMEOUT_SECS: u64 = 600;
/// The longest a Harbor run may be given, whether reached through a guarded `harbor run` command or
/// through `harbor_run_start`. Public because the advertised bound must read it rather than restate
/// it, and both paths bound the same operation.
pub const HARBOR_RUN_MAX_TIMEOUT_SECS: u64 = 3600;

/// pytest's plugin-loading option, in both its separated and combined short forms.
const PYTEST_PLUGIN_OPTION: &str = "-p";
const LONG_OPTION_PREFIX: &str = "--";

/// The base-image check, which is the one script carrying a documented optional argument.
const BASE_IMAGE_SCRIPT: &str = "references/check-base-image.sh";

/// Repository shell scripts the guarded runner may execute.
///
/// Safety-contract clause 19 permits a shell-script exception only when it is fixed to specific
/// repository scripts rather than granting arbitrary shell authority, so this is an exact
/// enumeration and deliberately not a `scripts/*.sh` pattern. A glob would execute any file later
/// dropped into that directory, which is precisely the authority the clause refuses; adding a gate
/// is instead a reviewed one-line change here.
///
/// This is an entry-point restriction, not isolation. Every entry is reviewed repository code that
/// runs with the server user's permissions and network access, and the same code is already
/// reachable through an allowlisted `bun`/`npm` test that spawns it. See
/// `docs/execution-threat-model.md`.
const ALLOWED_SHELL_SCRIPTS: &[&str] = &[
    BASE_IMAGE_SCRIPT,
    "scripts/check-doc-commands.sh",
    "scripts/check-docs.sh",
    "scripts/check-endpoint-literals.sh",
    "scripts/check-hosted-target-readiness.sh",
    "scripts/docs-audit.sh",
    // The six root proofs. Measured rather than assumed: these are shell scripts that bring the
    // system up themselves and contain no `docker compose` invocation, which is why the Compose
    // stack tool is not the instrument for them and its action list stays empty.
    "scripts/front-door-proof.sh",
    "scripts/full-platform-proof.sh",
    "scripts/prove-auto-workflow.sh",
    "scripts/prove-dispatch-preflight.sh",
    "scripts/prove-human-ai-team-flow.sh",
    "scripts/prove-local-edition-bundle.sh",
];

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

    if program == crate::process::runner::PROGRAM_PYTEST {
        if let Some(option) = args.iter().find(|arg| is_pytest_plugin_option(arg)) {
            return Err(ContextPatchError::new(format!(
                "guarded command refused: pytest option `{option}` loads a caller-named plugin; \
                 ambient plugin autoload is disabled and reviewed repository `conftest.py` is \
                 collected instead"
            )));
        }
    }

    let subcommand = args.first().map(String::as_str);
    let allowed = match program {
        "git" => matches!(
            subcommand,
            Some(
                "status"
                    | "diff"
                    | "log"
                    | "show"
                    | "rev-parse"
                    | "rev-list"
                    | "shortlog"
                    | "ls-tree"
            )
        ),
        "cargo" => match subcommand {
            Some("check" | "test" | "build" | "clippy") => true,
            Some("fmt") => cargo_fmt_arguments_are_allowed(args),
            _ => false,
        },
        "bun" => matches!(subcommand, Some("run" | "test")),
        "npm" => matches!(subcommand, Some("run" | "test")),
        "pnpm" => matches!(subcommand, Some("run" | "test")),
        "python" | "python3" => {
            subcommand.is_some_and(|script| script.ends_with(".py") && !script.starts_with('-'))
        }
        "pytest" => true,
        "harbor" => matches!(subcommand, Some("run")),
        "bash" => is_allowed_shell_script(args),
        "rg" => subcommand.is_some() && rg_arguments_are_allowed(args),
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

/// `cargo fmt` arguments, listed positively so a later rustfmt option cannot widen this surface.
///
/// `fmt` is admitted only as a check, never as a rewrite: reformatting the tree is a mutation and
/// this surface grants none. The reason to permit it at all is that a gate whose result cannot be
/// captured is not evidence, and `--check` reports a diff and an exit code without touching a file.
///
/// This began as a scan that refused `--emit`, which is a denylist, and C37 is the argument that a
/// denylist does not hold: the option worth refusing is whatever the next release adds. Measured
/// against the current rustfmt, neither `--config emit_mode=Files` nor `--config-path` wrote a file
/// under `--check`, so the reason to refuse them is not that they were shown to write. It is that
/// anything not named here is refused, which does not depend on that measurement surviving an
/// upgrade. Widening this is deliberately an edit to this list.
const CARGO_FMT_CHECK: &str = "--check";

/// The one option whose following argument is a value rather than an option.
const CARGO_FMT_PACKAGE: &str = "-p";

const CARGO_FMT_OPTIONS: &[&str] = &["--all", CARGO_FMT_CHECK, "--"];

fn cargo_fmt_arguments_are_allowed(args: &[String]) -> bool {
    let mut checked = false;
    let mut expecting_package_name = false;
    for argument in args.iter().skip(1).map(String::as_str) {
        if expecting_package_name {
            expecting_package_name = false;
            continue;
        }
        if argument == CARGO_FMT_PACKAGE {
            expecting_package_name = true;
            continue;
        }
        if !CARGO_FMT_OPTIONS.contains(&argument) {
            return false;
        }
        checked |= argument == CARGO_FMT_CHECK;
    }
    checked && !expecting_package_name
}

/// ripgrep options that only search, listed positively so a new release cannot widen this surface.
///
/// `rg` was previously admitted on `subcommand.is_some()`, which permitted every option it has. That
/// was arbitrary program execution rather than search: `rg --pre sh --pre-glob '*'` runs a shell over
/// repository files, which was demonstrated writing outside the repository root as the server user.
/// It walked around the fixed shell-script list, the repository-relative Python rule, and the pytest
/// plugin hardening in a single argument.
///
/// A denylist cannot hold this shut, because the dangerous options are whatever the next ripgrep
/// version adds. Anything not named here is refused, so `--pre`, `--pre-glob`, `--hostname-bin`,
/// `-z`/`--search-zip`, and `--follow` are excluded by construction rather than by enumeration: the
/// first three start programs, the fourth shells out to decompressors, and the last follows symlinks
/// out of the repository.
const RG_LONG_OPTIONS: &[&str] = &[
    "byte-offset",
    "case-sensitive",
    "color",
    "column",
    "context",
    "count",
    "count-matches",
    "crlf",
    "engine",
    "files",
    "files-with-matches",
    "files-without-match",
    "fixed-strings",
    "glob",
    "heading",
    "hidden",
    "iglob",
    "ignore-case",
    "invert-match",
    "json",
    "line-number",
    "line-regexp",
    "max-count",
    "max-depth",
    "max-filesize",
    "multiline",
    "no-config",
    "no-filename",
    "no-heading",
    "no-ignore",
    "no-line-number",
    "no-messages",
    "no-require-git",
    "null",
    "one-file-system",
    "only-matching",
    "pcre2",
    "quiet",
    "regexp",
    "replace",
    "smart-case",
    "sort",
    "sortr",
    "stats",
    "text",
    "threads",
    "trim",
    "type",
    "type-not",
    "unrestricted",
    "vimgrep",
    "with-filename",
    "word-regexp",
];

/// Short forms of the same set. Bundles such as `-in` are accepted only if every letter appears here.
const RG_SHORT_OPTIONS: &str = "ABCFHINSTcefgilmnostuvwx";

/// Whether every `rg` argument is a permitted option, a pattern, or a path.
///
/// `--` ends option parsing in ripgrep itself, so everything after it is positional and is checked
/// only by the shared path confinement. Tracking that here keeps a leading-dash *pattern* usable,
/// which is the ordinary reason to write `--` at all.
fn rg_arguments_are_allowed(args: &[String]) -> bool {
    let mut positional_only = false;
    for arg in args {
        if positional_only {
            continue;
        }
        if arg == "--" {
            positional_only = true;
            continue;
        }
        if !is_allowed_rg_argument(arg) {
            return false;
        }
    }
    true
}

/// Whether one `rg` argument is a permitted option, a pattern, or a path.
///
/// Anything not beginning with `-` is a pattern or a path and is already confined by
/// [`validate_common_command_shape`], so only option-shaped arguments are checked here. A value that
/// itself begins with `-` must be written as `--option=value`; that is a deliberate false refusal,
/// because distinguishing an option from an option-shaped value requires knowing which options take
/// values, and being wrong about that is how an allowlist silently admits the argument after `--pre`.
fn is_allowed_rg_argument(arg: &str) -> bool {
    if arg == "--" || !arg.starts_with('-') || arg == "-" {
        return true;
    }
    let name = arg.split('=').next().unwrap_or(arg);
    if let Some(long) = name.strip_prefix("--") {
        return RG_LONG_OPTIONS.contains(&long);
    }
    name.strip_prefix('-').is_some_and(|shorts| {
        !shorts.is_empty() && shorts.chars().all(|short| RG_SHORT_OPTIONS.contains(short))
    })
}

/// Whether one `bash` invocation names a script on the fixed list.
///
/// A leading `./` is accepted because it names the identical file, and refusing it is a usability
/// trap with no security value. Traversal, absolute paths, and `..` are already refused by
/// `validate_common_command_shape` before this runs, so this function only has to decide
/// membership and argument shape.
///
/// Only the base-image check takes an argument. The doc gates are argument-free, and keeping them
/// that way means a caller cannot reach a script's own option surface through this exception.
/// The fixed shell-script list, for surfaces that must report it rather than restate it.
///
/// The capability manifest previously carried its own hand-written copy of this list and fell
/// behind the moment the list changed, which is the worst possible staleness: the manifest exists so
/// a client can tell a missing capability from a stale binary.
/// The ripgrep options this server permits, for the capability manifest to advertise.
///
/// Exposed so the manifest can derive the list instead of describing it. The hand-written `["search"]`
/// it replaces was accurate when any argument was accepted and survived C37 replacing that with a
/// positive allowlist, so it went on describing a surface that no longer existed.
pub fn allowed_rg_long_options() -> &'static [&'static str] {
    RG_LONG_OPTIONS
}

pub fn allowed_shell_scripts() -> &'static [&'static str] {
    ALLOWED_SHELL_SCRIPTS
}

fn is_allowed_shell_script(args: &[String]) -> bool {
    let Some(first) = args.first() else {
        return false;
    };
    let script = first.strip_prefix("./").unwrap_or(first);

    if !ALLOWED_SHELL_SCRIPTS.contains(&script) {
        return false;
    }

    if script == BASE_IMAGE_SCRIPT {
        return args.len() == 1 || (args.len() == 2 && args[1] == "task");
    }
    args.len() == 1
}

/// pytest accepts node ids and options freely, so refuse the options that load a caller-named
/// plugin. Outside-repository paths are already refused by argument path confinement, and ambient
/// plugin autoload is disabled for pytest children in the bounded runner.
fn is_pytest_plugin_option(arg: &str) -> bool {
    arg == PYTEST_PLUGIN_OPTION
        || (arg.starts_with(PYTEST_PLUGIN_OPTION) && !arg.starts_with(LONG_OPTION_PREFIX))
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
        checked_command_timeout, is_pytest_plugin_option, redact_and_truncate_output,
        redact_and_truncate_output_tail, run_guarded_command, validate_command,
    };
    use crate::process::runner::redact_line;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn refusal(program: &str, values: &[&str]) -> String {
        validate_command(program, &args(values))
            .expect_err("policy must refuse this command")
            .to_string()
    }

    #[test]
    fn refuses_azure_cli() {
        for values in [
            vec!["login"],
            vec!["account", "show"],
            vec!["bicep", "build"],
        ] {
            assert!(refusal("az", &values).contains("not allowlisted"));
        }
    }

    #[test]
    fn refuses_npx() {
        let message = refusal("npx", &["-y", "@azure/mcp", "server", "start"]);
        assert!(message.contains("not allowlisted"), "unexpected: {message}");
    }

    #[test]
    fn refuses_package_installation() {
        for (program, values) in [
            ("npm", vec!["install", "-g", "@azure/mcp"]),
            ("npm", vec!["ci"]),
            ("pnpm", vec!["add", "left-pad"]),
            ("pnpm", vec!["install"]),
            ("bun", vec!["add", "left-pad"]),
            ("bun", vec!["install"]),
        ] {
            assert!(refusal(program, &values).contains("not allowlisted"));
        }
    }

    #[test]
    fn refuses_arbitrary_python_module_execution() {
        for values in [
            vec!["-m", "http.server"],
            vec!["-c", "import os"],
            vec!["-m", "pip", "install", "requests"],
        ] {
            assert!(refusal("python3", &values).contains("not allowlisted"));
            assert!(refusal("python", &values).contains("not allowlisted"));
        }
    }

    #[test]
    fn refuses_generic_shell_programs_and_strings() {
        for (program, values) in [
            ("bash", vec!["-c", "echo hi"]),
            ("bash", vec!["scripts/anything.sh"]),
            ("sh", vec!["-c", "echo hi"]),
            ("zsh", vec!["-c", "echo hi"]),
            ("env", vec!["echo", "hi"]),
        ] {
            assert!(refusal(program, &values).contains("not allowlisted"));
        }
    }

    #[test]
    fn permits_the_fixed_repository_validation_scripts() {
        for values in [
            vec!["scripts/check-docs.sh"],
            vec!["./scripts/check-docs.sh"],
            vec!["scripts/check-doc-commands.sh"],
            vec!["scripts/docs-audit.sh"],
            vec!["scripts/check-endpoint-literals.sh"],
            vec!["scripts/check-hosted-target-readiness.sh"],
        ] {
            validate_command("bash", &args(&values))
                .unwrap_or_else(|error| panic!("{values:?} must be permitted: {error}"));
        }
    }

    /// The exception is fixed to named scripts, so neighbouring paths and argument surfaces stay
    /// refused. A glob over `scripts/` would admit every case below.
    #[test]
    fn refuses_shell_scripts_outside_the_fixed_set() {
        for values in [
            // Not on the list, though it sits in the same directory.
            vec!["scripts/unlisted.sh"],
            // Right basename, wrong directory.
            vec!["tools/check-docs.sh"],
            vec!["scripts/check-docs.sh.bak"],
            // The doc gates take no arguments, so their own option surface stays out of reach.
            vec!["scripts/check-docs.sh", "--fix"],
            // `task` belongs to the base-image check alone.
            vec!["scripts/docs-audit.sh", "task"],
            vec!["-c", "scripts/check-docs.sh"],
        ] {
            let message = refusal("bash", &values);
            assert!(
                message.contains("not allowlisted"),
                "unexpected refusal for {values:?}: {message}"
            );
        }
    }

    #[test]
    fn refuses_traversal_through_an_allowlisted_script_name() {
        let message = refusal("bash", &["scripts/../../etc/check-docs.sh"]);
        assert!(
            message.contains("outside the repository root"),
            "unexpected refusal: {message}"
        );
    }

    #[test]
    fn names_the_fixed_script_list_when_refusing_a_shell_gate() {
        // The old message sent an unlisted gate to `validation_profile_run`, which a caller cannot
        // extend, so the advice could never resolve the refusal.
        let message = refusal("bash", &["scripts/unlisted.sh"]);
        assert!(
            message.contains("fixed validation-script list"),
            "refusal must name the list: {message}"
        );
    }

    /// Formatting is a mutation; checking is not. Only the second is admitted.
    #[test]
    fn permits_cargo_fmt_only_as_a_check() {
        for values in [
            vec!["fmt", "--", "--check"],
            vec!["fmt", "--all", "--", "--check"],
            vec!["fmt", "--check"],
            // A package name is a value rather than an option, so it is not matched against the list.
            vec!["fmt", "-p", "core", "--check"],
        ] {
            validate_command("cargo", &args(&values))
                .unwrap_or_else(|error| panic!("{values:?} must be permitted: {error}"));
        }

        for values in [
            vec!["fmt"],
            vec!["fmt", "--all"],
            vec!["fmt", "--check", "--emit", "files"],
            // Refused for being unnamed rather than for writing. Measured against the current
            // rustfmt neither of these wrote a file under `--check`, and the list does not rest on
            // that measurement surviving an upgrade.
            vec!["fmt", "--check", "--config", "emit_mode=Files"],
            vec!["fmt", "--check", "--config-path", "rustfmt.toml"],
            // `-p` consumes the argument after it, so the check is no longer present.
            vec!["fmt", "-p", "--check"],
            vec!["fmt", "--check", "-p"],
        ] {
            let message = refusal("cargo", &values);
            assert!(
                message.contains("not allowlisted"),
                "unexpected refusal for {values:?}: {message}"
            );
        }
    }

    /// The options that make `rg` a program launcher rather than a search tool.
    ///
    /// `--pre sh --pre-glob '*'` was demonstrated executing a shell over repository files and
    /// writing outside the repository root, which defeated the fixed shell-script list, the
    /// repository-relative Python rule, and the pytest hardening at once.
    #[test]
    fn refuses_rg_options_that_start_programs() {
        for values in [
            vec!["--pre", "sh", "."],
            vec!["--pre=sh", "."],
            vec!["--pre-glob", "*", "."],
            vec!["--hostname-bin", "hostname", "."],
            vec!["-z", "pattern"],
            vec!["--search-zip", "pattern"],
            // Following symlinks reads outside the repository even when argv stays inside it.
            vec!["--follow", "pattern"],
            vec!["-L", "pattern"],
            // A bundle is only as safe as its least safe letter.
            vec!["-iz", "pattern"],
        ] {
            let message = refusal("rg", &values);
            assert!(
                message.contains("not allowlisted"),
                "unexpected refusal for {values:?}: {message}"
            );
        }
    }

    #[test]
    fn retains_ordinary_rg_search_invocations() {
        for values in [
            vec!["--files"],
            vec!["pattern"],
            vec!["-n", "pattern", "src"],
            vec!["-i", "--glob", "*.rs", "pattern"],
            vec!["-e", "pattern", "--max-depth", "2"],
            vec!["--json", "pattern"],
            vec!["-inH", "pattern"],
            vec!["--", "-dash-leading-pattern"],
        ] {
            validate_command("rg", &args(&values))
                .unwrap_or_else(|error| panic!("{values:?} must stay permitted: {error}"));
        }
    }

    /// `rev-list` and `shortlog` are admitted, and their neighbours in the same namespace are not.
    ///
    /// Both only read history, but they sit beside `fetch`, `push` and `commit` under one program, so
    /// the boundary is a property of this list rather than of the word `git`. The refusals are what
    /// make that legible: without them, adding two read subcommands looks indistinguishable from
    /// widening the program.
    ///
    /// Their absence had a measured cost. Counting commits was impossible through this surface, so it
    /// was done by eye from `log` output, and got the same number wrong twice in two consecutive
    /// messages whose subject was that number.
    #[test]
    fn admits_git_history_reads_without_admitting_their_neighbours() {
        for values in [
            vec!["rev-list", "--count", "main..HEAD"],
            vec!["shortlog", "--summary", "--numbered"],
        ] {
            validate_command("git", &args(&values))
                .unwrap_or_else(|error| panic!("{values:?} must be permitted: {error}"));
        }

        for values in [
            vec!["fetch", "origin"],
            vec!["push", "origin", "HEAD"],
            vec!["commit", "-m", "subject"],
            vec!["add", "README.md"],
            vec!["reset", "--hard"],
            vec!["clean", "-fd"],
        ] {
            let message = refusal("git", &values);
            assert!(
                message.contains("not allowlisted"),
                "unexpected refusal for {values:?}: {message}"
            );
        }
    }

    #[test]
    fn refuses_pytest_plugin_loading_options() {
        for values in [
            vec!["-p", "attacker_plugin"],
            vec!["-pattacker_plugin"],
            vec!["tests", "-p", "no:cacheprovider"],
        ] {
            let message = refusal("pytest", &values);
            assert!(
                message.contains("loads a caller-named plugin"),
                "unexpected refusal: {message}"
            );
        }
    }

    #[test]
    fn retains_repository_local_pytest_invocations() {
        for values in [
            vec!["tests"],
            vec!["-q", "tests/test_thing.py"],
            vec!["--maxfail", "1"],
        ] {
            validate_command("pytest", &args(&values))
                .expect("repository-local pytest invocations stay permitted");
        }
    }

    #[test]
    fn refuses_pytest_paths_outside_the_repository() {
        for values in [
            vec!["/etc"],
            vec!["../other-repo/tests"],
            vec!["tests/../../escape"],
            vec!["--rootdir", "/tmp"],
        ] {
            let message = refusal("pytest", &values);
            assert!(
                message.contains("outside the repository root"),
                "unexpected refusal: {message}"
            );
        }
    }

    #[test]
    fn refuses_program_given_as_a_path() {
        for program in ["/usr/bin/env", "./scripts/run", "../bin/az"] {
            let message = refusal(program, &["--version"]);
            assert!(
                message.contains("not a path"),
                "unexpected refusal: {message}"
            );
        }
    }

    #[test]
    fn long_options_are_not_mistaken_for_plugin_loading() {
        assert!(!is_pytest_plugin_option("--pyargs"));
        assert!(!is_pytest_plugin_option("--maxfail"));
        assert!(is_pytest_plugin_option("-p"));
    }

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
