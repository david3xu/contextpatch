use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::tools;
use contextpatch_core::git::state as git_state;
use contextpatch_core::process::runner::BoundedProcessOutput;
pub(crate) fn validate_commit_subject(subject: &str) -> Result<String, String> {
    contextpatch_core::git::validate::commit_subject(subject)
        .map_err(|error| format!("{} refused: {error}", tools::git_commit_exact::NAME))
}

pub(crate) fn validate_commit_body(body: &str) -> Result<String, String> {
    contextpatch_core::git::validate::commit_body(body)
        .map_err(|error| format!("{} refused: {error}", tools::git_commit_exact::NAME))
}

pub(crate) fn normalize_git_paths(
    tool_name: &str,
    root: &Path,
    paths: &[String],
) -> Result<Vec<String>, String> {
    contextpatch_core::git::validate::git_paths(root, paths)
        .map_err(|error| format!("{tool_name} refused: {error}"))
}

pub(crate) fn normalize_git_prefixes(
    tool_name: &str,
    root: &Path,
    prefixes: &[String],
) -> Result<Vec<String>, String> {
    contextpatch_core::git::validate::git_prefixes(root, prefixes)
        .map_err(|error| format!("{tool_name} refused: {error}"))
}

pub(crate) fn paths_under_prefixes(
    paths: &BTreeSet<String>,
    prefixes: &[String],
) -> BTreeSet<String> {
    contextpatch_core::git::validate::paths_under_prefixes(paths, prefixes)
}

pub(crate) fn normalize_git_path(
    tool_name: &str,
    root: &Path,
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

pub(crate) fn git_status_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    git_status_paths_for_tool(tools::git_commit_exact::NAME, root)
}

