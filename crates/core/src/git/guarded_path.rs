use std::path::{Component, Path, PathBuf};

use crate::error::ContextPatchError;
use crate::fs::rooted::{self, RootedEntryKind};
use crate::git::repository::GitRepository;
use crate::git::root::RepositoryRoot;
use crate::process::runner::{run_bounded_command, BoundedProcessOutput, CommandCwd};
use crate::process::GIT_SUBPROCESS_TIMEOUT;

/// Canonicalize a repository root, asserting nothing about worktrees.
///
/// This is the one canonicalization in the workspace. [`exact_worktree_root`] builds on it and then adds
/// the worktree requirement, so the two resolvers cannot drift on *how* a root is resolved while differing
/// only in what they additionally demand.
///
/// The underlying I/O error is returned rather than a worded refusal, because the two surfaces above this
/// word the failure differently and always have: `core` names the path, the MCP adapter reports a shorter
/// phrase. Sharing the resolution without sharing the wording is what makes consolidating it
/// behavior-neutral.
pub fn resolve_repo_root(repo_root: &Path) -> std::io::Result<PathBuf> {
    repo_root.canonicalize()
}

/// Resolve a repository root that must be *exactly* a Git worktree root.
///
/// Canonicalizes, requires a directory, and requires `rev-parse --show-toplevel` to equal the result.
/// It deliberately does not walk upward: a subdirectory of a worktree is refused rather than silently
/// promoted to its enclosing repository.
///
/// This is the policy an action needs whenever its blast radius is the repository or its remote rather
/// than a set of caller-named paths. Branch switching and pushing take no paths, so nothing else bounds
/// them; run under a subdirectory root they would act on the enclosing repository, touching files the
/// operator never configured. Path-taking actions have their own confinement and use the weaker
/// resolution.
pub fn exact_worktree_root(repo_root: &Path) -> Result<PathBuf, ContextPatchError> {
    worktree_root(repo_root, None)
}

/// The same check, with `rev-parse` running in an already-open directory rather than a named one.
///
/// Used while a workspace selection still holds its descriptor, so the worktree this confirms is the
/// worktree that was validated rather than whatever the name resolves to by the time the check runs.
pub fn exact_worktree_root_in(
    repo_root: &Path,
    cwd: CommandCwd<'_>,
) -> Result<PathBuf, ContextPatchError> {
    worktree_root(repo_root, Some(cwd))
}

fn worktree_root(
    repo_root: &Path,
    anchored_cwd: Option<CommandCwd<'_>>,
) -> Result<PathBuf, ContextPatchError> {
    let root = resolve_repo_root(repo_root).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    if !root.is_dir() {
        return Err(ContextPatchError::new(format!(
            "repository root {} is not a directory",
            root.display()
        )));
    }

    let cwd = anchored_cwd.unwrap_or(CommandCwd::Path(&root));
    let output = git_output(cwd, &["rev-parse", "--show-toplevel"], "resolve Git root")?;
    let git_root = std::str::from_utf8(&output.stdout)
        .map_err(|error| ContextPatchError::new(format!("Git root is not UTF-8: {error}")))?
        .trim();
    let git_root = Path::new(git_root).canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to canonicalize Git root {git_root}: {error}"
        ))
    })?;
    if git_root != root {
        return Err(ContextPatchError::new(format!(
            "configured repository root {} is not the Git worktree root {}",
            root.display(),
            git_root.display()
        )));
    }

    Ok(root)
}

/// Confine one existing regular file, returning its repository-relative spelling.
///
/// Containment comes from the descriptor walk under either authority, which refuses a symlink at any
/// component, so there is no second resolution to compare against a prefix. The resolved absolute path is
/// deliberately not returned: callers act through the root's authority and the relative spelling, which is
/// what stops a path being checked here and then used against a different directory.
pub(crate) fn resolve_existing_regular_file(
    root: RepositoryRoot<'_>,
    path: &Path,
) -> Result<String, ContextPatchError> {
    let relative = normalize_relative_path(path)?;

    match rooted::entry_kind(root, &relative)? {
        None => Err(ContextPatchError::new(format!(
            "source file `{relative}` does not exist"
        ))),
        Some(RootedEntryKind::Symlink) => Err(ContextPatchError::new(format!(
            "source file `{relative}` must not be a symlink"
        ))),
        Some(RootedEntryKind::RegularFile) => Ok(relative),
        Some(_) => Err(ContextPatchError::new(format!(
            "source path `{relative}` is not a regular file"
        ))),
    }
}

