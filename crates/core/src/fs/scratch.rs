//! A writable working area outside the repository, addressable from an argv token.
//!
//! Guarded commands are confined to the repository root, which is right for reads and edits and wrong
//! for byproducts. A caller that needs to hand a subprocess somewhere to write, a temporary output
//! directory, a scratch copy, a captured artifact, currently has two options: put it in the
//! repository and remember to delete it, or be refused. Both are bad. Writing byproducts into the
//! tree pollutes `git status`, risks landing in a commit, and in a shared worktree invites another
//! process to sweep it; being refused means the work does not happen.
//!
//! So the server owns one directory per repository, outside every repository, and lets a caller name
//! it as `{scratch}` inside an argument. The token expands to an absolute path that the path policy
//! then accepts, and nothing else outside the repository becomes reachable: expansion is textual and
//! the result is re-checked to be inside the scratch root, so `{scratch}/../../etc` is refused rather
//! than resolved.
//!
//! The location is derived from a digest of the canonical repository path, so it is stable across
//! restarts (a caller can hand the same path to two calls and expect continuity) and distinct per
//! repository (five server instances do not collide).

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::ContextPatchError;

/// The token a caller writes inside an argument to mean "the scratch directory for this repository".
pub const SCRATCH_TOKEN: &str = "{scratch}";

/// Where scratch directories live, independent of any repository.
#[cfg(not(target_os = "windows"))]
fn container() -> PathBuf {
    // `HOME` is the right home for state that should survive a reboot. The temp directory is the
    // fallback rather than the default precisely because a caller may reasonably expect continuity
    // between two calls, and temp directories are periodically reaped.
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => PathBuf::from(home).join(".contextpatch/scratch"),
        _ => std::env::temp_dir().join(".contextpatch/scratch"),
    }
}

#[cfg(target_os = "windows")]
fn container() -> PathBuf {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty())
    {
        return PathBuf::from(local_app_data)
            .join("contextpatch")
            .join("scratch");
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return PathBuf::from(profile).join(".contextpatch").join("scratch");
    }
    std::env::temp_dir().join("contextpatch").join("scratch")
}

/// A stable, repository-specific scratch path. Does not touch the filesystem.
///
/// Keyed by repository identity rather than by path spelling, so a rename keeps its scratch directory and
/// two names for one directory never split into two. The label half of the name carries the repository leaf
/// so a human reading a path or an error can tell at a glance which repository it belongs to; only the
/// digest distinguishes repositories.
pub fn scratch_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
) -> Result<PathBuf, ContextPatchError> {
    let identity = crate::fs::repository_identity::identity_of(repo_root.into())?;
    Ok(container().join(identity.directory_name()))
}

/// The scratch path a path-keyed build would have produced.
///
/// Kept so receipts written before identity keying stay readable. Never used for new writes.
pub fn legacy_scratch_root<'a>(repo_root: impl Into<crate::git::RepositoryRoot<'a>>) -> PathBuf {
    let identity = crate::fs::repository_identity::legacy_path_identity(repo_root.into());
    container().join(identity.directory_name())
}

/// The scratch directory for this repository, created if absent.
pub fn ensure_scratch_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
) -> Result<PathBuf, ContextPatchError> {
    let root = scratch_root(repo_root)?;
    fs::create_dir_all(&root).map_err(|error| {
        ContextPatchError::invalid(format!(
            "could not create the scratch directory `{}`: {error}",
            root.display()
        ))
    })?;
    Ok(root)
}

/// True when any argument refers to the scratch directory.
pub fn references_scratch(args: &[String]) -> bool {
    args.iter().any(|arg| arg.contains(SCRATCH_TOKEN))
}

