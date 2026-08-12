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

use crate::error::ContextPatchError;
use crate::git::repository::GitRepository;
use crate::process::runner::{run_bounded_command, BoundedProcessOutput};
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
    repository: impl Into<GitRepository<'a>>,
    args: &[&str],
) -> Result<BoundedProcessOutput, ContextPatchError> {
    let cwd = repository.into().command_cwd();
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
    repository: impl Into<GitRepository<'a>>,
    args: &[&str],
) -> Result<BoundedProcessOutput, ContextPatchError> {
    let output = run(repository, args)?;
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
    repository: impl Into<GitRepository<'a>>,
    args: &[&str],
) -> Result<String, ContextPatchError> {
    let output = output(repository, args)?;
    String::from_utf8(output.stdout)
        .map_err(|error| ContextPatchError::new(format!("git output was not UTF-8: {error}")))
}

/// Run `git` and require success, discarding output.
pub fn success<'a>(
    repository: impl Into<GitRepository<'a>>,
    args: &[String],
) -> Result<(), ContextPatchError> {
    let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let _ = output(repository, &arg_refs)?;
    Ok(())
}

/// Run `git` and report its exit code without treating a non-zero code as a refusal.
///
/// Timeout and truncation are still refused, because those mean the code is not trustworthy.
pub fn exit_code<'a>(
    repository: impl Into<GitRepository<'a>>,
    args: &[&str],
) -> Result<i32, ContextPatchError> {
    Ok(run(repository, args)?.exit_code)
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
/// A rename or copy is one record spanning two NUL-separated pieces: the new path carrying the status
/// characters, then the original path bare. Both sides are collected, because both changed and a
/// caller listing an exact path set must be able to name them. Picking one and discarding the other
/// would silently choose wrongly, which is why this used to refuse the whole record instead.
///
/// Similarity is deliberately not consulted. Porcelain v1 status reports `R` without a score, so a
/// pure move and a move carrying edits are indistinguishable here; requiring one would mean running a
/// second command to learn something this one cannot say. The exact path set is the guard: a caller
/// commits a rename only by naming both sides, exactly as it names any other change.
pub fn parse_porcelain_paths(
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let mut paths = BTreeSet::new();
    let mut entries = porcelain_entries(bytes);
    while let Some(entry) = entries.next() {
        ensure_porcelain_entry_shape(entry, label)?;
        paths.insert(porcelain_entry_path(entry, label)?);
        if is_rename_or_copy(entry) {
            paths.insert(porcelain_original_path(&mut entries, label)?);
        }
    }
    Ok(paths)
}

/// Every path Git reports only as the source of a staged rename.
///
/// These cannot be staged by pathspec. `git mv` has already removed the source from both the index and
/// the worktree, so `git add -- <source>` matches nothing and aborts the entire invocation, taking the
/// other paths with it. They remain part of the changed set a caller must name, which is why the gate
/// and the staging step need different lists: naming both sides is how a caller acknowledges the
/// rename, while only one side can be handed to `git add`.
///
/// A copy's source is deliberately excluded. A copy leaves its original in place, so pathspec matches
/// it and there is nothing to work around.
pub fn parse_porcelain_rename_sources(
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let mut sources = BTreeSet::new();
    let mut entries = porcelain_entries(bytes);
    while let Some(entry) = entries.next() {
        ensure_porcelain_entry_shape(entry, label)?;
        if is_rename_or_copy(entry) {
            let original = porcelain_original_path(&mut entries, label)?;
            if entry[0] == b'R' || entry[1] == b'R' {
                sources.insert(original);
            }
        }
    }
    Ok(sources)
}

/// Parse porcelain v1 entries, keeping only untracked paths.
///
/// A rename's original path is consumed and dropped: it is tracked by definition, so it can never be
/// one of the paths this returns, but leaving it in the stream would fail the entry shape check.
pub fn parse_untracked_porcelain_paths(
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let mut paths = BTreeSet::new();
    let mut entries = porcelain_entries(bytes);
    while let Some(entry) = entries.next() {
        ensure_porcelain_entry_shape(entry, label)?;
        if is_rename_or_copy(entry) {
            porcelain_original_path(&mut entries, label)?;
            continue;
        }
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
    let mut entries = porcelain_entries(bytes);
    while let Some(entry) = entries.next() {
        ensure_porcelain_entry_shape(entry, label)?;
        if is_rename_or_copy(entry) {
            porcelain_original_path(&mut entries, label)?;
            continue;
        }
        if (entry[0] == b'?' && entry[1] == b'?') || (entry[0] == b'!' && entry[1] == b'!') {
            paths.insert(
                porcelain_entry_path(entry, label)?
                    .trim_end_matches('/')
                    .to_string(),
            );
        }
    }
    Ok(paths)
}

/// Parse `--name-status -z` output into the set of paths it reports.
///
/// The field layout is not porcelain's. Porcelain packs the two status characters and a space into the
/// front of the path field; `--name-status -z` emits the status as its own NUL-terminated field, so a
/// normal change is two fields and a rename or copy is three, the score being part of the status field
/// as `R100` rather than a bare `R`. Reusing the porcelain parser here would misread every record.
///
/// Both sides of a rename are collected for the same reason as in [`parse_porcelain_paths`]: the index
/// holds a deletion and an addition, so a caller's exact set names both, and reporting one would make
/// the staged set differ from the requested set for a rename that staged correctly.
pub fn parse_name_status_paths(
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let mut paths = BTreeSet::new();
    let mut fields = porcelain_entries(bytes);
    while let Some(status) = fields.next() {
        let carries_two_paths = matches!(status.first(), Some(b'R') | Some(b'C'));
        let path = fields.next().ok_or_else(|| {
            ContextPatchError::new(format!("{label} reported a status with no path"))
        })?;
        paths.insert(porcelain_path(path, label)?);
        if carries_two_paths {
            let destination = fields.next().ok_or_else(|| {
                ContextPatchError::new(format!(
                    "{label} reported a rename or copy with only one path"
                ))
            })?;
            paths.insert(porcelain_path(destination, label)?);
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
        return Err(ContextPatchError::new(format!("unexpected {label} entry")));
    }
    Ok(())
}

/// Whether either status character marks this record as a rename or a copy.
///
/// Both are reported the same way and carry the same two-piece shape, so both are handled together.
/// No action in this surface produces a copy, but treating it as a special case would leave a second
/// code path that nothing exercises.
fn is_rename_or_copy(entry: &[u8]) -> bool {
    matches!(entry[0], b'R' | b'C') || matches!(entry[1], b'R' | b'C')
}

/// Consume the bare original path that follows a rename or copy record.
///
/// It carries no status characters, so it must be taken from the stream here rather than reaching the
/// entry shape check, which would reject it.
fn porcelain_original_path<'a>(
    entries: &mut impl Iterator<Item = &'a [u8]>,
    label: &str,
) -> Result<String, ContextPatchError> {
    let original = entries.next().ok_or_else(|| {
        ContextPatchError::new(format!(
            "{label} reported a rename or copy with no original path"
        ))
    })?;
    porcelain_path(original, label)
}

fn porcelain_entry_path(entry: &[u8], label: &str) -> Result<String, ContextPatchError> {
    porcelain_path(&entry[3..], label)
}

fn porcelain_path(bytes: &[u8], label: &str) -> Result<String, ContextPatchError> {
    std::str::from_utf8(bytes)
        .map(str::to_string)
        .map_err(|error| ContextPatchError::new(format!("{label} path is not UTF-8: {error}")))
}

/// Every path Git reports as changed, including untracked files.
pub fn status_paths<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = output(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_porcelain_paths(&output.stdout, STATUS_LABEL)
}

/// Every path Git reports only as the source of a staged rename.
pub fn rename_source_paths<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = output(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_porcelain_rename_sources(&output.stdout, STATUS_LABEL)
}

/// Only the untracked paths Git reports.
pub fn untracked_paths<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = output(
        repository,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    parse_untracked_porcelain_paths(&output.stdout, STATUS_LABEL)
}

/// The untracked and ignored paths Git reports.
pub fn untracked_and_ignored_paths<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = output(
        repository,
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
///
/// `--name-status` rather than `--name-only`, because `--name-only` reports a rename as its new path
/// alone. The exact set a caller must name holds both sides, so with `--name-only` a correctly staged
/// rename verified as a mismatch after staging had already happened.
pub fn cached_paths<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = output(repository, &["diff", "--cached", "--name-status", "-z"])?;
    parse_name_status_paths(&output.stdout, CACHED_DIFF_LABEL)
}

/// Short-format status text, including untracked files.
pub fn status_short<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<String, ContextPatchError> {
    stdout(repository, &["status", "--short", "--untracked-files=all"])
}

/// The current head commit, or [`UNBORN_HEAD`] when the repository has no commits.
///
/// A failing `rev-parse` is only treated as an unborn head when the repository genuinely has no
/// revisions, so a real failure is still reported rather than disguised as an empty repository.
pub fn head<'a>(repository: impl Into<GitRepository<'a>>) -> Result<String, ContextPatchError> {
    // Converted once and reused, which the copyable target makes free.
    let repository = repository.into();
    let probe = run(repository, &["rev-parse", "--verify", "HEAD"])?;
    if probe.success() {
        return String::from_utf8(probe.stdout)
            .map(|head| head.trim().to_string())
            .map_err(|error| ContextPatchError::new(format!("git output was not UTF-8: {error}")));
    }

    let revision_count = stdout(repository, &["rev-list", "--all", "--count"])?;
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
pub fn current_branch<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<String, ContextPatchError> {
    let branch = stdout(repository, &["branch", "--show-current"])?;
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
pub fn local_branch_exists<'a>(
    repository: impl Into<GitRepository<'a>>,
    branch: &str,
) -> Result<bool, ContextPatchError> {
    let branch_ref = format!("refs/heads/{branch}");
    match exit_code(
        repository,
        &["show-ref", "--verify", "--quiet", &branch_ref],
    )? {
        0 => Ok(true),
        1 => Ok(false),
        code => Err(ContextPatchError::new(format!(
            "failed to check local branch `{branch}` (exit code {code})"
        ))),
    }
}

/// Count the revisions in a range.
pub fn rev_count<'a>(
    repository: impl Into<GitRepository<'a>>,
    range: &str,
) -> Result<u64, ContextPatchError> {
    let counted = stdout(repository, &["rev-list", "--count", range])?;
    counted.trim().parse::<u64>().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to parse revision count `{}`: {error}",
            counted.trim()
        ))
    })
}