/// Confine one absent destination path, returning its repository-relative spelling.
///
/// The leaf must not exist and its parent must be a real directory, both established through the
/// descriptor walk under either authority.
pub(crate) fn resolve_absent_file(
    root: RepositoryRoot<'_>,
    path: &Path,
) -> Result<String, ContextPatchError> {
    let relative = normalize_relative_path(path)?;

    if rooted::entry_kind(root, &relative)?.is_some() {
        return Err(ContextPatchError::new(format!(
            "destination path `{relative}` already exists"
        )));
    }
    // The parent is named explicitly so an absent one is refused here rather than surfacing later as a
    // rename failure. A root-level leaf has no parent component to check.
    if let Some(parent) = Path::new(&relative).parent().and_then(|parent| {
        let parent = parent.to_string_lossy().to_string();
        (!parent.is_empty()).then_some(parent)
    }) {
        if !rooted::is_directory(root, &parent)? {
            return Err(ContextPatchError::new(format!(
                "destination parent for `{relative}` is not a directory"
            )));
        }
    }
    Ok(relative)
}

/// Apply the exact-worktree-root requirement to a root without re-resolving an anchored one.
///
/// An anchored selection was verified as a worktree root when it was selected, so checking it again here
/// would be exactly the reopen this phase removes. `None` means the caller should keep the root it already
/// holds; `Some` carries the resolved path a path-backed caller must switch to.
pub(crate) fn exact_worktree_root_of(
    root: RepositoryRoot<'_>,
) -> Result<Option<PathBuf>, ContextPatchError> {
    if root.is_anchored() {
        return Ok(None);
    }
    exact_worktree_root(root.logical_path()).map(Some)
}

pub(crate) fn ensure_tracked(
    repository: GitRepository<'_>,
    relative: &str,
    label: &str,
) -> Result<(), ContextPatchError> {
    if !path_is_tracked(repository, relative)? {
        return Err(ContextPatchError::new(format!(
            "{label} `{relative}` is not tracked by Git"
        )));
    }
    Ok(())
}

pub(crate) fn ensure_not_tracked(
    repository: GitRepository<'_>,
    relative: &str,
    label: &str,
) -> Result<(), ContextPatchError> {
    if path_is_tracked(repository, relative)? {
        return Err(ContextPatchError::new(format!(
            "{label} `{relative}` is already tracked in the Git index"
        )));
    }
    Ok(())
}

pub(crate) fn path_is_tracked(
    repository: GitRepository<'_>,
    relative: &str,
) -> Result<bool, ContextPatchError> {
    let output = git_output(
        repository,
        &["ls-files", "-z", "--", relative],
        "inspect tracked path",
    )?;
    for entry in output.stdout.split(|byte| *byte == 0) {
        if entry == relative.as_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) fn ensure_path_clean(
    repository: GitRepository<'_>,
    relative: &str,
) -> Result<(), ContextPatchError> {
    let output = git_output(
        repository,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            relative,
        ],
        "inspect source status",
    )?;
    if !output.stdout.is_empty() {
        return Err(ContextPatchError::new(format!(
            "tracked source `{relative}` must be clean before mutation; status: {}",
            display_nul_output(&output.stdout)
        )));
    }
    Ok(())
}

