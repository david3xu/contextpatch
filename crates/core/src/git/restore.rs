//! Typed planning and guarded execution for restore and delete workflows.
//!
//! These three actions undo work, which is why they are planned before they run and verified after. The
//! policy is the same in each case: establish that every named path is in the state the action requires,
//! compute exactly what would change, and then check afterwards that it actually changed. A postcondition
//! that fails is reported rather than assumed away, because "the restore ran" and "the file is clean" are
//! different claims.
//!
//! Planning and applying are separate so the adapter can hold its mutation lock and open its receipt
//! around the mutation alone. Nothing here journals, locks, or formats a response.
//!
//! Refusals carry the detail only. The adapter supplies the prefix, which is what keeps the public
//! refusal text identical while the policy lives here. Two of those prefixes are not the usual shape:
//! a failed postcondition is addressed as `refused after restore` or `refused after delete`, so the
//! offending set is returned as data and the adapter words it.

use std::collections::BTreeSet;

use crate::error::ContextPatchError;
use crate::fs::rooted;
use crate::git::repository::GitRepository;
use crate::git::root::RepositoryRoot;
use crate::git::state;

/// Most paths one restore may name.
pub const MAX_RESTORE_PATHS: usize = 100;

/// Most paths one untracked delete may name.
pub const MAX_UNTRACKED_DELETE_PATHS: usize = 100;

/// Most prefixes one generated delete may expand.
pub const MAX_GENERATED_PREFIXES: usize = 20;

/// Most paths one generated delete may expand to.
///
/// A prefix expansion that reaches thousands of paths is far more likely to be a mistaken prefix than an
/// intended cleanup, so the expansion is bounded rather than trusted.
pub const MAX_GENERATED_DELETE_PATHS: usize = 5000;

/// Label used when describing a tracked-file listing in a refusal.
pub const LS_FILES_LABEL: &str = "git ls-files";

/// One validated restore, resolved but not yet applied.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RestorePlan {
    /// Normalized paths in the order the caller submitted them.
    pub paths: Vec<String>,
    /// Every dirty path at plan time.
    pub dirty_paths: BTreeSet<String>,
    /// Dirty paths that would survive the restore.
    pub remaining_dirty_after: BTreeSet<String>,
}

/// What a restore actually did, including the postcondition result.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RestoreOutcome {
    /// Every dirty path after the restore.
    pub dirty_after: BTreeSet<String>,
    /// Requested paths that are still dirty. Non-empty means the restore did not take effect.
    pub still_dirty: BTreeSet<String>,
}

/// One validated untracked delete, resolved but not yet applied.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UntrackedDeletePlan {
    /// Normalized paths in the order the caller submitted them.
    pub paths: Vec<String>,
    /// Every untracked path at plan time.
    pub untracked_paths: BTreeSet<String>,
    /// Untracked paths that would survive the delete.
    pub remaining_untracked_after: BTreeSet<String>,
}

/// What an untracked delete actually did, including the postcondition result.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UntrackedDeleteOutcome {
    /// Every untracked path after the delete.
    pub untracked_after: BTreeSet<String>,
    /// Requested paths Git still reports as untracked. Non-empty means the delete did not take effect.
    pub still_untracked: BTreeSet<String>,
}

/// One validated generated-path delete, resolved but not yet applied.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct GeneratedDeletePlan {
    /// Normalized prefixes in the order the caller submitted them.
    pub prefixes: Vec<String>,
    /// Files to remove, in sorted order.
    pub files: Vec<String>,
    /// Directories to remove, in sorted order. Applied in reverse so children go before parents.
    pub directories: Vec<String>,
    /// Paths that matched a prefix directly. An ignored directory that matched may be removed whole,
    /// while a directory that merely became empty may not.
    matched: BTreeSet<String>,
}

/// Render a path set for a refusal, naming emptiness rather than printing nothing.
pub fn format_path_set(paths: &BTreeSet<String>) -> String {
    if paths.is_empty() {
        return "(none)".to_string();
    }
    paths.iter().cloned().collect::<Vec<_>>().join("\n")
}