/// Reject a path that climbs out of the scratch root.
///
/// Checked on the lexical form rather than by canonicalizing, because the path usually does not exist
/// yet: a caller naming `{scratch}/output` expects the subprocess to create it. Canonicalization
/// would fail on the not-yet-created leaf and tell us nothing about intent, whereas a `..` component
/// is unambiguous regardless of what exists.
fn rejects_traversal(relative: &str) -> Result<(), ContextPatchError> {
    // The token is a directory prefix, so a caller naturally writes `{scratch}/output` and the suffix
    // arrives with a leading separator. That separator belongs to the join, not to the caller's path,
    // and treating the suffix as absolute would refuse the ordinary form while permitting nothing
    // extra. Traversal is still caught below, which is the property that actually matters.
    if relative
        .split(['/', '\\'])
        .any(|component| component == "..")
    {
        return Err(ContextPatchError::invalid(format!(
            "`{SCRATCH_TOKEN}{relative}` climbs outside the scratch directory"
        )));
    }
    let candidate = Path::new(relative.trim_start_matches(['/', '\\']));
    for component in candidate.components() {
        match component {
            Component::ParentDir => {
                return Err(ContextPatchError::invalid(format!(
                    "`{SCRATCH_TOKEN}{relative}` climbs outside the scratch directory"
                )));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(ContextPatchError::invalid(format!(
                    "`{SCRATCH_TOKEN}{relative}` must stay inside the scratch directory"
                )));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

fn validate_occurrences(arg: &str) -> Result<(), ContextPatchError> {
    let mut search_from = 0;
    while let Some(relative_position) = arg[search_from..].find(SCRATCH_TOKEN) {
        let position = search_from + relative_position;
        let tail_start = position + SCRATCH_TOKEN.len();
        let tail = &arg[tail_start..];
        if tail
            .chars()
            .next()
            .is_some_and(|character| !matches!(character, '/' | '\\'))
        {
            return Err(ContextPatchError::invalid(format!(
                "`{SCRATCH_TOKEN}` must be followed by a path separator or the end of its argument"
            )));
        }
        let next_token = tail.find(SCRATCH_TOKEN).unwrap_or(tail.len());
        rejects_traversal(&tail[..next_token])?;
        search_from = tail_start + next_token;
    }
    Ok(())
}

/// Replace every `{scratch}` occurrence with the absolute scratch path.
///
/// The scratch root is created as a side effect, because an argument naming it is a statement that
/// the subprocess is about to write there, and a missing parent directory is the most common way that
/// fails. Substitution is whole-token and textual, so an argument may embed the token mid-string
/// (`--output-dir={scratch}/run-1`) as callers naturally write it.
pub fn expand_scratch_tokens(
    repo_root: &Path,
    args: &[String],
) -> Result<Vec<String>, ContextPatchError> {
    if !references_scratch(args) {
        return Ok(args.to_vec());
    }
    let root = ensure_scratch_root(repo_root)?;
    let rendered = root.display().to_string();

    args.iter()
        .map(|arg| {
            if !arg.contains(SCRATCH_TOKEN) {
                return Ok(arg.clone());
            }
            validate_occurrences(arg)?;
            Ok(arg.replace(SCRATCH_TOKEN, &rendered))
        })
        .collect()
}

/// True when `path` is the scratch root or lies beneath it.
///
/// Used by the path policy to admit an expanded scratch path while continuing to refuse every other
/// location outside the repository.
pub fn is_within_scratch<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    path: &Path,
) -> bool {
    // An unidentifiable repository admits nothing, which is the safe direction: the caller's path stays
    // subject to the ordinary repository-confinement rules.
    let Ok(root) = scratch_root(repo_root) else {
        return false;
    };
    path == root || path.starts_with(&root)
}

/// Remove everything inside the scratch directory, leaving the directory itself.
///
/// Offered so a caller can reclaim space deliberately. Never called implicitly: a caller that writes
/// a scratch file in one call and reads it in the next would be broken by automatic cleanup, and
/// surprising deletion is worse than accumulated bytes.
pub fn purge_scratch<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
) -> Result<usize, ContextPatchError> {
    let root = scratch_root(repo_root)?;
    if !root.exists() {
        return Ok(0);
    }
    let entries = fs::read_dir(&root).map_err(|error| {
        ContextPatchError::invalid(format!(
            "could not read the scratch directory `{}`: {error}",
            root.display()
        ))
    })?;

    let mut removed = 0usize;
    for entry in entries {
        let entry = entry.map_err(|error| {
            ContextPatchError::invalid(format!("could not enumerate scratch entries: {error}"))
        })?;
        let path = entry.path();
        let outcome = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        outcome.map_err(|error| {
            ContextPatchError::invalid(format!(
                "could not remove scratch entry `{}`: {error}",
                path.display()
            ))
        })?;
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn scratch_root_is_outside_the_repository_and_stable() {
        let root = repo();
        let first = scratch_root(&root).unwrap();
        let second = scratch_root(&root).unwrap();
        assert_eq!(
            first, second,
            "the scratch path must be stable across calls"
        );
        assert!(
            !first.starts_with(&root),
            "the scratch path must not live inside the repository"
        );
    }

    #[test]
    fn distinct_repositories_get_distinct_scratch_paths() {
        let first = scratch_root(Path::new("/tmp/alpha-repository"));
        let second = scratch_root(Path::new("/tmp/beta-repository"));
        assert_ne!(first, second);
    }

    #[test]
    fn expansion_is_a_no_op_without_the_token() {
        let args = vec!["--files".to_string(), "task".to_string()];
        assert_eq!(expand_scratch_tokens(&repo(), &args).unwrap(), args);
    }

    #[test]
    fn expansion_substitutes_inside_a_larger_argument() {
        let args = vec![format!("--output-dir={SCRATCH_TOKEN}/run-1")];
        let expanded = expand_scratch_tokens(&repo(), &args).unwrap();
        assert!(expanded[0].starts_with("--output-dir="));
        assert!(expanded[0].ends_with("/run-1"));
        assert!(!expanded[0].contains(SCRATCH_TOKEN));
    }

    #[test]
    fn traversal_out_of_scratch_is_refused() {
        let args = vec![format!("{SCRATCH_TOKEN}/../../etc/passwd")];
        let error = expand_scratch_tokens(&repo(), &args).unwrap_err();
        assert!(
            error.to_string().contains("climbs outside"),
            "unexpected refusal: {error}"
        );
    }

    #[test]
    fn ordinary_child_path_after_the_token_is_allowed() {
        let args = vec![format!("{SCRATCH_TOKEN}/tmp")];
        assert!(expand_scratch_tokens(&repo(), &args).is_ok());
    }

    #[test]
    fn sibling_prefix_after_the_token_is_refused() {
        let args = vec![format!("{SCRATCH_TOKEN}evil/output")];
        let error = expand_scratch_tokens(&repo(), &args).unwrap_err();
        assert!(error.to_string().contains("path separator"));
    }

    #[test]
    fn every_token_occurrence_is_validated() {
        let args = vec![format!(
            "--inputs={SCRATCH_TOKEN}/safe,{SCRATCH_TOKEN}/../../etc"
        )];
        let error = expand_scratch_tokens(&repo(), &args).unwrap_err();
        assert!(error.to_string().contains("climbs outside"));
    }

    #[test]
    fn membership_recognises_expanded_paths() {
        let root = repo();
        let scratch = scratch_root(&root).unwrap();
        assert!(is_within_scratch(&root, &scratch));
        assert!(is_within_scratch(&root, &scratch.join("run-1/output")));
        assert!(!is_within_scratch(&root, Path::new("/etc/passwd")));
    }
}
