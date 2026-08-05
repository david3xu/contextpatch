use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::tools;
use contextpatch_core::git::state as git_state;
use contextpatch_core::git::{GitRepository, RepositoryRoot};
pub(crate) fn validate_commit_subject(subject: &str) -> Result<String, String> {
    contextpatch_core::git::validate::commit_subject(subject)
        .map_err(|error| format!("{} refused: {error}", tools::git_commit_exact::NAME))
}

pub(crate) fn validate_commit_body(body: &str) -> Result<String, String> {
    contextpatch_core::git::validate::commit_body(body)
        .map_err(|error| format!("{} refused: {error}", tools::git_commit_exact::NAME))
}

pub(crate) fn normalize_git_paths<'a>(
    tool_name: &str,
    root: impl Into<RepositoryRoot<'a>>,
    paths: &[String],
) -> Result<Vec<String>, String> {
    contextpatch_core::git::validate::git_paths(root, paths)
        .map_err(|error| format!("{tool_name} refused: {error}"))
}

pub(crate) fn normalize_git_prefixes<'a>(
    tool_name: &str,
    root: impl Into<RepositoryRoot<'a>>,
    prefixes: &[String],
) -> Result<Vec<String>, String> {
    contextpatch_core::git::validate::git_prefixes(root, prefixes)
        .map_err(|error| format!("{tool_name} refused: {error}"))
}

pub(crate) fn normalize_git_path<'a>(
    tool_name: &str,
    root: impl Into<RepositoryRoot<'a>>,
    raw: &str,
) -> Result<String, String> {
    contextpatch_core::git::validate::git_path(root, raw)
        .map_err(|error| format!("{tool_name} refused: {error}"))
}

/// Prefix a moved-policy refusal so it is addressed to the calling tool.
///
/// `core` returns the detail only. This is the seam that keeps every public refusal string identical
/// while the policy behind it lives elsewhere.
fn refused(tool_name: &str, error: contextpatch_core::error::ContextPatchError) -> String {
    format!("{tool_name} refused: {error}")
}

pub(crate) fn git_status_paths_for_tool<'a>(
    tool_name: &str,
    repository: impl Into<GitRepository<'a>>,
) -> Result<BTreeSet<String>, String> {
    git_state::status_paths(repository).map_err(|error| refused(tool_name, error))
}

/// A repository target resolved under one action's root policy.
///
/// An anchored selection was already resolved when it was selected, and for the strict policy it was
/// already verified as a worktree root, so re-resolving it here would be exactly the reopen this phase
/// removes. A path-backed target still goes through the full existing check. Owning the resolved path is
/// what lets the borrowed target outlive it.
pub(crate) enum PolicyTarget {
    Anchored,
    Resolved(PathBuf),
}

impl PolicyTarget {
    pub(crate) fn bind<'a>(&'a self, original: GitRepository<'a>) -> GitRepository<'a> {
        match self {
            Self::Anchored => original,
            Self::Resolved(path) => GitRepository::from_path(path),
        }
    }

    /// The same decision, as a repository root rather than its Git projection.
    ///
    /// Path confinement needs the root's authority, not the Git projection, so that a path is checked
    /// against the same directory it will be used against.
    pub(crate) fn bind_root<'a>(&'a self, original: RepositoryRoot<'a>) -> RepositoryRoot<'a> {
        match self {
            Self::Anchored => original,
            Self::Resolved(path) => RepositoryRoot::from_path(path),
        }
    }
}

/// Apply an action's root policy to a repository target.
pub(crate) fn repository_under_policy(
    repository: GitRepository<'_>,
    tool_name: &str,
    policy: WorktreeRootPolicy,
) -> Result<PolicyTarget, String> {
    if repository.is_anchored() {
        return Ok(PolicyTarget::Anchored);
    }
    repo_root_for_policy(repository.path(), tool_name, policy).map(PolicyTarget::Resolved)
}

