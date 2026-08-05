//! Typed Git state and query guards.
//!
//! Running Git and interpreting what it reports is policy, not adaptation. The bounded-output contract
//! in particular is a guard: a truncated `git status` looks like a shorter list of dirty paths, and a
//! tool that acted on it would silently act on the wrong set. That refusal belongs beside the other Git
//! guards in `core` rather than in the MCP layer.
//!
//! Every refusal carries the detail only, never a prefix. The adapter owns how a refusal is addressed to
//! its caller, which is what lets this policy move without changing a single public refusal string.
//!
//! Execution reuses the shared no-shell primitive: `git` is invoked directly with an owned argument
//! vector, never through a shell, with the repository as the working directory and the shared Git
//! subprocess timeout.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::ContextPatchError;
use crate::process::runner::{run_bounded_command, BoundedProcessOutput, CommandCwd};
use crate::process::GIT_SUBPROCESS_TIMEOUT;

/// Reported as the head of a repository that has no commits yet.
///
/// A repository with an unborn head is a legitimate state, not a failure, so it gets a name rather than
/// an error. Callers that record a before-and-after head need something to record.
pub const UNBORN_HEAD: &str = "unborn";

/// Label used when describing a `git status` payload in a refusal.
pub const STATUS_LABEL: &str = "git status";

/// Label used when describing a staged-diff payload in a refusal.
pub const CACHED_DIFF_LABEL: &str = "git diff --cached";

/// Run `git` in the repository, refusing on timeout or truncated capture.
///
/// Does not inspect the exit code; a caller that needs a non-zero code to be meaningful uses
/// [`exit_code`], and a caller that needs success uses [`output`].
pub fn run<'a>(
    cwd: impl Into<CommandCwd<'a>>,
    args: &[&str],
) -> Result<BoundedProcessOutput, ContextPatchError> {
    let cwd = cwd.into();
    let owned_args = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    let output = run_bounded_command(
        cwd,
        "git",
        &owned_args,
        GIT_SUBPROCESS_TIMEOUT,
        "Git subprocess",
    )
    .map_err(|error| ContextPatchError::new(format!("failed to run git: {error}")))?;
    if output.timed_out {
        return Err(ContextPatchError::new(format!(
            "git {} timed out after {} seconds; the Git process was terminated",
            args.join(" "),
            GIT_SUBPROCESS_TIMEOUT.as_secs()
        )));
    }
    ensure_output_complete(args, &output)?;
    Ok(output)
}

/// Run `git` and require success.
pub fn output<'a>(
    cwd: impl Into<CommandCwd<'a>>,
    args: &[&str],
) -> Result<BoundedProcessOutput, ContextPatchError> {
    let output = run(cwd, args)?;
    if !output.success() {
        return Err(ContextPatchError::new(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output)
}

/// Run `git`, require success, and return stdout as UTF-8.
pub fn stdout<'a>(
    cwd: impl Into<CommandCwd<'a>>,
    args: &[&str],
) -> Result<String, ContextPatchError> {
    let output = output(cwd, args)?;
    String::from_utf8(output.stdout)
        .map_err(|error| ContextPatchError::new(format!("git output was not UTF-8: {error}")))
}

/// Run `git` and require success, discarding output.
pub fn success<'a>(
    cwd: impl Into<CommandCwd<'a>>,
    args: &[String],
) -> Result<(), ContextPatchError> {
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let _ = output(cwd, &arg_refs)?;
    Ok(())
}

/// Run `git` and report its exit code without treating a non-zero code as a refusal.
///
/// Timeout and truncation are still refused, because those mean the code is not trustworthy.
pub fn exit_code<'a>(
    cwd: impl Into<CommandCwd<'a>>,
    args: &[&str],
) -> Result<i32, ContextPatchError> {
    Ok(run(cwd, args)?.exit_code)
}

/// Refuse a bounded capture that filled up.
///
/// A truncated payload is not a smaller answer, it is an unknown one. Reporting fewer dirty paths than
/// exist would let a caller act on a set it never saw.
fn ensure_output_complete(
    args: &[&str],
    output: &BoundedProcessOutput,
) -> Result<(), ContextPatchError> {
    if output.stdout_truncated || output.stderr_truncated {
        return Err(ContextPatchError::new(format!(
            "git {} output exceeded the bounded capture; the result is incomplete and was not used",
            args.join(" ")
        )));
    }
    Ok(())
}

/// Parse NUL-separated porcelain v1 entries into the set of changed paths.
///
/// Rename and copy entries are refused rather than half-interpreted, because porcelain reports them as
/// two paths in one record and treating either as "the" path would silently pick the wrong one.
pub fn parse_porcelain_paths(
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let mut paths = BTreeSet::new();
    for entry in porcelain_entries(bytes) {
        ensure_porcelain_entry_shape(entry, label)?;
        if matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C') {
            return Err(ContextPatchError::new(
                "rename/copy entries require a future dedicated tool",
            ));
        }
        paths.insert(porcelain_entry_path(entry, label)?);
    }
    Ok(paths)
}