pub(crate) fn ensure_index_clean(repository: GitRepository<'_>) -> Result<(), ContextPatchError> {
    let output = git_output_allow_failure(
        repository,
        &["diff", "--cached", "--quiet", "--exit-code"],
        "inspect Git index",
    )?;
    match output.exit_code {
        0 => Ok(()),
        1 => Err(ContextPatchError::new(
            "Git index must be clean before moving a tracked file",
        )),
        _ => Err(ContextPatchError::new(format!(
            "failed to inspect Git index: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

pub(crate) fn move_with_git(
    repository: GitRepository<'_>,
    from: &str,
    to: &str,
) -> Result<(), ContextPatchError> {
    git_output(repository, &["mv", "--", from, to], "move tracked file")?;
    Ok(())
}

pub(crate) fn path_has_unstaged_deletion(
    repository: GitRepository<'_>,
    relative: &str,
) -> Result<bool, ContextPatchError> {
    let output = git_output(
        repository,
        &["status", "--porcelain=v1", "-z", "--", relative],
        "verify tracked deletion",
    )?;
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            entry.len() >= 4
                && entry[0] == b' '
                && entry[1] == b'D'
                && entry[2] == b' '
                && &entry[3..] == relative.as_bytes()
        }))
}

fn normalize_relative_path(path: &Path) -> Result<String, ContextPatchError> {
    let raw = path
        .to_str()
        .ok_or_else(|| ContextPatchError::new("path must be valid UTF-8"))?;
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.ends_with('/')
        || raw.contains("//")
        || raw.contains('\\')
        || raw.chars().any(char::is_control)
    {
        return Err(ContextPatchError::new(
            "path must be a normalized repository-relative path",
        ));
    }
    if raw.starts_with(':') || raw.chars().any(|ch| matches!(ch, '*' | '?' | '[')) {
        return Err(ContextPatchError::new(format!(
            "path `{raw}` contains Git pathspec metacharacters"
        )));
    }

    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(ContextPatchError::new(
                "path must be a normalized repository-relative path",
            ));
        };
        if part
            .to_str()
            .is_some_and(|value| value.eq_ignore_ascii_case(".git"))
        {
            return Err(ContextPatchError::new(
                "paths under the Git administrative directory are refused",
            ));
        }
        parts.push(part);
    }
    let normalized = parts
        .iter()
        .map(|part| part.to_str().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("/");
    if normalized != raw {
        return Err(ContextPatchError::new(
            "path must be a normalized repository-relative path",
        ));
    }
    Ok(normalized)
}

fn git_output<'a>(
    cwd: impl Into<CommandCwd<'a>>,
    args: &[&str],
    label: &str,
) -> Result<BoundedProcessOutput, ContextPatchError> {
    let output = git_output_allow_failure(cwd, args, label)?;
    if !output.success() {
        return Err(ContextPatchError::new(format!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output)
}

fn git_output_allow_failure<'a>(
    cwd: impl Into<CommandCwd<'a>>,
    args: &[&str],
    label: &str,
) -> Result<BoundedProcessOutput, ContextPatchError> {
    let args = args
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
    let output = run_bounded_command(cwd, "git", &args, GIT_SUBPROCESS_TIMEOUT, label)?;
    if output.timed_out {
        return Err(ContextPatchError::new(format!(
            "{label} timed out after {} seconds; the Git process was terminated",
            GIT_SUBPROCESS_TIMEOUT.as_secs()
        )));
    }
    if output.stdout_truncated || output.stderr_truncated {
        return Err(ContextPatchError::new(format!(
            "{label} refused: Git output exceeded the bounded capture limit"
        )));
    }
    Ok(output)
}

fn display_nul_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace('\0', "\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::ErrorKind;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-git-resolver-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn worktree(name: &str) -> PathBuf {
        let root = unique_dir(name);
        let status = Command::new("git")
            .current_dir(&root)
            .args(["init", "--quiet", "--initial-branch=main"])
            .status()
            .unwrap();
        assert!(status.success());
        root
    }

    #[test]
    fn resolution_reports_the_canonical_path() {
        let root = unique_dir("canonical");
        fs::create_dir_all(root.join("nested")).unwrap();

        assert_eq!(
            resolve_repo_root(&root.join("nested")).unwrap(),
            root.join("nested")
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolution_follows_a_symlinked_root_to_its_target() {
        // Canonicalization is the point: two spellings of one directory must resolve to one path, which is
        // what makes the derived scratch and lock keys stable.
        let root = unique_dir("symlinked");
        let real = root.join("real");
        fs::create_dir_all(&real).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        assert_eq!(resolve_repo_root(&link).unwrap(), real);
    }

    #[test]
    fn an_absent_root_is_an_error_the_caller_words_itself() {
        let root = unique_dir("absent");

        // The raw I/O error is returned so each surface keeps its own long-standing phrasing.
        let error = resolve_repo_root(&root.join("missing")).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound);
    }

    #[test]
    fn the_two_resolvers_differ_only_in_what_they_demand_beyond_resolution() {
        let plain = unique_dir("plain_directory");

        // A directory that is not a worktree resolves fine, and is refused only by the strict resolver.
        assert_eq!(resolve_repo_root(&plain).unwrap(), plain);
        assert!(exact_worktree_root(&plain).is_err());

        let root = worktree("worktree_root");
        assert_eq!(resolve_repo_root(&root).unwrap(), root);
        assert_eq!(exact_worktree_root(&root).unwrap(), root);
    }

    #[test]
    fn a_subdirectory_of_a_worktree_resolves_but_is_not_a_worktree_root() {
        let root = worktree("worktree_subdirectory");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();

        // This is the distinction the per-action policy depends on: resolution succeeds, so path-confined
        // actions keep working from a descendant root, while the unscoped mutations refuse.
        assert_eq!(resolve_repo_root(&nested).unwrap(), nested);
        let error = exact_worktree_root(&nested).unwrap_err().to_string();
        assert!(error.contains("is not the Git worktree root"), "{error}");
    }

    #[test]
    fn the_strict_resolver_reports_the_shared_resolution_failure() {
        let root = unique_dir("strict_absent");

        // Built on the shared resolver, so an absent root fails with this crate's wording rather than a
        // second implementation's.
        let error = exact_worktree_root(&root.join("missing"))
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with("failed to resolve repository root "),
            "{error}"
        );
    }
}