/// How strictly an action must verify its configured repository root.
///
/// The distinction is blast radius, not caution. An action that names the paths it touches is already
/// bounded by path normalization, which rejects parent components, absolute paths, and pathspec
/// metacharacters, so resolving the root is enough and a descendant root stays useful. An action that
/// names no paths has nothing else bounding it: run under a subdirectory root, branch switching and
/// pushing act on the *enclosing* repository and reach files the operator never configured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeRootPolicy {
    /// The root must be exactly a Git worktree root. Required for repository- and remote-scoped
    /// mutations that take no paths.
    ExactWorktreeRoot,
    /// The root only has to resolve. Correct for read-only reporting and for path-confined mutations.
    ResolvedPath,
}

/// Resolve the configured repository root under one action's policy.
///
/// The strict branch delegates to `core`, so there is exactly one implementation of the worktree-root
/// check rather than a second copy grown here.
pub(crate) fn repo_root_for_policy(
    repo_root: &Path,
    tool_name: &str,
    policy: WorktreeRootPolicy,
) -> Result<PathBuf, String> {
    match policy {
        WorktreeRootPolicy::ResolvedPath => resolved_repo_root(repo_root, tool_name),
        WorktreeRootPolicy::ExactWorktreeRoot => {
            contextpatch_core::git::exact_worktree_root(repo_root).map_err(|error| {
                format!(
                    "{tool_name} refused: this action mutates repository or remote state without \
                     naming any paths, so it requires the configured repo root to be exactly a Git \
                     worktree root; {error}"
                )
            })
        }
    }
}

/// Resolve the configured repository root, asserting nothing about worktrees.
///
/// The resolution itself lives in `core`; this adds only the prefix that addresses a failure to a tool,
/// which is why the MCP surface keeps its own shorter wording. Callers whose blast radius is bounded by
/// something other than the root use this; the Git actions that are bounded only by the root go through
/// [`repo_root_for_policy`] with [`WorktreeRootPolicy::ExactWorktreeRoot`].
pub(crate) fn resolved_repo_root(repo_root: &Path, tool_name: &str) -> Result<PathBuf, String> {
    contextpatch_core::git::resolve_repo_root(repo_root)
        .map_err(|error| format!("{tool_name} refused: failed to resolve repo root: {error}"))
}

pub(crate) fn git_status_short<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<String, String> {
    git_state::status_short(repository).map_err(|error| refused("git_status_short", error))
}

pub(crate) fn git_stdout_for_tool<'a>(
    tool_name: &str,
    repository: impl Into<GitRepository<'a>>,
    args: &[&str],
) -> Result<String, String> {
    git_state::stdout(repository, args).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_head_for_tool<'a>(
    tool_name: &str,
    repository: impl Into<GitRepository<'a>>,
) -> Result<String, String> {
    git_state::head(repository).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_success_for_tool<'a>(
    tool_name: &str,
    repository: impl Into<GitRepository<'a>>,
    args: Vec<String>,
) -> Result<(), String> {
    git_state::success(repository, &args).map_err(|error| refused(tool_name, error))
}

pub(crate) fn rev_count_for_tool<'a>(
    tool_name: &str,
    repository: impl Into<GitRepository<'a>>,
    range: &str,
) -> Result<u64, String> {
    git_state::rev_count(repository, range).map_err(|error| refused(tool_name, error))
}

pub(crate) fn ensure_remote_exists<'a>(
    tool_name: &str,
    repository: impl Into<GitRepository<'a>>,
    remote: &str,
) -> Result<(), String> {
    git_state::ensure_remote_exists(repository, remote).map_err(|error| refused(tool_name, error))
}

pub(crate) fn validate_git_remote(remote: &str) -> Result<String, String> {
    validate_git_ref_component("remote", remote)?;
    Ok(remote.to_string())
}

pub(crate) fn current_branch<'a>(
    tool_name: &str,
    repository: impl Into<GitRepository<'a>>,
) -> Result<String, String> {
    git_state::current_branch(repository).map_err(|error| refused(tool_name, error))
}

pub(crate) fn local_branch_exists<'a>(
    repository: impl Into<GitRepository<'a>>,
    branch: &str,
) -> Result<bool, String> {
    git_state::local_branch_exists(repository, branch)
        .map_err(|error| refused("git workflow", error))
}