/// Parse porcelain v1 entries, keeping only untracked paths.
pub fn parse_untracked_porcelain_paths(
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let mut paths = BTreeSet::new();
    for entry in porcelain_entries(bytes) {
        ensure_porcelain_entry_shape(entry, label)?;
        if entry[0] == b'?' && entry[1] == b'?' {
            paths.insert(porcelain_entry_path(entry, label)?);
        }
    }
    Ok(paths)
}

/// Parse porcelain v1 entries, keeping untracked and ignored paths.
///
/// Ignored directories arrive with a trailing separator, which is trimmed so a directory and its own
/// path spell the same thing.
pub fn parse_untracked_and_ignored_porcelain_paths(
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let mut paths = BTreeSet::new();
    for entry in porcelain_entries(bytes) {
        ensure_porcelain_entry_shape(entry, label)?;
        if (entry[0] == b'?' && entry[1] == b'?') || (entry[0] == b'!' && entry[1] == b'!') {
            paths.insert(porcelain_entry_path(entry, label)?.trim_end_matches('/').to_string());
        }
    }
    Ok(paths)
}

/// Parse a plain NUL-separated path list, such as `--name-only -z` output.
pub fn parse_nul_paths(bytes: &[u8], label: &str) -> Result<BTreeSet<String>, ContextPatchError> {
    porcelain_entries(bytes)
        .map(|entry| {
            std::str::from_utf8(entry)
                .map(|path| path.to_string())
                .map_err(|error| {
                    ContextPatchError::new(format!("{label} path is not UTF-8: {error}"))
                })
        })
        .collect()
}

fn porcelain_entries(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
}

/// Porcelain v1 prefixes two status characters and one separating space before each path.
fn ensure_porcelain_entry_shape(entry: &[u8], label: &str) -> Result<(), ContextPatchError> {
    if entry.len() < 4 || entry[2] != b' ' {
        return Err(ContextPatchError::new(format!(
            "unexpected {label} entry"
        )));
    }
    Ok(())
}

fn porcelain_entry_path(entry: &[u8], label: &str) -> Result<String, ContextPatchError> {
    std::str::from_utf8(&entry[3..])
        .map(str::to_string)
        .map_err(|error| ContextPatchError::new(format!("{label} path is not UTF-8: {error}")))
}

/// Every path Git reports as changed, including untracked files.
pub fn status_paths(root: &Path) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = output(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_porcelain_paths(&output.stdout, STATUS_LABEL)
}

/// Only the untracked paths Git reports.
pub fn untracked_paths(root: &Path) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = output(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_untracked_porcelain_paths(&output.stdout, STATUS_LABEL)
}

/// The untracked and ignored paths Git reports.
pub fn untracked_and_ignored_paths(root: &Path) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = output(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )?;
    parse_untracked_and_ignored_porcelain_paths(&output.stdout, STATUS_LABEL)
}

/// The paths currently staged in the index.
pub fn cached_paths(root: &Path) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = output(root, &["diff", "--cached", "--name-only", "-z"])?;
    parse_nul_paths(&output.stdout, CACHED_DIFF_LABEL)
}

/// Short-format status text, including untracked files.
pub fn status_short(root: &Path) -> Result<String, ContextPatchError> {
    stdout(root, &["status", "--short", "--untracked-files=all"])
}

/// The current head commit, or [`UNBORN_HEAD`] when the repository has no commits.
///
/// A failing `rev-parse` is only treated as an unborn head when the repository genuinely has no
/// revisions, so a real failure is still reported rather than disguised as an empty repository.
pub fn head(root: &Path) -> Result<String, ContextPatchError> {
    let probe = run(root, &["rev-parse", "--verify", "HEAD"])?;
    if probe.success() {
        return String::from_utf8(probe.stdout)
            .map(|head| head.trim().to_string())
            .map_err(|error| {
                ContextPatchError::new(format!("git output was not UTF-8: {error}"))
            });
    }

    let revision_count = stdout(root, &["rev-list", "--all", "--count"])?;
    if revision_count.trim() == "0" {
        Ok(UNBORN_HEAD.to_string())
    } else {
        Err(ContextPatchError::new(format!(
            "git rev-parse --verify HEAD failed: {}",
            String::from_utf8_lossy(&probe.stderr).trim()
        )))
    }
}

/// The checked-out branch, refusing a detached head.
pub fn current_branch(root: &Path) -> Result<String, ContextPatchError> {
    let branch = stdout(root, &["branch", "--show-current"])?;
    let branch = branch.trim();
    if branch.is_empty() {
        Err(ContextPatchError::new(
            "repository is in detached HEAD state",
        ))
    } else {
        Ok(branch.to_string())
    }
}

