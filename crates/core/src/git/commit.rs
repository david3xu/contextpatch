//! Typed planning and guarded execution for the commit and staging workflows.
//!
//! Three commit shapes differ only in how they select paths, and each selection carries its own claim.
//! `exact` claims the caller knows the entire dirty set, so an inexact list is a stale view rather than a
//! smaller request. `scoped` and `prefix` claim only a subset, so they additionally require a clean index
//! to prove no unrelated staged work is swept in. All three verify after staging that the index holds
//! exactly what was planned, because `git add` on a path that changed underneath would otherwise commit
//! something the caller never saw.
//!
//! Planning, staging, committing, and verification are separate functions. That is not decomposition for
//! its own sake: the adapter opens a Git receipt around the mutation and each step's refusal is addressed
//! differently, as `refused`, `refused after staging`, or `refused after commit`. Splitting them lets the
//! adapter word each one exactly as it always has.
//!
//! Refusals carry the detail only; the adapter supplies the prefix.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::ContextPatchError;
use crate::git::restore::format_path_set;
use crate::git::state;
use crate::git::validate;

/// Most paths one commit may name.
pub const MAX_COMMIT_PATHS: usize = 100;

/// Most prefixes one commit may expand.
pub const MAX_COMMIT_PREFIXES: usize = 20;

/// Most dirty paths a prefix commit may expand to.
pub const MAX_EXPANDED_COMMIT_PATHS: usize = 5000;

/// Most paths one staging request may name.
pub const MAX_STAGE_PATHS: usize = 500;

/// Most paths a staged-scope check may list.
pub const MAX_SCOPE_CHECK_PATHS: usize = 500;

/// Most prefixes a staged-scope check may list.
pub const MAX_SCOPE_CHECK_PREFIXES: usize = 50;

/// The exact argv used to stage paths.
///
/// The `--` separator is not decoration: without it a path that looks like a revision would be read as
/// one.
pub fn stage_argv(paths: &[String]) -> Vec<String> {
    std::iter::once("add".to_string())
        .chain(std::iter::once("--".to_string()))
        .chain(paths.iter().cloned())
        .collect()
}

/// The exact argv used to commit staged work.
///
/// A body becomes a second `-m` rather than being folded into the subject, so the subject stays one line.
pub fn commit_argv(subject: &str, body: &str) -> Vec<String> {
    let mut argv = vec![
        "commit".to_string(),
        "--quiet".to_string(),
        "-m".to_string(),
        subject.to_string(),
    ];
    if !body.is_empty() {
        argv.push("-m".to_string());
        argv.push(body.to_string());
    }
    argv
}

fn ensure_no_duplicates(
    values: &[String],
    unique: &BTreeSet<String>,
    noun: &str,
) -> Result<(), ContextPatchError> {
    if unique.len() == values.len() {
        return Ok(());
    }
    Err(ContextPatchError::new(format!(
        "duplicate {noun} are not allowed"
    )))
}

fn dirty_or_refuse(root: &Path) -> Result<BTreeSet<String>, ContextPatchError> {
    let dirty_paths = state::status_paths(root)?;
    if dirty_paths.is_empty() {
        return Err(ContextPatchError::new("repository has no dirty paths"));
    }
    Ok(dirty_paths)
}

/// Require a clean index so a partial commit cannot sweep in unrelated staged work.
fn ensure_index_clean(root: &Path, reason: &str) -> Result<(), ContextPatchError> {
    let staged_before = state::cached_paths(root)?;
    if !staged_before.is_empty() {
        return Err(ContextPatchError::new(format!(
            "{reason}\nstaged_paths:\n{}",
            format_path_set(&staged_before)
        )));
    }
    Ok(())
}

/// One validated exact commit: the caller's list must be the whole dirty set.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExactCommitPlan {
    /// Normalized paths in submission order.
    pub paths: Vec<String>,
    /// The same paths as a set, which is also the full dirty set.
    pub expected_paths: BTreeSet<String>,
}