/// Require that a remote is configured.
pub fn ensure_remote_exists<'a>(
    repository: impl Into<GitRepository<'a>>,
    remote: &str,
) -> Result<(), ContextPatchError> {
    let remotes = stdout(repository, &["remote"])?;
    if remotes.lines().any(|line| line == remote) {
        Ok(())
    } else {
        Err(ContextPatchError::new(format!(
            "remote `{remote}` is not configured"
        )))
    }
}

/// Resolve a ref to the commit it names.
pub fn resolve_commit<'a>(
    repository: impl Into<GitRepository<'a>>,
    ref_name: &str,
) -> Result<String, ContextPatchError> {
    let commit_ref = format!("{ref_name}^{{commit}}");
    let commit = stdout(repository, &["rev-parse", "--verify", &commit_ref])?;
    Ok(commit.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
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
        assert!(
            reported.chars().all(|c| c.is_ascii_hexdigit()),
            "{reported}"
        );
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
    fn cached_paths_report_both_sides_of_a_staged_rename() {
        // The index holds a deletion and an addition, and the exact set a caller names holds both, so
        // reporting only the new path made a correctly staged rename verify as a mismatch.
        let root = committed_repo("cached_paths_rename");
        fs::write(root.join("old.txt"), "moved\n").unwrap();
        git(&root, &["add", "old.txt"]);
        git(&root, &["commit", "--quiet", "-m", "add old"]);
        git(&root, &["mv", "old.txt", "new.txt"]);

        assert_eq!(cached_paths(&root).unwrap(), names(&["new.txt", "old.txt"]));
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
            ensure_remote_exists(&root, "origin")
                .unwrap_err()
                .to_string(),
            "remote `origin` is not configured"
        );

        git(
            &root,
            &[
                "remote",
                "add",
                "origin",
                "https://example.invalid/repo.git",
            ],
        );
        assert!(ensure_remote_exists(&root, "origin").is_ok());
    }

    #[test]
    fn a_failing_git_invocation_reports_the_argv_and_stderr() {
        let root = committed_repo("failing_git");

        let error = stdout(&root, &["rev-parse", "--verify", "refs/heads/absent"])
            .unwrap_err()
            .to_string();

        assert!(
            error.starts_with("git rev-parse --verify refs/heads/absent failed:"),
            "{error}"
        );
    }

    #[test]
    fn an_exit_code_is_reported_without_treating_failure_as_a_refusal() {
        let root = committed_repo("exit_code");

        assert_eq!(
            exit_code(
                &root,
                &["show-ref", "--verify", "--quiet", "refs/heads/main"]
            )
            .unwrap(),
            0
        );
        assert_eq!(
            exit_code(
                &root,
                &["show-ref", "--verify", "--quiet", "refs/heads/absent"]
            )
            .unwrap(),
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
        assert!(
            error.starts_with("git status --porcelain output exceeded"),
            "{error}"
        );
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
    fn a_rename_entry_yields_both_of_its_paths() {
        // Porcelain reports a rename as two pieces in one record: the new path with the status
        // characters, then the original bare. Both changed, so a caller naming an exact path set has to
        // be able to name both, and picking one would silently discard the other.
        let entry = b"R  new.txt\0old.txt\0";

        let paths = parse_porcelain_paths(entry, STATUS_LABEL).unwrap();

        assert_eq!(
            paths,
            BTreeSet::from(["new.txt".to_string(), "old.txt".to_string()])
        );
    }

    #[test]
    fn a_copy_entry_yields_both_of_its_paths() {
        // No action in this surface produces a copy, but it carries the identical two-piece shape, so
        // it takes the identical path rather than a second one nothing exercises.
        let entry = b"C  copy.txt\0source.txt\0";

        let paths = parse_porcelain_paths(entry, STATUS_LABEL).unwrap();

        assert_eq!(
            paths,
            BTreeSet::from(["copy.txt".to_string(), "source.txt".to_string()])
        );
    }

    #[test]
    fn a_rename_beside_other_changes_does_not_consume_them() {
        // The original path is taken from the stream by position, so an off-by-one here would swallow
        // the following record and report a clean path as unchanged.
        let entries = b"R  new.txt\0old.txt\0 M kept.txt\0?? extra.txt\0";

        let paths = parse_porcelain_paths(entries, STATUS_LABEL).unwrap();

        assert_eq!(
            paths,
            BTreeSet::from([
                "new.txt".to_string(),
                "old.txt".to_string(),
                "kept.txt".to_string(),
                "extra.txt".to_string()
            ])
        );
    }

    #[test]
    fn a_rename_missing_its_original_path_is_refused() {
        // A truncated record must not be read as a single-path change, which would report the rename as
        // an addition and leave the deletion invisible.
        let entry = b"R  new.txt\0";

        assert_eq!(
            parse_porcelain_paths(entry, STATUS_LABEL)
                .unwrap_err()
                .to_string(),
            "git status reported a rename or copy with no original path"
        );
    }

    #[test]
    fn an_untracked_scan_drops_a_rename_original_without_failing_on_its_shape() {
        // The bare original path has no status characters, so leaving it in the stream would fail the
        // entry shape check and make any untracked scan error out once a rename existed.
        let entries = b"R  new.txt\0old.txt\0?? untracked.txt\0";

        let paths = parse_untracked_porcelain_paths(entries, STATUS_LABEL).unwrap();

        assert_eq!(paths, BTreeSet::from(["untracked.txt".to_string()]));
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