/// Whether a local branch ref exists.
///
/// `show-ref --verify --quiet` answers with an exit code rather than output, so absence is a normal
/// answer and only an unexpected code is a refusal.
pub fn local_branch_exists(root: &Path, branch: &str) -> Result<bool, ContextPatchError> {
    let branch_ref = format!("refs/heads/{branch}");
    match exit_code(root, &["show-ref", "--verify", "--quiet", &branch_ref])? {
        0 => Ok(true),
        1 => Ok(false),
        code => Err(ContextPatchError::new(format!(
            "failed to check local branch `{branch}` (exit code {code})"
        ))),
    }
}

/// Count the revisions in a range.
pub fn rev_count(root: &Path, range: &str) -> Result<u64, ContextPatchError> {
    let counted = stdout(root, &["rev-list", "--count", range])?;
    counted.trim().parse::<u64>().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to parse revision count `{}`: {error}",
            counted.trim()
        ))
    })
}

/// Require that a remote is configured.
pub fn ensure_remote_exists(root: &Path, remote: &str) -> Result<(), ContextPatchError> {
    let remotes = stdout(root, &["remote"])?;
    if remotes.lines().any(|line| line == remote) {
        Ok(())
    } else {
        Err(ContextPatchError::new(format!(
            "remote `{remote}` is not configured"
        )))
    }
}