/// One validated partial commit, whether selected by path or by prefix.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PartialCommitPlan {
    /// Prefixes that selected the paths, empty for a scoped commit.
    pub prefixes: Vec<String>,
    /// Selected paths in the order they will be staged.
    pub paths: Vec<String>,
    /// The selected paths as a set.
    pub requested_paths: BTreeSet<String>,
    /// Dirty paths that will survive the commit.
    pub remaining_dirty_after: BTreeSet<String>,
}

/// One validated staging request.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StagePlan {
    /// Normalized paths in submission order.
    pub paths: Vec<String>,
    /// The same paths as a set.
    pub requested_paths: BTreeSet<String>,
}

/// The commit a mutation produced, plus the resulting short status.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CommitIdentity {
    pub commit: String,
    pub short_commit: String,
    pub status_short: String,
}

/// Result of checking the index against an allow list.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StagedScopeReport {
    pub staged_paths: BTreeSet<String>,
    pub disallowed_paths: BTreeSet<String>,
    pub missing_required_paths: BTreeSet<String>,
    pub passed: bool,
}

/// Plan a commit that must name the entire dirty set.
///
/// An inexact list means the caller is working from a stale view of the repository, so it is refused
/// rather than narrowed. The refusal reports all four sets so the caller can see which way it is stale.
pub fn plan_commit_exact(
    root: &Path,
    paths: &[String],
) -> Result<ExactCommitPlan, ContextPatchError> {
    let expected_paths: BTreeSet<String> = paths.iter().cloned().collect();
    ensure_no_duplicates(paths, &expected_paths, "paths")?;

    let dirty_paths = dirty_or_refuse(root)?;
    if dirty_paths != expected_paths {
        // The `expected_paths` and `actual_dirty_paths` labels are swapped relative to the sets they
        // print. That is preserved deliberately: this migration is behavior-neutral, and correcting the
        // labels would change public refusal text. It is recorded as a separate follow-up.
        return Err(ContextPatchError::new(format!(
            "provided paths must exactly match the full dirty-path set\nexpected_paths:\n{}\nactual_dirty_paths:\n{}\nmissing_from_input:\n{}\nunexpected_in_input:\n{}",
            format_path_set(&dirty_paths),
            format_path_set(&expected_paths),
            format_path_set(&dirty_paths.difference(&expected_paths).cloned().collect()),
            format_path_set(&expected_paths.difference(&dirty_paths).cloned().collect())
        )));
    }

    Ok(ExactCommitPlan {
        paths: paths.to_vec(),
        expected_paths,
    })
}