/// The exact argv a restore runs.
///
/// Both the index and the worktree are restored, so a path staged *and* modified comes back clean in one
/// invocation rather than half-restored.
pub fn restore_argv(paths: &[String]) -> Vec<String> {
    ["restore", "--staged", "--worktree", "--"]
        .into_iter()
        .map(ToString::to_string)
        .chain(paths.iter().cloned())
        .collect()
}

fn ensure_no_duplicates(
    paths: &[String],
    unique: &BTreeSet<String>,
    noun: &str,
) -> Result<(), ContextPatchError> {
    if unique.len() == paths.len() {
        return Ok(());
    }
    Err(ContextPatchError::new(format!(
        "duplicate {noun} are not allowed"
    )))
}

/// Every path Git currently tracks.
pub fn tracked_paths<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = state::output(repository, &["ls-files", "-z"])?;
    state::parse_nul_paths(&output.stdout, LS_FILES_LABEL)
}

/// Require that every named path is tracked, so a restore has something to restore *from*.
///
/// An untracked path has no version in HEAD, so restoring it would either fail or silently do nothing.
pub fn ensure_paths_are_tracked<'a>(
    repository: impl Into<GitRepository<'a>>,
    paths: &[String],
) -> Result<(), ContextPatchError> {
    let repository = repository.into();
    let untracked = paths
        .iter()
        .filter_map(|path| {
            let args = ["ls-files", "--error-unmatch", "--", path.as_str()];
            match state::output(repository, &args) {
                Ok(_) => None,
                Err(_) => Some(path.clone()),
            }
        })
        .collect::<BTreeSet<_>>();

    if !untracked.is_empty() {
        return Err(ContextPatchError::new(format!(
            "untracked paths cannot be restored from HEAD\n{}",
            format_path_set(&untracked)
        )));
    }
    Ok(())
}

/// Plan a restore of exactly the named paths.
///
/// Requires every path to be dirty now, because restoring a clean path is a no-op the caller did not ask
/// for and cannot distinguish from success.
pub fn plan_restore_exact<'a>(
    repository: impl Into<GitRepository<'a>>,
    paths: &[String],
) -> Result<RestorePlan, ContextPatchError> {
    let repository = repository.into();
    let requested: BTreeSet<String> = paths.iter().cloned().collect();
    ensure_no_duplicates(paths, &requested, "paths")?;

    let dirty_paths = state::status_paths(repository)?;
    let missing_dirty = requested
        .difference(&dirty_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !missing_dirty.is_empty() {
        return Err(ContextPatchError::new(format!(
            "every requested path must currently be dirty\nnot_dirty_paths:\n{}",
            format_path_set(&missing_dirty)
        )));
    }
    ensure_paths_are_tracked(repository, paths)?;

    let remaining_dirty_after = dirty_paths
        .difference(&requested)
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(RestorePlan {
        paths: paths.to_vec(),
        dirty_paths,
        remaining_dirty_after,
    })
}

/// Run a planned restore and report whether it took effect.
///
/// The postcondition is returned rather than raised, because a failed postcondition is addressed to the
/// caller differently from a refusal that happened before anything changed.
pub fn apply_restore_exact<'a>(
    repository: impl Into<GitRepository<'a>>,
    plan: &RestorePlan,
) -> Result<RestoreOutcome, ContextPatchError> {
    let repository = repository.into();
    state::success(repository, &restore_argv(&plan.paths))?;

    let requested: BTreeSet<String> = plan.paths.iter().cloned().collect();
    let dirty_after = state::status_paths(repository)?;
    let still_dirty = requested
        .intersection(&dirty_after)
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(RestoreOutcome {
        dirty_after,
        still_dirty,
    })
}