/// Resolve a ref to the commit it names.
pub fn resolve_commit(root: &Path, ref_name: &str) -> Result<String, ContextPatchError> {
    let commit_ref = format!("{ref_name}^{{commit}}");
    let commit = stdout(root, &["rev-parse", "--verify", &commit_ref])?;
    Ok(commit.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn empty_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-git-state-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        git(&root, &["init", "--quiet", "--initial-branch=main"]);
        git(&root, &["config", "user.email", "guard@example.invalid"]);
        git(&root, &["config", "user.name", "Guard"]);
        root
    }

    fn committed_repo(name: &str) -> PathBuf {
        let root = empty_repo(name);
        fs::write(root.join("tracked.txt"), "alpha\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "initial"]);
        root
    }

    fn names(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn an_unborn_repository_reports_a_named_head_rather_than_failing() {
        let root = empty_repo("unborn_head");

        // No commits is a legitimate state, and a caller recording a before-and-after head needs a
        // value for it.
        assert_eq!(head(&root).unwrap(), UNBORN_HEAD);
    }

    #[test]
    fn a_committed_repository_reports_its_head_and_branch() {
        let root = committed_repo("head_and_branch");

        let reported = head(&root).unwrap();
        assert_eq!(reported.len(), 40, "{reported}");
        assert!(reported.chars().all(|c| c.is_ascii_hexdigit()), "{reported}");
        assert_eq!(current_branch(&root).unwrap(), "main");
        assert_eq!(resolve_commit(&root, "HEAD").unwrap(), reported);
        assert_eq!(rev_count(&root, "HEAD").unwrap(), 1);
    }

    #[test]
    fn a_detached_head_is_refused_rather_than_reported_as_an_empty_branch() {
        let root = committed_repo("detached_head");
        let commit = head(&root).unwrap();
        git(&root, &["checkout", "--quiet", &commit]);

        assert_eq!(
            current_branch(&root).unwrap_err().to_string(),
            "repository is in detached HEAD state"
        );
    }

    #[test]
    fn status_reports_modified_and_untracked_paths_separately() {
        let root = committed_repo("status_paths");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("fresh.txt"), "beta\n").unwrap();

        assert_eq!(
            status_paths(&root).unwrap(),
            names(&["tracked.txt", "fresh.txt"])
        );
        assert_eq!(untracked_paths(&root).unwrap(), names(&["fresh.txt"]));
        assert!(!status_short(&root).unwrap().is_empty());
    }

    #[test]
    fn ignored_paths_are_reported_only_when_asked_for() {
        let root = committed_repo("ignored_paths");
        fs::write(root.join(".gitignore"), "build/\n").unwrap();
        fs::create_dir_all(root.join("build")).unwrap();
        fs::write(root.join("build").join("out.bin"), "binary").unwrap();

        let untracked = untracked_paths(&root).unwrap();
        assert!(untracked.contains(".gitignore"), "{untracked:?}");
        assert!(
            !untracked.iter().any(|path| path.starts_with("build")),
            "{untracked:?}"
        );

        // The ignored directory arrives with a trailing separator, which is trimmed.
        let with_ignored = untracked_and_ignored_paths(&root).unwrap();
        assert!(with_ignored.contains("build"), "{with_ignored:?}");
    }

    #[test]
    fn cached_paths_report_only_what_is_staged() {
        let root = committed_repo("cached_paths");
        fs::write(root.join("staged.txt"), "beta\n").unwrap();
        fs::write(root.join("loose.txt"), "gamma\n").unwrap();
        git(&root, &["add", "staged.txt"]);

        assert_eq!(cached_paths(&root).unwrap(), names(&["staged.txt"]));
    }

    #[test]
    fn a_local_branch_is_present_or_absent_without_refusing() {
        let root = committed_repo("local_branch");
        git(&root, &["branch", "feature"]);

        assert!(local_branch_exists(&root, "feature").unwrap());
        assert!(!local_branch_exists(&root, "absent").unwrap());
    }

    #[test]
    fn a_missing_remote_is_refused_by_name() {
        let root = committed_repo("missing_remote");

        assert_eq!(
            ensure_remote_exists(&root, "origin").unwrap_err().to_string(),
            "remote `origin` is not configured"
        );

        git(&root, &["remote", "add", "origin", "https://example.invalid/repo.git"]);
        assert!(ensure_remote_exists(&root, "origin").is_ok());
    }

    #[test]
    fn a_failing_git_invocation_reports_the_argv_and_stderr() {
        let root = committed_repo("failing_git");

        let error = stdout(&root, &["rev-parse", "--verify", "refs/heads/absent"])
            .unwrap_err()
            .to_string();

        assert!(error.starts_with("git rev-parse --verify refs/heads/absent failed:"), "{error}");
    }

    #[test]
    fn an_exit_code_is_reported_without_treating_failure_as_a_refusal() {
        let root = committed_repo("exit_code");

        assert_eq!(
            exit_code(&root, &["show-ref", "--verify", "--quiet", "refs/heads/main"]).unwrap(),
            0
        );
        assert_eq!(
            exit_code(&root, &["show-ref", "--verify", "--quiet", "refs/heads/absent"]).unwrap(),
            1
        );
    }

    #[test]
    fn a_truncated_capture_is_refused_rather_than_treated_as_a_shorter_answer() {
        // A truncated `git status` looks like a shorter list of dirty paths. A tool acting on it would
        // silently act on a set it never saw, so the incomplete payload is refused outright.
        let output = BoundedProcessOutput {
            cwd: PathBuf::from("/repository"),
            exit_code: 0,
            timed_out: false,
            duration_ms: 1,
            stdout: b"partial".to_vec(),
            stderr: Vec::new(),
            stdout_truncated: true,
            stderr_truncated: false,
        };

        let error = ensure_output_complete(&["status", "--porcelain"], &output)
            .unwrap_err()
            .to_string();

        assert!(error.contains("result is incomplete"), "{error}");
        assert!(error.contains("was not used"), "{error}");
        // The detail names the argv it ran, and carries no tool prefix.
        assert!(error.starts_with("git status --porcelain output exceeded"), "{error}");
    }

    #[test]
    fn a_truncated_stderr_is_refused_the_same_way() {
        let output = BoundedProcessOutput {
            cwd: PathBuf::from("/repository"),
            exit_code: 0,
            timed_out: false,
            duration_ms: 1,
            stdout: Vec::new(),
            stderr: b"partial".to_vec(),
            stdout_truncated: false,
            stderr_truncated: true,
        };

        assert!(ensure_output_complete(&["remote"], &output).is_err());
    }

    #[test]
    fn a_rename_entry_is_refused_rather_than_half_interpreted() {
        // Porcelain reports a rename as two paths in one record, so picking either would silently pick
        // the wrong one.
        let entry = b"R  new.txt\0old.txt\0";

        assert_eq!(
            parse_porcelain_paths(entry, STATUS_LABEL)
                .unwrap_err()
                .to_string(),
            "rename/copy entries require a future dedicated tool"
        );
    }

    #[test]
    fn a_malformed_porcelain_entry_is_refused_with_its_label() {
        assert_eq!(
            parse_porcelain_paths(b"XY\0", STATUS_LABEL)
                .unwrap_err()
                .to_string(),
            "unexpected git status entry"
        );
    }

    #[test]
    fn porcelain_parsing_selects_by_status_code() {
        let entries = b"?? fresh.txt\0 M tracked.txt\0!! build/\0";

        assert_eq!(
            parse_untracked_porcelain_paths(entries, STATUS_LABEL).unwrap(),
            names(&["fresh.txt"])
        );
        assert_eq!(
            parse_untracked_and_ignored_porcelain_paths(entries, STATUS_LABEL).unwrap(),
            names(&["fresh.txt", "build"])
        );
        assert_eq!(
            parse_nul_paths(b"one.txt\0two.txt\0", CACHED_DIFF_LABEL).unwrap(),
            names(&["one.txt", "two.txt"])
        );
    }
}