/// Plan a commit of a named subset of the dirty set.
pub fn plan_commit_scoped(
    root: &Path,
    paths: &[String],
) -> Result<PartialCommitPlan, ContextPatchError> {
    let requested_paths: BTreeSet<String> = paths.iter().cloned().collect();
    ensure_no_duplicates(paths, &requested_paths, "paths")?;

    let dirty_paths = dirty_or_refuse(root)?;
    let not_dirty = requested_paths
        .difference(&dirty_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !not_dirty.is_empty() {
        return Err(ContextPatchError::new(format!(
            "every requested path must currently be dirty\nnot_dirty_paths:\n{}",
            format_path_set(&not_dirty)
        )));
    }

    ensure_index_clean(
        root,
        "index must be clean before scoped commit so unrelated staged work is not included",
    )?;

    let remaining_dirty_after = dirty_paths
        .difference(&requested_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(PartialCommitPlan {
        prefixes: Vec::new(),
        paths: paths.to_vec(),
        requested_paths,
        remaining_dirty_after,
    })
}

/// Plan a commit of every dirty path under the named prefixes.
pub fn plan_commit_prefix(
    root: &Path,
    prefixes: &[String],
) -> Result<PartialCommitPlan, ContextPatchError> {
    let prefix_set: BTreeSet<String> = prefixes.iter().cloned().collect();
    ensure_no_duplicates(prefixes, &prefix_set, "prefixes")?;

    let dirty_paths = dirty_or_refuse(root)?;
    let requested_paths = validate::paths_under_prefixes(&dirty_paths, prefixes);
    if requested_paths.is_empty() {
        return Err(ContextPatchError::new(format!(
            "no dirty paths matched prefixes\nprefixes:\n{}",
            format_path_set(&prefix_set)
        )));
    }
    if requested_paths.len() > MAX_EXPANDED_COMMIT_PATHS {
        return Err(ContextPatchError::new(format!(
            "matched {} dirty paths; maximum is {MAX_EXPANDED_COMMIT_PATHS}",
            requested_paths.len()
        )));
    }

    ensure_index_clean(
        root,
        "index must be clean before prefix commit so unrelated staged work is not included",
    )?;

    let remaining_dirty_after = dirty_paths
        .difference(&requested_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(PartialCommitPlan {
        prefixes: prefixes.to_vec(),
        paths: requested_paths.iter().cloned().collect(),
        requested_paths,
        remaining_dirty_after,
    })
}

/// Plan staging a named subset without committing.
pub fn plan_stage_exact(root: &Path, paths: &[String]) -> Result<StagePlan, ContextPatchError> {
    let requested_paths: BTreeSet<String> = paths.iter().cloned().collect();
    ensure_no_duplicates(paths, &requested_paths, "paths")?;

    let dirty_paths = dirty_or_refuse(root)?;
    let not_dirty = requested_paths
        .difference(&dirty_paths)
        .cloned()
        .collect::<BTreeSet<_>>();
    if !not_dirty.is_empty() {
        return Err(ContextPatchError::new(format!(
            "every requested path must currently be dirty\nnot_dirty_paths:\n{}",
            format_path_set(&not_dirty)
        )));
    }

    ensure_index_clean(root, "index must be clean before staging exact paths")?;
    Ok(StagePlan {
        paths: paths.to_vec(),
        requested_paths,
    })
}

/// Stage the planned paths.
///
/// Separate from verification because a failure here is a plain refusal while a failed verification is
/// addressed as happening *after* staging.
pub fn stage_paths(root: &Path, paths: &[String]) -> Result<(), ContextPatchError> {
    state::success(root, &stage_argv(paths))
}

/// Verify the index holds exactly the exact-commit set, and that the dirty set has not moved.
///
/// Both checks matter: the first catches `git add` picking up something else, the second catches the tree
/// changing between planning and staging, which would commit content the caller never reviewed.
pub fn verify_exact_staged(
    root: &Path,
    plan: &ExactCommitPlan,
) -> Result<(), ContextPatchError> {
    let staged_paths = state::cached_paths(root)?;
    if staged_paths != plan.expected_paths {
        return Err(ContextPatchError::new(format!(
            "staged paths differ from requested exact set\nrequested:\n{}\nstaged:\n{}",
            format_path_set(&plan.expected_paths),
            format_path_set(&staged_paths)
        )));
    }
    let dirty_paths_after_stage = state::status_paths(root)?;
    if dirty_paths_after_stage != plan.expected_paths {
        return Err(ContextPatchError::new(format!(
            "dirty paths changed before commit\nrequested:\n{}\ncurrent_dirty_paths:\n{}",
            format_path_set(&plan.expected_paths),
            format_path_set(&dirty_paths_after_stage)
        )));
    }
    Ok(())
}

/// Verify the index holds exactly the scoped set.
pub fn verify_scoped_staged(
    root: &Path,
    plan: &PartialCommitPlan,
) -> Result<(), ContextPatchError> {
    let staged_paths = state::cached_paths(root)?;
    if staged_paths != plan.requested_paths {
        return Err(ContextPatchError::new(format!(
            "staged paths differ from requested scoped set\nrequested:\n{}\nstaged:\n{}",
            format_path_set(&plan.requested_paths),
            format_path_set(&staged_paths)
        )));
    }
    Ok(())
}

/// Verify the index holds exactly the prefix-expanded set.
pub fn verify_prefix_staged(
    root: &Path,
    plan: &PartialCommitPlan,
) -> Result<(), ContextPatchError> {
    let staged_paths = state::cached_paths(root)?;
    if staged_paths != plan.requested_paths {
        return Err(ContextPatchError::new(format!(
            "staged paths differ from expanded prefix set\nexpanded:\n{}\nstaged:\n{}",
            format_path_set(&plan.requested_paths),
            format_path_set(&staged_paths)
        )));
    }
    Ok(())
}

/// Verify the index holds exactly the staged set a staging request planned.
pub fn verify_stage_exact(root: &Path, plan: &StagePlan) -> Result<(), ContextPatchError> {
    let staged_paths = state::cached_paths(root)?;
    if staged_paths != plan.requested_paths {
        return Err(ContextPatchError::new(format!(
            "staged paths differ from requested exact set\nrequested:\n{}\nstaged:\n{}",
            format_path_set(&plan.requested_paths),
            format_path_set(&staged_paths)
        )));
    }
    Ok(())
}

/// Commit whatever is staged. Exactly one commit, never more.
pub fn commit_staged(root: &Path, subject: &str, body: &str) -> Result<(), ContextPatchError> {
    state::success(root, &commit_argv(subject, body))
}

/// Requested paths that are still dirty after a commit, which must be empty.
pub fn requested_still_dirty(
    root: &Path,
    requested: &BTreeSet<String>,
) -> Result<(BTreeSet<String>, BTreeSet<String>), ContextPatchError> {
    let dirty_after = state::status_paths(root)?;
    let still_dirty = requested
        .intersection(&dirty_after)
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok((dirty_after, still_dirty))
}

/// Read the commit a mutation produced.
pub fn read_commit_identity(root: &Path) -> Result<CommitIdentity, ContextPatchError> {
    Ok(CommitIdentity {
        commit: state::stdout(root, &["rev-parse", "HEAD"])?.trim().to_string(),
        short_commit: state::stdout(root, &["rev-parse", "--short", "HEAD"])?
            .trim()
            .to_string(),
        status_short: state::stdout(root, &["status", "--short"])?,
    })
}

/// Check the index against an allow list and a required list.
///
/// Read-only: it reports rather than refuses, because the caller is asking a question about scope, not
/// requesting a change.
pub fn evaluate_staged_scope(
    root: &Path,
    allowed_paths: &[String],
    allowed_prefixes: &[String],
    required_paths: &[String],
) -> Result<StagedScopeReport, ContextPatchError> {
    let allowed_path_set: BTreeSet<String> = allowed_paths.iter().cloned().collect();
    ensure_no_duplicates(allowed_paths, &allowed_path_set, "allowed_paths")?;
    let allowed_prefix_set: BTreeSet<String> = allowed_prefixes.iter().cloned().collect();
    ensure_no_duplicates(allowed_prefixes, &allowed_prefix_set, "allowed_prefixes")?;
    let required_path_set: BTreeSet<String> = required_paths.iter().cloned().collect();
    ensure_no_duplicates(required_paths, &required_path_set, "required_paths")?;

    let staged_paths = state::cached_paths(root)?;
    let disallowed_paths = staged_paths
        .iter()
        .filter(|path| {
            !allowed_path_set.contains(*path)
                && !allowed_prefixes
                    .iter()
                    .any(|prefix| *path == prefix || path.starts_with(&format!("{prefix}/")))
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing_required_paths = required_path_set
        .difference(&staged_paths)
        .cloned()
        .collect::<BTreeSet<_>>();

    Ok(StagedScopeReport {
        passed: disallowed_paths.is_empty() && missing_required_paths.is_empty(),
        staged_paths,
        disallowed_paths,
        missing_required_paths,
    })
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

    fn repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-git-commit-{name}-{}",
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
    fn the_stage_argv_separates_paths_from_revisions() {
        // Without `--` a path that looks like a revision would be read as one.
        assert_eq!(
            stage_argv(&owned(&["a.txt"])),
            owned(&["add", "--", "a.txt"])
        );
    }

    #[test]
    fn a_body_becomes_a_second_message_rather_than_joining_the_subject() {
        assert_eq!(
            commit_argv("Subject", ""),
            owned(&["commit", "--quiet", "-m", "Subject"])
        );
        assert_eq!(
            commit_argv("Subject", "Why"),
            owned(&["commit", "--quiet", "-m", "Subject", "-m", "Why"])
        );
    }

    #[test]
    fn an_exact_commit_requires_the_whole_dirty_set() {
        let root = repo("exact_whole_set");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("fresh.txt"), "beta\n").unwrap();

        // Naming only one of two dirty paths means the caller is working from a stale view.
        let error = plan_commit_exact(&root, &owned(&["tracked.txt"]))
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("provided paths must exactly match the full dirty-path set"), "{error}");
        assert!(error.contains("missing_from_input:\nfresh.txt"), "{error}");

        let plan = plan_commit_exact(&root, &owned(&["tracked.txt", "fresh.txt"])).unwrap();
        assert_eq!(plan.expected_paths, names(&["tracked.txt", "fresh.txt"]));
    }

    #[test]
    fn a_clean_repository_refuses_every_commit_shape() {
        let root = repo("clean_repository");

        for error in [
            plan_commit_exact(&root, &owned(&["tracked.txt"])).unwrap_err(),
            plan_commit_scoped(&root, &owned(&["tracked.txt"])).unwrap_err(),
            plan_commit_prefix(&root, &owned(&[""])).unwrap_err(),
            plan_stage_exact(&root, &owned(&["tracked.txt"])).unwrap_err(),
        ] {
            assert_eq!(error.to_string(), "repository has no dirty paths");
        }
    }

    #[test]
    fn a_partial_commit_requires_a_clean_index() {
        let root = repo("partial_needs_clean_index");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("fresh.txt"), "beta\n").unwrap();
        git(&root, &["add", "fresh.txt"]);

        // Unrelated staged work would otherwise be swept into a partial commit.
        let scoped = plan_commit_scoped(&root, &owned(&["tracked.txt"]))
            .unwrap_err()
            .to_string();
        assert!(scoped.starts_with("index must be clean before scoped commit"), "{scoped}");
        assert!(scoped.contains("staged_paths:\nfresh.txt"), "{scoped}");

        let staged = plan_stage_exact(&root, &owned(&["tracked.txt"]))
            .unwrap_err()
            .to_string();
        assert!(staged.starts_with("index must be clean before staging exact paths"), "{staged}");
    }

    #[test]
    fn an_exact_commit_does_not_require_a_clean_index() {
        let root = repo("exact_allows_staged");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        git(&root, &["add", "tracked.txt"]);

        // The exact set already accounts for everything dirty, so staged work cannot be unrelated.
        assert!(plan_commit_exact(&root, &owned(&["tracked.txt"])).is_ok());
    }

    #[test]
    fn a_scoped_commit_leaves_unrelated_dirty_paths_behind() {
        let root = repo("scoped_roundtrip");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("fresh.txt"), "beta\n").unwrap();

        let plan = plan_commit_scoped(&root, &owned(&["tracked.txt"])).unwrap();
        assert_eq!(plan.remaining_dirty_after, names(&["fresh.txt"]));

        stage_paths(&root, &plan.paths).unwrap();
        verify_scoped_staged(&root, &plan).unwrap();
        commit_staged(&root, "Scoped", "").unwrap();

        let (dirty_after, still_dirty) = requested_still_dirty(&root, &plan.requested_paths).unwrap();
        assert!(still_dirty.is_empty(), "{still_dirty:?}");
        assert_eq!(dirty_after, names(&["fresh.txt"]));
        assert_eq!(state::rev_count(&root, "HEAD").unwrap(), 2, "exactly one commit");
    }

    #[test]
    fn a_prefix_commit_expands_only_dirty_paths_under_the_prefix() {
        let root = repo("prefix_expansion");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("a.rs"), "alpha\n").unwrap();
        fs::write(root.join("src").join("b.rs"), "beta\n").unwrap();
        fs::write(root.join("outside.txt"), "gamma\n").unwrap();

        let plan = plan_commit_prefix(&root, &owned(&["src"])).unwrap();

        assert_eq!(plan.requested_paths, names(&["src/a.rs", "src/b.rs"]));
        assert_eq!(plan.remaining_dirty_after, names(&["outside.txt"]));

        let unmatched = plan_commit_prefix(&root, &owned(&["absent"]))
            .unwrap_err()
            .to_string();
        assert!(unmatched.starts_with("no dirty paths matched prefixes"), "{unmatched}");
        assert!(unmatched.contains("prefixes:\nabsent"), "{unmatched}");
    }

    #[test]
    fn staging_verification_catches_an_index_that_moved() {
        let root = repo("staging_moved");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("fresh.txt"), "beta\n").unwrap();
        let plan = plan_commit_scoped(&root, &owned(&["tracked.txt"])).unwrap();

        // Something else staged an extra path between planning and verification.
        stage_paths(&root, &plan.paths).unwrap();
        git(&root, &["add", "fresh.txt"]);

        let error = verify_scoped_staged(&root, &plan).unwrap_err().to_string();
        assert!(error.starts_with("staged paths differ from requested scoped set"), "{error}");
        assert!(error.contains("staged:\nfresh.txt\ntracked.txt"), "{error}");
    }

    #[test]
    fn a_staging_request_stages_without_committing() {
        let root = repo("stage_only");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();

        let plan = plan_stage_exact(&root, &owned(&["tracked.txt"])).unwrap();
        stage_paths(&root, &plan.paths).unwrap();
        verify_stage_exact(&root, &plan).unwrap();

        assert_eq!(state::cached_paths(&root).unwrap(), names(&["tracked.txt"]));
        assert_eq!(state::rev_count(&root, "HEAD").unwrap(), 1, "no commit was made");
    }

    #[test]
    fn a_scope_check_reports_rather_than_refuses() {
        let root = repo("scope_check");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("a.rs"), "alpha\n").unwrap();
        git(&root, &["add", "tracked.txt", "src/a.rs"]);

        let report = evaluate_staged_scope(&root, &owned(&["tracked.txt"]), &owned(&["src"]), &[])
            .unwrap();
        assert!(report.passed, "{report:?}");
        assert_eq!(report.staged_paths, names(&["tracked.txt", "src/a.rs"]));

        let narrowed = evaluate_staged_scope(&root, &owned(&["tracked.txt"]), &[], &[]).unwrap();
        assert!(!narrowed.passed);
        assert_eq!(narrowed.disallowed_paths, names(&["src/a.rs"]));

        let demanding =
            evaluate_staged_scope(&root, &[], &owned(&["src", "tracked.txt"]), &owned(&["absent.txt"]))
                .unwrap();
        assert!(!demanding.passed);
        assert_eq!(demanding.missing_required_paths, names(&["absent.txt"]));
    }

    #[test]
    fn duplicates_are_refused_for_every_shape() {
        let root = repo("duplicates");

        assert_eq!(
            plan_commit_exact(&root, &owned(&["a", "a"])).unwrap_err().to_string(),
            "duplicate paths are not allowed"
        );
        assert_eq!(
            plan_commit_prefix(&root, &owned(&["a", "a"])).unwrap_err().to_string(),
            "duplicate prefixes are not allowed"
        );
        assert_eq!(
            evaluate_staged_scope(&root, &owned(&["a", "a"]), &[], &[])
                .unwrap_err()
                .to_string(),
            "duplicate allowed_paths are not allowed"
        );
        assert_eq!(
            evaluate_staged_scope(&root, &[], &owned(&["a", "a"]), &[])
                .unwrap_err()
                .to_string(),
            "duplicate allowed_prefixes are not allowed"
        );
        assert_eq!(
            evaluate_staged_scope(&root, &[], &[], &owned(&["a", "a"]))
                .unwrap_err()
                .to_string(),
            "duplicate required_paths are not allowed"
        );
    }

    #[test]
    fn planning_never_stages_or_commits() {
        let root = repo("planning_is_read_only");
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();

        let _ = plan_commit_exact(&root, &owned(&["tracked.txt"])).unwrap();
        let _ = plan_commit_scoped(&root, &owned(&["tracked.txt"])).unwrap();
        let _ = plan_stage_exact(&root, &owned(&["tracked.txt"])).unwrap();

        assert!(state::cached_paths(&root).unwrap().is_empty(), "nothing was staged");
        assert_eq!(state::rev_count(&root, "HEAD").unwrap(), 1, "nothing was committed");
    }
}