pub(crate) fn validate_git_branch(branch: &str) -> Result<String, String> {
    validate_git_ref_component("branch", branch)?;
    if branch.contains("..")
        || branch.contains("@{")
        || branch.starts_with('-')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.ends_with(".lock")
    {
        return Err(format!("git workflow refused: invalid branch `{branch}`"));
    }
    let status = git_state::run(
        Path::new("."),
        &["check-ref-format", "--branch", branch],
    )
    .map_err(|error| refused("git workflow", error))?;
    if !status.success() {
        return Err(format!("git workflow refused: invalid branch `{branch}`"));
    }
    Ok(branch.to_string())
}

pub(crate) fn validate_git_ref_expression(value: &str) -> Result<String, String> {
    if value == "HEAD" {
        return Ok(value.to_string());
    }
    if (7..=40).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(value.to_ascii_lowercase());
    }

    validate_git_ref_component("ref", value)?;
    if value.contains("..")
        || value.contains('@')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with(".lock")
    {
        return Err(format!("git workflow refused: invalid ref `{value}`"));
    }

    let status = git_state::run(
        Path::new("."),
        &["check-ref-format", "--allow-onelevel", value],
    )
    .map_err(|error| refused("git workflow", error))?;
    if !status.success() {
        return Err(format!("git workflow refused: invalid ref `{value}`"));
    }
    Ok(value.to_string())
}

pub(crate) fn validate_git_ref_component(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.contains('\0')
        || value.chars().any(char::is_whitespace)
        || value.starts_with('-')
        || value.contains('\\')
        || value.contains(':')
        || value.contains('^')
        || value.contains('~')
        || value.contains('?')
        || value.contains('*')
        || value.contains('[')
    {
        return Err(format!("git workflow refused: invalid {label} `{value}`"));
    }
    Ok(())
}

pub(crate) fn resolve_commit<'a>(
    tool_name: &str,
    repository: impl Into<GitRepository<'a>>,
    ref_name: &str,
) -> Result<String, String> {
    git_state::resolve_commit(repository, ref_name).map_err(|error| refused(tool_name, error))
}