/// Plan a delete of exactly the named untracked files.
///
/// Requires each path to be untracked *and* a regular file, so the action cannot remove tracked history
/// or walk into a directory the caller only named in passing.
pub fn plan_delete_untracked_exact<'a>(
    root: impl Into<RepositoryRoot<'a>>,
    paths: &[String],
) -> Result<UntrackedDeletePlan, ContextPatchError> {
    let root = root.into();
    let requested: BTreeSet<String> = paths.iter().cloned().collect();
    ensure_no_duplicates(paths, &requested, "paths")?;

    let untracked_paths = state::untracked_paths(root.git())?;
    let not_untracked = requested
        .difference(&untracked_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !not_untracked.is_empty() {
        return Err(ContextPatchError::new(format!(
            "every requested path must be an untracked file\nnot_untracked_paths:\n{}",
            format_path_set(&not_untracked)
        )));
    }
    for path in paths {
        if !rooted::is_regular_file(root, path)? {
            return Err(ContextPatchError::new(format!(
                "`{path}` is not a regular file"
            )));
        }
    }

    let remaining_untracked_after = untracked_paths
        .difference(&requested)
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(UntrackedDeletePlan {
        paths: paths.to_vec(),
        untracked_paths,
        remaining_untracked_after,
    })
}

/// Remove the planned untracked files.
///
/// Kept separate from planning and from verification so the adapter can wrap exactly this step in its
/// receipt.
pub fn delete_untracked_files<'a>(
    root: impl Into<RepositoryRoot<'a>>,
    plan: &UntrackedDeletePlan,
) -> Result<(), ContextPatchError> {
    let root = root.into();
    for path in &plan.paths {
        rooted::remove_file(root, path).map_err(|error| {
            ContextPatchError::new(format!("failed to delete `{path}`: {error}"))
        })?;
    }
    Ok(())
}

/// Check that a planned untracked delete took effect.
pub fn verify_untracked_deleted<'a>(
    root: impl Into<RepositoryRoot<'a>>,
    plan: &UntrackedDeletePlan,
) -> Result<UntrackedDeleteOutcome, ContextPatchError> {
    let root = root.into();
    let requested: BTreeSet<String> = plan.paths.iter().cloned().collect();
    let untracked_after = state::untracked_paths(root.git())?;
    let still_untracked = requested
        .intersection(&untracked_after)
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(UntrackedDeleteOutcome {
        untracked_after,
        still_untracked,
    })
}

/// Plan a delete of generated paths under the named prefixes.
///
/// Only untracked and ignored paths are candidates, and a tracked path anywhere under a matched prefix
/// refuses the whole plan. Empty directories under a prefix are collected too, so a cleanup does not
/// leave a skeleton behind.
pub fn plan_delete_generated_prefix<'a>(
    root: impl Into<RepositoryRoot<'a>>,
    prefixes: &[String],
) -> Result<GeneratedDeletePlan, ContextPatchError> {
    let root = root.into();
    let prefix_set: BTreeSet<String> = prefixes.iter().cloned().collect();
    ensure_no_duplicates(prefixes, &prefix_set, "prefixes")?;

    let tracked = tracked_paths(root.git())?;
    let candidates = state::untracked_and_ignored_paths(root.git())?;
    let matched = crate::git::validate::paths_under_prefixes(&candidates, prefixes);

    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    for path in &matched {
        if tracked.contains(path) {
            return Err(ContextPatchError::new(format!(
                "matched tracked path `{path}`"
            )));
        }
        if rooted::is_regular_file(root, path)? {
            files.insert(path.clone());
        } else if rooted::is_directory(root, path)? {
            directories.insert(path.clone());
            collect_untracked_files_in_dir(root, path, &tracked, &mut files)?;
        }
    }
    collect_empty_dirs_under_prefixes(root, prefixes, &mut directories)?;

    if files.len() + directories.len() > MAX_GENERATED_DELETE_PATHS {
        return Err(ContextPatchError::new(format!(
            "matched {} paths; maximum is {MAX_GENERATED_DELETE_PATHS}",
            files.len() + directories.len()
        )));
    }
    if files.is_empty() && directories.is_empty() {
        return Err(ContextPatchError::new(format!(
            "no generated files or empty directories matched prefixes\nprefixes:\n{}",
            format_path_set(&prefix_set)
        )));
    }

    Ok(GeneratedDeletePlan {
        prefixes: prefixes.to_vec(),
        files: files.iter().cloned().collect(),
        directories: directories.iter().cloned().collect(),
        matched,
    })
}