pub(crate) fn git_status_paths_for_tool(
    tool_name: &str,
    root: &Path,
) -> Result<BTreeSet<String>, String> {
    git_state::status_paths(root).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_untracked_paths_for_tool(
    tool_name: &str,
    root: &Path,
) -> Result<BTreeSet<String>, String> {
    git_state::untracked_paths(root).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_untracked_and_ignored_paths_for_tool(
    tool_name: &str,
    root: &Path,
) -> Result<BTreeSet<String>, String> {
    git_state::untracked_and_ignored_paths(root).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_cached_paths(root: &Path) -> Result<BTreeSet<String>, String> {
    git_cached_paths_for_tool(tools::git_commit_exact::NAME, root)
}

pub(crate) fn git_cached_paths_for_tool(
    tool_name: &str,
    root: &Path,
) -> Result<BTreeSet<String>, String> {
    git_state::cached_paths(root).map_err(|error| refused(tool_name, error))
}

pub(crate) fn parse_nul_paths_for_tool(
    tool_name: &str,
    bytes: &[u8],
    label: &str,
) -> Result<BTreeSet<String>, String> {
    git_state::parse_nul_paths(bytes, label).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_args(subcommand: &str, paths: &[String]) -> Vec<String> {
    std::iter::once(subcommand.to_string())
        .chain(std::iter::once("--".to_string()))
        .chain(paths.iter().cloned())
        .collect()
}

pub(crate) fn git_restore_args(paths: &[String]) -> Vec<String> {
    ["restore", "--staged", "--worktree", "--"]
        .into_iter()
        .map(ToString::to_string)
        .chain(paths.iter().cloned())
        .collect()
}

pub(crate) fn ensure_restore_paths_are_tracked(
    root: &Path,
    paths: &[String],
) -> Result<(), String> {
    let untracked = paths
        .iter()
        .filter_map(|path| {
            let args = ["ls-files", "--error-unmatch", "--", path.as_str()];
            match git_output_for_tool(tools::git_restore_exact::NAME, root, &args) {
                Ok(_) => None,
                Err(_) => Some(path.clone()),
            }
        })
        .collect::<BTreeSet<_>>();

    if !untracked.is_empty() {
        return Err(format!(
            "git_restore_exact refused: untracked paths cannot be restored from HEAD\n{}",
            format_set(&untracked)
        ));
    }
    Ok(())
}

pub(crate) fn git_stdout(root: &Path, args: &[&str]) -> Result<String, String> {
    git_stdout_for_tool(tools::git_commit_exact::NAME, root, args)
}

pub(crate) fn git_success(root: &Path, args: Vec<String>) -> Result<(), String> {
    git_success_for_tool(tools::git_commit_exact::NAME, root, args)
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
        WorktreeRootPolicy::ResolvedPath => canonical_repo_root(repo_root, tool_name),
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

pub(crate) fn canonical_repo_root(repo_root: &Path, tool_name: &str) -> Result<PathBuf, String> {
    repo_root
        .canonicalize()
        .map_err(|error| format!("{tool_name} refused: failed to resolve repo root: {error}"))
}

pub(crate) fn git_status_short(root: &Path) -> Result<String, String> {
    git_state::status_short(root).map_err(|error| refused("git_status_short", error))
}

pub(crate) fn git_stdout_for_tool(
    tool_name: &str,
    root: &Path,
    args: &[&str],
) -> Result<String, String> {
    git_state::stdout(root, args).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_head_for_tool(tool_name: &str, root: &Path) -> Result<String, String> {
    git_state::head(root).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_success_for_tool(
    tool_name: &str,
    root: &Path,
    args: Vec<String>,
) -> Result<(), String> {
    git_state::success(root, &args).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_output_for_tool(
    tool_name: &str,
    root: &Path,
    args: &[&str],
) -> Result<BoundedProcessOutput, String> {
    git_state::output(root, args).map_err(|error| refused(tool_name, error))
}

pub(crate) fn git_status_code(root: &Path, args: &[&str]) -> Result<i32, String> {
    git_state::exit_code(root, args).map_err(|error| refused("git operation", error))
}

pub(crate) fn rev_count_for_tool(tool_name: &str, root: &Path, range: &str) -> Result<u64, String> {
    git_state::rev_count(root, range).map_err(|error| refused(tool_name, error))
}

pub(crate) fn ensure_remote_exists(
    tool_name: &str,
    root: &Path,
    remote: &str,
) -> Result<(), String> {
    git_state::ensure_remote_exists(root, remote).map_err(|error| refused(tool_name, error))
}

pub(crate) fn validate_git_remote(remote: &str) -> Result<String, String> {
    validate_git_ref_component("remote", remote)?;
    Ok(remote.to_string())
}

pub(crate) fn current_branch(tool_name: &str, root: &Path) -> Result<String, String> {
    git_state::current_branch(root).map_err(|error| refused(tool_name, error))
}

pub(crate) fn local_branch_exists(root: &Path, branch: &str) -> Result<bool, String> {
    git_state::local_branch_exists(root, branch).map_err(|error| refused("git workflow", error))
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

pub(crate) fn validate_expected_head(expected_head: &str) -> Result<String, String> {
    if expected_head.len() < 7 || expected_head.len() > 40 {
        return Err(
            "git_push_exact refused: expected_head must be a 7 to 40 character commit hash"
                .to_string(),
        );
    }
    if !expected_head.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err("git_push_exact refused: expected_head must be hexadecimal".to_string());
    }
    Ok(expected_head.to_ascii_lowercase())
}

pub(crate) fn remote_ref(remote: &str, branch: &str) -> String {
    format!("refs/remotes/{remote}/{branch}")
}

pub(crate) fn infer_fetch_branch(remote: &str, target_ref: &str) -> Result<String, String> {
    let refs_remotes_prefix = format!("refs/remotes/{remote}/");
    let remote_prefix = format!("{remote}/");
    let branch = if let Some(branch) = target_ref.strip_prefix(&refs_remotes_prefix) {
        branch
    } else if let Some(branch) = target_ref.strip_prefix(&remote_prefix) {
        branch
    } else {
        return Err(format!(
            "git_merge_readiness refused: fetch=true requires target_branch unless target_ref starts with `{remote}/` or `refs/remotes/{remote}/`"
        ));
    };
    validate_git_branch(branch)
}

pub(crate) fn resolve_commit(
    tool_name: &str,
    root: &Path,
    ref_name: &str,
) -> Result<String, String> {
    git_state::resolve_commit(root, ref_name).map_err(|error| refused(tool_name, error))
}

pub(crate) fn changed_files_between(
    tool_name: &str,
    root: &Path,
    from_ref: &str,
    to_ref: &str,
) -> Result<BTreeSet<String>, String> {
    let output = git_output_for_tool(
        tool_name,
        root,
        &["diff", "--name-only", "-z", from_ref, to_ref, "--"],
    )?;
    parse_nul_paths_for_tool(tool_name, &output.stdout, "git diff --name-only")
}

pub(crate) fn validate_required_file_path(
    tool_name: &str,
    root: &Path,
    raw: &str,
) -> Result<String, String> {
    validate_required_file_path_syntax(tool_name, raw)?;
    let path = Path::new(raw);
    let candidate = root.join(path);
    if !candidate.is_file() {
        return Err(format!(
            "{tool_name} refused: required file `{raw}` is missing after branch preparation"
        ));
    }
    let resolved = candidate.canonicalize().map_err(|error| {
        format!("{tool_name} refused: failed to resolve required file `{raw}`: {error}")
    })?;
    if !resolved.starts_with(root) {
        return Err(format!(
            "{tool_name} refused: required file `{raw}` resolves outside repository root"
        ));
    }
    Ok(raw.to_string())
}

pub(crate) fn validate_required_files_in_ref(
    tool_name: &str,
    root: &Path,
    ref_name: &str,
    paths: &[String],
) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|path| {
            validate_required_file_path_syntax(tool_name, path)?;
            let object = format!("{ref_name}:{path}");
            let object_type = git_stdout_for_tool(tool_name, root, &["cat-file", "-t", &object])
                .map_err(|_| {
                    format!(
                        "{tool_name} refused: required file `{path}` is missing from `{ref_name}`"
                    )
                })?;
            if object_type.trim() != "blob" {
                return Err(format!(
                    "{tool_name} refused: required path `{path}` in `{ref_name}` is not a file"
                ));
            }
            Ok(path.to_string())
        })
        .collect()
}

pub(crate) fn validate_required_file_path_syntax(tool_name: &str, raw: &str) -> Result<(), String> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(format!(
            "{tool_name} refused: required file path must not be empty or contain NUL"
        ));
    }
    if raw.contains(':') || raw.contains('*') || raw.contains('?') || raw.contains('[') {
        return Err(format!(
            "{tool_name} refused: required file path `{raw}` contains Git pathspec metacharacters"
        ));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!(
            "{tool_name} refused: required file path `{raw}` must be repository-relative"
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "{tool_name} refused: required file path `{raw}` must be a normalized relative path"
                ))
            }
        }
    }
    Ok(())
}

pub(crate) fn empty_label(text: &str) -> &str {
    if text.trim().is_empty() {
        "(empty)"
    } else {
        text
    }
}

pub(crate) fn format_set(paths: &BTreeSet<String>) -> String {
    if paths.is_empty() {
        return "(none)".to_string();
    }
    paths.iter().cloned().collect::<Vec<_>>().join("\n")
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

        let error = git_status_paths(&outside_any_repository).unwrap_err();
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
}