pub(crate) fn format_set(paths: &BTreeSet<String>) -> String {
    contextpatch_core::git::restore::format_path_set(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moved_validation_keeps_its_refusal_strings_addressed_to_the_calling_tool() {
        // The validation policy now lives in core, which returns the detail only. These assertions pin
        // the strings a caller actually sees, so a detail reworded in core cannot silently change the
        // public refusal text.
        let root = std::env::temp_dir();

        assert_eq!(
            normalize_git_path("git_commit_scoped", &root, "../escape.txt").unwrap_err(),
            "git_commit_scoped refused: path `../escape.txt` must be a normalized relative path"
        );
        assert_eq!(
            normalize_git_path("git_stage_exact", &root, "src/*.rs").unwrap_err(),
            "git_stage_exact refused: path `src/*.rs` contains Git pathspec metacharacters"
        );
        assert_eq!(
            normalize_git_path("delete_untracked_exact", &root, "").unwrap_err(),
            "delete_untracked_exact refused: path must not be empty or contain NUL"
        );
        assert_eq!(
            normalize_git_path("git_restore_exact", &root, "/etc/hosts").unwrap_err(),
            "git_restore_exact refused: path `/etc/hosts` must be repository-relative"
        );
        assert_eq!(
            normalize_git_prefixes("delete_generated_prefix", &root, &["/".to_string()])
                .unwrap_err(),
            "delete_generated_prefix refused: prefixes must not be empty"
        );

        // Commit message refusals stay addressed to git_commit_exact even when another commit tool
        // validated the message, because that is the string callers have always received.
        assert_eq!(
            validate_commit_subject("   ").unwrap_err(),
            "git_commit_exact refused: subject must not be empty"
        );
        assert_eq!(
            validate_commit_subject("first\nsecond").unwrap_err(),
            "git_commit_exact refused: subject must be a single line without NUL bytes"
        );
        assert_eq!(
            validate_commit_subject(&"a".repeat(201)).unwrap_err(),
            "git_commit_exact refused: subject must be at most 200 bytes"
        );
        assert_eq!(
            validate_commit_body("has\0nul").unwrap_err(),
            "git_commit_exact refused: body must not contain NUL bytes"
        );
        assert_eq!(
            validate_commit_body(&"a".repeat(20_001)).unwrap_err(),
            "git_commit_exact refused: body must be at most 20000 bytes"
        );

        // Accepted values still round-trip unchanged.
        assert_eq!(validate_commit_subject("  Fix  ").unwrap(), "Fix");
        assert_eq!(validate_commit_body("  why  ").unwrap(), "why");
    }

    #[test]
    fn one_resolver_renders_two_exact_wordings() {
        // Consolidating the canonicalization was only behavior-neutral if neither surface's text moved, so
        // both renderings are pinned exactly here, side by side. The I/O tail is derived from the same
        // failure rather than written out, which keeps the assertion exact without depending on the
        // platform's error string.
        use std::time::{SystemTime, UNIX_EPOCH};

        let absent = std::env::temp_dir().join(format!(
            "contextpatch-c11-absent-root-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let io_error = std::fs::canonicalize(&absent).unwrap_err();

        // The MCP adapter says `repo root` and omits the path.
        assert_eq!(
            resolved_repo_root(&absent, "file_info").unwrap_err(),
            format!("file_info refused: failed to resolve repo root: {io_error}")
        );
        assert_eq!(
            resolved_repo_root(&absent, "git_restore_exact").unwrap_err(),
            format!("git_restore_exact refused: failed to resolve repo root: {io_error}")
        );

        // `core` says `repository root` and names the path. Same resolver underneath.
        assert_eq!(
            contextpatch_core::git::exact_worktree_root(&absent)
                .unwrap_err()
                .to_string(),
            format!(
                "failed to resolve repository root {}: {io_error}",
                absent.display()
            )
        );
    }

    #[test]
    fn moved_state_guards_keep_their_refusal_prefixes() {
        // The state and query guards now live in core, which returns the detail only. These assertions
        // pin the prefixes the adapters assemble, including the two that were never tool names and are
        // therefore the easiest to lose in a move.
        let outside_any_repository = std::env::temp_dir();

        let error = git_status_short(&outside_any_repository).unwrap_err();
        assert!(
            error.starts_with("git_status_short refused: git status --short --untracked-files=all failed:"),
            "{error}"
        );

        let error = git_status_paths_for_tool("git_commit_exact", &outside_any_repository)
            .unwrap_err();
        assert!(error.starts_with("git_commit_exact refused: git status"), "{error}");

        let error = current_branch("git_push_exact", &outside_any_repository).unwrap_err();
        assert!(error.starts_with("git_push_exact refused: git branch"), "{error}");

        // This one is addressed to `git workflow`, not to any tool, which is the string callers have
        // always received.
        let error = local_branch_exists(&outside_any_repository, "main").unwrap_err();
        assert!(
            error.starts_with("git workflow refused: failed to check local branch `main` (exit code "),
            "{error}"
        );
    }

    #[test]
    fn the_consolidated_resolver_keeps_the_server_wording() {
        // Resolution now happens once in core, which returns the raw I/O error. The MCP surface has always
        // reported a shorter phrase than core does, including the underlying error, so this pins it.
        let absent = std::env::temp_dir().join("contextpatch-absent-root-for-resolver-regression");

        let error = resolved_repo_root(&absent, "file_info").unwrap_err();
        assert!(
            error.starts_with("file_info refused: failed to resolve repo root: "),
            "{error}"
        );
        // Not core's wording, which names the path instead.
        assert!(!error.contains("failed to resolve repository root"), "{error}");

        // The policy wrapper routes through the same resolver and keeps the same wording.
        let policy_error = repo_root_for_policy(&absent, "list_directory", WorktreeRootPolicy::ResolvedPath)
            .unwrap_err();
        assert!(
            policy_error.starts_with("list_directory refused: failed to resolve repo root: "),
            "{policy_error}"
        );
    }
}