/// Remove the planned generated files and directories.
///
/// Directories are removed in reverse sorted order so a child is gone before its parent is considered.
/// A directory that merely became empty is removed only while empty; a directory that matched a prefix
/// outright may be removed whole, because the caller named it.
pub fn apply_delete_generated_prefix<'a>(
    root: impl Into<RepositoryRoot<'a>>,
    plan: &GeneratedDeletePlan,
) -> Result<(), ContextPatchError> {
    let root = root.into();
    for path in &plan.files {
        if rooted::exists(root, path)? {
            rooted::remove_file(root, path).map_err(|error| {
                ContextPatchError::new(format!("failed to delete `{path}`: {error}"))
            })?;
        }
    }
    for path in plan.directories.iter().rev() {
        let is_directory = rooted::is_directory(root, path)?;
        if is_directory && rooted::directory_is_empty(root, path)? {
            rooted::remove_dir(root, path).map_err(|error| {
                ContextPatchError::new(format!("failed to delete directory `{path}`: {error}"))
            })?;
        } else if is_directory && plan.matched.contains(path) {
            rooted::remove_dir_all(root, path).map_err(|error| {
                ContextPatchError::new(format!(
                    "failed to delete ignored directory `{path}`: {error}"
                ))
            })?;
        }
    }
    Ok(())
}

/// Collect every file under a matched ignored directory, refusing if tracked history is found.
///
/// The tracked-path refusal guards a race rather than a routine case: Git only collapses a directory to
/// itself in an ignored listing when the directory holds nothing tracked, so a tracked file found here
/// appeared between the listing and this walk. Keeping the check means that window ends in a refusal
/// rather than in a recursive delete of tracked history.
fn collect_untracked_files_in_dir(
    root: RepositoryRoot<'_>,
    path: &str,
    tracked: &BTreeSet<String>,
    files: &mut BTreeSet<String>,
) -> Result<(), ContextPatchError> {
    for entry in rooted::read_dir(root, path)? {
        if tracked.contains(&entry.relative) {
            return Err(ContextPatchError::new(format!(
                "ignored directory `{path}` contains tracked path `{}`",
                entry.relative
            )));
        }
        match entry.kind {
            rooted::RootedEntryKind::RegularFile => {
                files.insert(entry.relative);
            }
            rooted::RootedEntryKind::Directory => {
                collect_untracked_files_in_dir(root, &entry.relative, tracked, files)?;
            }
            _ => {
                return Err(ContextPatchError::new(format!(
                    "`{}` is not a regular file or directory",
                    entry.relative
                )))
            }
        }
    }
    Ok(())
}

fn collect_empty_dirs_under_prefixes(
    root: RepositoryRoot<'_>,
    prefixes: &[String],
    dirs: &mut BTreeSet<String>,
) -> Result<(), ContextPatchError> {
    for prefix in prefixes {
        if rooted::is_directory(root, prefix)? {
            collect_empty_dirs(root, prefix, dirs)?;
        }
    }
    Ok(())
}

fn collect_empty_dirs(
    root: RepositoryRoot<'_>,
    path: &str,
    dirs: &mut BTreeSet<String>,
) -> Result<bool, ContextPatchError> {
    let mut empty = true;
    for entry in rooted::read_dir(root, path)? {
        if entry.kind == rooted::RootedEntryKind::Directory {
            if !collect_empty_dirs(root, &entry.relative, dirs)? {
                empty = false;
            }
        } else {
            empty = false;
        }
    }
    if empty {
        dirs.insert(path.to_string());
    }
    Ok(empty)
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

    fn repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-git-restore-{name}-{}",
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
        fs::write(root.join("tracked.txt"), "alpha\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "initial"]);
        root
    }

    fn owned(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn names(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn the_restore_argv_covers_index_and_worktree() {
        // A path both staged and modified has to come back clean in one invocation.
        assert_eq!(
            restore_argv(&owned(&["a.txt"])),
            owned(&["restore", "--staged", "--worktree", "--", "a.txt"])
        );
    }

    #[test]
    fn an_empty_set_is_named_rather_than_printed_as_nothing() {
        assert_eq!(format_path_set(&BTreeSet::new()), "(none)");
        assert_eq!(format_path_set(&names(&["b", "a"])), "a\nb");
    }

    #[test]
    fn a_restore_plans_then_applies_and_verifies() {
        let root = repo("restore_roundtrip");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("other.txt"), "beta\n").unwrap();
        git(&root, &["add", "other.txt"]);
        git(&root, &["commit", "--quiet", "-m", "second"]);
        fs::write(root.join("other.txt"), "also changed\n").unwrap();

        let plan = plan_restore_exact(&root, &owned(&["tracked.txt"])).unwrap();
        assert_eq!(plan.remaining_dirty_after, names(&["other.txt"]));

        let outcome = apply_restore_exact(&root, &plan).unwrap();

        assert!(outcome.still_dirty.is_empty(), "{outcome:?}");
        assert_eq!(outcome.dirty_after, names(&["other.txt"]));
        assert_eq!(fs::read_to_string(root.join("tracked.txt")).unwrap(), "alpha\n");
    }

    #[test]
    fn a_clean_path_cannot_be_restored() {
        let root = repo("restore_clean");

        let error = plan_restore_exact(&root, &owned(&["tracked.txt"]))
            .unwrap_err()
            .to_string();

        assert!(error.starts_with("every requested path must currently be dirty"), "{error}");
        assert!(error.contains("not_dirty_paths:\ntracked.txt"), "{error}");
    }

    #[test]
    fn an_untracked_path_cannot_be_restored_from_head() {
        let root = repo("restore_untracked");
        fs::write(root.join("fresh.txt"), "beta\n").unwrap();

        // Dirty, but with no version in HEAD to come back to.
        let error = plan_restore_exact(&root, &owned(&["fresh.txt"]))
            .unwrap_err()
            .to_string();

        assert!(error.starts_with("untracked paths cannot be restored from HEAD"), "{error}");
    }

    #[test]
    fn duplicate_paths_are_refused_before_any_query() {
        let root = repo("restore_duplicates");

        assert_eq!(
            plan_restore_exact(&root, &owned(&["tracked.txt", "tracked.txt"]))
                .unwrap_err()
                .to_string(),
            "duplicate paths are not allowed"
        );
        assert_eq!(
            plan_delete_untracked_exact(&root, &owned(&["a.txt", "a.txt"]))
                .unwrap_err()
                .to_string(),
            "duplicate paths are not allowed"
        );
        assert_eq!(
            plan_delete_generated_prefix(&root, &owned(&["build", "build"]))
                .unwrap_err()
                .to_string(),
            "duplicate prefixes are not allowed"
        );
    }

    #[test]
    fn an_untracked_delete_plans_then_removes_and_verifies() {
        let root = repo("delete_untracked_roundtrip");
        fs::write(root.join("fresh.txt"), "beta\n").unwrap();
        fs::write(root.join("keep.txt"), "gamma\n").unwrap();

        let plan = plan_delete_untracked_exact(&root, &owned(&["fresh.txt"])).unwrap();
        assert_eq!(plan.remaining_untracked_after, names(&["keep.txt"]));

        delete_untracked_files(&root, &plan).unwrap();
        let outcome = verify_untracked_deleted(&root, &plan).unwrap();

        assert!(outcome.still_untracked.is_empty(), "{outcome:?}");
        assert!(!root.join("fresh.txt").exists());
        assert!(root.join("keep.txt").exists());
    }

    #[test]
    fn a_tracked_path_cannot_be_deleted_as_untracked() {
        let root = repo("delete_untracked_tracked");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();

        let error = plan_delete_untracked_exact(&root, &owned(&["tracked.txt"]))
            .unwrap_err()
            .to_string();

        assert!(error.starts_with("every requested path must be an untracked file"), "{error}");
        assert!(root.join("tracked.txt").exists());
    }

    #[test]
    fn a_directory_cannot_be_deleted_as_an_untracked_file() {
        let root = repo("delete_untracked_directory");
        fs::create_dir_all(root.join("scratch")).unwrap();
        fs::write(root.join("scratch").join("inner.txt"), "beta\n").unwrap();

        // Porcelain reports the file, not the directory, so naming the directory fails the untracked
        // check before the regular-file check is even reached.
        assert!(plan_delete_untracked_exact(&root, &owned(&["scratch"])).is_err());
        assert!(root.join("scratch").is_dir());
    }

    #[test]
    fn a_generated_delete_collects_files_and_empty_directories() {
        let root = repo("generated_roundtrip");
        fs::write(root.join(".gitignore"), "build/\n").unwrap();
        git(&root, &["add", ".gitignore"]);
        git(&root, &["commit", "--quiet", "-m", "ignore build"]);
        fs::create_dir_all(root.join("build").join("nested")).unwrap();
        fs::write(root.join("build").join("out.bin"), "binary").unwrap();
        fs::create_dir_all(root.join("build").join("hollow")).unwrap();

        let plan = plan_delete_generated_prefix(&root, &owned(&["build"])).unwrap();
        assert!(plan.files.contains(&"build/out.bin".to_string()), "{plan:?}");

        apply_delete_generated_prefix(&root, &plan).unwrap();

        assert!(!root.join("build").exists(), "the ignored directory was named, so it goes whole");
    }

    #[test]
    fn a_prefix_deletes_untracked_siblings_and_leaves_tracked_history() {
        let root = repo("generated_mixed_directory");
        fs::create_dir_all(root.join("mixed")).unwrap();
        fs::write(root.join("mixed").join("kept.txt"), "alpha\n").unwrap();
        git(&root, &["add", "mixed/kept.txt"]);
        git(&root, &["commit", "--quiet", "-m", "tracked under prefix"]);
        fs::write(root.join("mixed").join("generated.txt"), "beta\n").unwrap();

        // Porcelain reports the untracked *file*, not the enclosing directory, so the prefix matches only
        // the generated sibling and the tracked file is never a candidate.
        let plan = plan_delete_generated_prefix(&root, &owned(&["mixed"])).unwrap();
        assert_eq!(plan.files, owned(&["mixed/generated.txt"]));
        assert!(plan.directories.is_empty(), "{plan:?}");

        apply_delete_generated_prefix(&root, &plan).unwrap();

        assert!(root.join("mixed").join("kept.txt").exists());
        assert!(!root.join("mixed").join("generated.txt").exists());
    }

    #[test]
    fn an_ignored_directory_is_removed_whole_when_it_holds_no_tracked_history() {
        let root = repo("generated_ignored_directory");
        fs::write(root.join(".gitignore"), "mixed/\n").unwrap();
        git(&root, &["add", ".gitignore"]);
        git(&root, &["commit", "--quiet", "-m", "ignore mixed"]);
        fs::create_dir_all(root.join("mixed").join("deep")).unwrap();
        fs::write(root.join("mixed").join("deep").join("out.bin"), "binary").unwrap();

        // Git collapses an ignored directory to the directory itself, so the directory is the candidate
        // and its contents are enumerated by walking it.
        let plan = plan_delete_generated_prefix(&root, &owned(&["mixed"])).unwrap();
        assert!(plan.directories.contains(&"mixed".to_string()), "{plan:?}");
        assert!(
            plan.files.contains(&"mixed/deep/out.bin".to_string()),
            "{plan:?}"
        );

        apply_delete_generated_prefix(&root, &plan).unwrap();

        assert!(!root.join("mixed").exists());
        assert!(root.join(".gitignore").exists());
    }

    #[test]
    fn a_prefix_matching_nothing_is_refused_with_the_prefixes_listed() {
        let root = repo("generated_no_match");

        let error = plan_delete_generated_prefix(&root, &owned(&["absent"]))
            .unwrap_err()
            .to_string();

        assert!(
            error.starts_with("no generated files or empty directories matched prefixes"),
            "{error}"
        );
        assert!(error.contains("prefixes:\nabsent"), "{error}");
    }

    #[test]
    fn planning_never_touches_the_worktree() {
        let root = repo("planning_is_read_only");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("fresh.txt"), "beta\n").unwrap();

        let _ = plan_restore_exact(&root, &owned(&["tracked.txt"])).unwrap();
        let _ = plan_delete_untracked_exact(&root, &owned(&["fresh.txt"])).unwrap();

        // Every guard above ran, and nothing changed. Planning is separate from applying precisely so a
        // refusal cannot leave a partial mutation behind.
        assert_eq!(fs::read_to_string(root.join("tracked.txt")).unwrap(), "changed\n");
        assert_eq!(fs::read_to_string(root.join("fresh.txt")).unwrap(), "beta\n");
    }
}
