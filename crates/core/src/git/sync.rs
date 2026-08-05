//! Typed policy for remote inspection, branch preparation, merge readiness, and pushing.
//!
//! These actions reach past the worktree, which is why they are the strictest in the surface. Two of them
//! name no paths at all, so nothing bounds them but the configured root, and that is enforced separately
//! by the worktree-root policy. What lives here is the rest: which refs may be named, what divergence
//! actually means, which branch command sequence a request implies, and which postconditions must hold
//! before a push is allowed to happen.
//!
//! A recurring guard is that a fetch must not disturb the worktree. Fetch only writes refs, so a status
//! that changed across one means something else is running, and continuing would act on a tree the caller
//! never saw. Every fetching action compares status before and after for that reason.
//!
//! Refusals carry the detail only; the adapter supplies the prefix.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::ContextPatchError;
use crate::git::repository::GitRepository;
use crate::git::state;

/// Remote assumed when a caller does not name one.
pub const DEFAULT_REMOTE: &str = "origin";

/// Shortest accepted abbreviated commit hash.
pub const MIN_EXPECTED_HEAD_LENGTH: usize = 7;

/// Longest accepted commit hash.
pub const MAX_EXPECTED_HEAD_LENGTH: usize = 40;

/// Label used when describing a diff payload in a refusal.
pub const DIFF_LABEL: &str = "git diff --name-only";

/// One configured remote as `git remote -v` reports it.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Remote {
    pub name: String,
    pub url: String,
    pub kind: String,
}

/// How a branch and its remote base relate.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Divergence {
    pub head: String,
    pub remote_head: String,
    pub remote_ahead_count: u64,
    pub local_ahead_count: u64,
    pub remote_is_ancestor_of_head: bool,
    pub head_is_ancestor_of_remote: bool,
}

/// Which branch command sequence a preparation request implies.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BranchAction {
    /// The branch exists and the caller confirmed recreating it from the remote base.
    ResetExisting,
    /// The branch exists and is already based on the remote base, so switching is enough.
    SwitchExisting,
    /// The branch does not exist yet.
    Create,
}

impl BranchAction {
    /// Name reported while the action is still only planned.
    pub const fn planned_name(self) -> &'static str {
        match self {
            Self::ResetExisting => "reset_existing_branch",
            Self::SwitchExisting => "switch_existing_branch",
            Self::Create => "create_branch",
        }
    }

    /// Name reported once the action has happened.
    ///
    /// Two of the three read as past tense, which is a deliberate distinction: a plan says what it would
    /// do, a result says what it did.
    pub const fn completed_name(self) -> &'static str {
        match self {
            Self::ResetExisting => "reset_existing_branch",
            Self::SwitchExisting => "switched_existing_branch",
            Self::Create => "created_branch",
        }
    }

    /// The exact command sequence this action runs.
    ///
    /// Deriving it once means the dry-run plan and the executed commands cannot drift apart, which they
    /// previously could because each path built the sequence separately.
    pub fn commands(
        self,
        branch: &str,
        remote_ref: &str,
        already_on_branch: bool,
    ) -> Vec<Vec<String>> {
        match self {
            Self::ResetExisting if already_on_branch => vec![vec![
                "reset".to_string(),
                "--hard".to_string(),
                remote_ref.to_string(),
            ]],
            Self::ResetExisting => vec![
                vec![
                    "branch".to_string(),
                    "-f".to_string(),
                    branch.to_string(),
                    remote_ref.to_string(),
                ],
                vec!["switch".to_string(), branch.to_string()],
            ],
            Self::SwitchExisting => vec![vec!["switch".to_string(), branch.to_string()]],
            Self::Create => vec![vec![
                "switch".to_string(),
                "-c".to_string(),
                branch.to_string(),
                remote_ref.to_string(),
            ]],
        }
    }
}

/// Choose the branch action a request implies.
pub const fn select_branch_action(branch_existed: bool, reset_existing: bool) -> BranchAction {
    match (branch_existed, reset_existing) {
        (true, true) => BranchAction::ResetExisting,
        (true, false) => BranchAction::SwitchExisting,
        (false, _) => BranchAction::Create,
    }
}

/// The remote-tracking ref for a branch.
pub fn remote_tracking_ref(remote: &str, branch: &str) -> String {
    format!("refs/remotes/{remote}/{branch}")
}

/// The refspec used to fetch one branch into its remote-tracking ref.
///
/// Exactly one branch, never a wildcard, so a fetch cannot quietly update refs the caller did not ask
/// about.
pub fn fetch_refspec(remote: &str, branch: &str) -> String {
    format!("refs/heads/{branch}:{}", remote_tracking_ref(remote, branch))
}

/// Name blank text so a before-and-after comparison is readable.
pub fn label_empty(text: &str) -> &str {
    if text.trim().is_empty() {
        "(empty)"
    } else {
        text
    }
}

/// Derive the branch to fetch from a remote-qualified target ref.
///
/// Refused rather than guessed when the ref is not remote-qualified, because fetching the wrong branch
/// would silently answer a different question than the caller asked.
pub fn infer_fetch_branch(remote: &str, target_ref: &str) -> Result<String, ContextPatchError> {
    let qualified = format!("refs/remotes/{remote}/");
    let short = format!("{remote}/");
    if let Some(branch) = target_ref
        .strip_prefix(&qualified)
        .or_else(|| target_ref.strip_prefix(&short))
    {
        return Ok(branch.to_string());
    }
    Err(ContextPatchError::new(format!(
        "fetch=true requires target_branch unless target_ref starts with `{remote}/` or `refs/remotes/{remote}/`"
    )))
}

/// Validate an abbreviated or full commit hash.
pub fn expected_head(raw: &str) -> Result<String, ContextPatchError> {
    if !(MIN_EXPECTED_HEAD_LENGTH..=MAX_EXPECTED_HEAD_LENGTH).contains(&raw.len()) {
        return Err(ContextPatchError::new(format!(
            "expected_head must be a {MIN_EXPECTED_HEAD_LENGTH} to {MAX_EXPECTED_HEAD_LENGTH} character commit hash"
        )));
    }
    if !raw.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(ContextPatchError::new("expected_head must be hexadecimal"));
    }
    Ok(raw.to_ascii_lowercase())
}

/// List configured remotes.
pub fn list_remotes<'a>(
    repository: impl Into<GitRepository<'a>>,
) -> Result<Vec<Remote>, ContextPatchError> {
    let output = state::stdout(repository, &["remote", "-v"])?;
    let mut remotes = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut fields = line.split_whitespace();
        let name = fields
            .next()
            .ok_or_else(|| ContextPatchError::new("malformed remote output"))?;
        let url = fields
            .next()
            .ok_or_else(|| ContextPatchError::new("malformed remote output"))?;
        let kind = fields.next().unwrap_or("").trim_matches(['(', ')']);
        remotes.push(Remote {
            name: name.to_string(),
            url: url.to_string(),
            kind: kind.to_string(),
        });
    }
    Ok(remotes)
}

/// Fetch exactly one refspec.
pub fn fetch<'a>(
    repository: impl Into<GitRepository<'a>>,
    args: Vec<String>,
) -> Result<(), ContextPatchError> {
    state::success(repository, &args)
}

/// Refuse when a fetch disturbed the worktree.
///
/// A fetch writes refs, never files, so a changed status means something else is running and the tree is
/// no longer the one that was inspected.
pub fn ensure_status_unchanged(before: &str, after: &str) -> Result<(), ContextPatchError> {
    if before != after {
        return Err(ContextPatchError::new(format!(
            "source worktree changed during fetch\nbefore:\n{}\nafter:\n{}",
            label_empty(before),
            label_empty(after)
        )));
    }
    Ok(())
}

/// Whether one ref is an ancestor of another.
pub fn is_ancestor<'a>(
    repository: impl Into<GitRepository<'a>>,
    ancestor: &str,
    descendant: &str,
) -> Result<bool, ContextPatchError> {
    Ok(state::exit_code(
        repository,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )? == 0)
}

/// Compute how HEAD and a remote-tracking ref relate.
pub fn divergence<'a>(
    repository: impl Into<GitRepository<'a>>,
    remote_ref: &str,
) -> Result<Divergence, ContextPatchError> {
    // Converted once and reused across all six queries, which the copyable target makes free.
    let repository = repository.into();
    Ok(Divergence {
        head: state::stdout(repository, &["rev-parse", "HEAD"])?
            .trim()
            .to_string(),
        remote_head: state::stdout(repository, &["rev-parse", remote_ref])?
            .trim()
            .to_string(),
        remote_ahead_count: state::rev_count(repository, &format!("HEAD..{remote_ref}"))?,
        local_ahead_count: state::rev_count(repository, &format!("{remote_ref}..HEAD"))?,
        remote_is_ancestor_of_head: is_ancestor(repository, remote_ref, "HEAD")?,
        head_is_ancestor_of_remote: is_ancestor(repository, "HEAD", remote_ref)?,
    })
}

/// Files that differ between two refs.
pub fn changed_files_between<'a>(
    repository: impl Into<GitRepository<'a>>,
    from_ref: &str,
    to_ref: &str,
) -> Result<BTreeSet<String>, ContextPatchError> {
    let output = state::output(
        repository,
        &["diff", "--name-only", "-z", from_ref, to_ref, "--"],
    )?;
    state::parse_nul_paths(&output.stdout, DIFF_LABEL)
}

/// Require a clean worktree before a repository-scoped mutation.
pub fn ensure_worktree_clean(status: &str, reason: &str) -> Result<(), ContextPatchError> {
    if !status.trim().is_empty() {
        return Err(ContextPatchError::new(format!(
            "{reason}\n{}",
            status.trim()
        )));
    }
    Ok(())
}

/// Validate the syntax of a required-file path.
///
/// Stricter than an ordinary repository path: a colon is rejected too, because these paths are also used
/// as `ref:path` object names and a colon there would silently address a different object.
pub fn required_file_path_syntax(raw: &str) -> Result<(), ContextPatchError> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(ContextPatchError::new(
            "required file path must not be empty or contain NUL",
        ));
    }
    if raw.contains(':') || raw.contains('*') || raw.contains('?') || raw.contains('[') {
        return Err(ContextPatchError::new(format!(
            "required file path `{raw}` contains Git pathspec metacharacters"
        )));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(ContextPatchError::new(format!(
            "required file path `{raw}` must be repository-relative"
        )));
    }
    for component in path.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(ContextPatchError::new(format!(
                "required file path `{raw}` must be a normalized relative path"
            )));
        }
    }
    Ok(())
}

/// Require every named path to exist as a file in a ref, before switching to it.
///
/// Checked against the ref rather than the worktree so a preparation that would land on a branch missing
/// a required file refuses before the switch, not after.
pub fn required_files_in_ref<'a>(
    repository: impl Into<GitRepository<'a>>,
    ref_name: &str,
    paths: &[String],
) -> Result<Vec<String>, ContextPatchError> {
    let repository = repository.into();
    paths
        .iter()
        .map(|path| {
            required_file_path_syntax(path)?;
            let object = format!("{ref_name}:{path}");
            let object_type =
                state::stdout(repository, &["cat-file", "-t", &object]).map_err(|_| {
                    ContextPatchError::new(format!(
                        "required file `{path}` is missing from `{ref_name}`"
                    ))
                })?;
            if object_type.trim() != "blob" {
                return Err(ContextPatchError::new(format!(
                    "required path `{path}` in `{ref_name}` is not a file"
                )));
            }
            Ok(path.to_string())
        })
        .collect()
}

/// Require a named path to exist in the worktree and stay inside the root.
///
/// Confinement and presence are both established through the root's descriptor authority, under either
/// authority, so the file confirmed present is the file inside the directory that was selected rather than
/// whatever the name resolves to by the time the check runs.
pub fn required_file_present<'a>(
    root: impl Into<crate::git::root::RepositoryRoot<'a>>,
    raw: &str,
) -> Result<String, ContextPatchError> {
    let root = root.into();
    required_file_path_syntax(raw)?;
    crate::git::validate::git_path(root, raw)?;
    if !crate::fs::rooted::is_regular_file(root, raw)? {
        return Err(ContextPatchError::new(format!(
            "required file `{raw}` is missing after branch preparation"
        )));
    }
    Ok(raw.to_string())
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

    fn unique_dir(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-git-sync-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    /// A bare repository standing in for a remote, plus a clone wired to it.
    ///
    /// Bare on purpose: pushing to a non-bare repository's checked-out branch is refused by Git itself,
    /// which would mask the guards under test.
    fn clone_with_bare_remote(name: &str) -> (PathBuf, PathBuf) {
        let bare = unique_dir(&format!("{name}-remote"));
        git(&bare, &["init", "--quiet", "--bare", "--initial-branch=main"]);

        let seed = unique_dir(&format!("{name}-seed"));
        git(&seed, &["init", "--quiet", "--initial-branch=main"]);
        git(&seed, &["config", "user.email", "guard@example.invalid"]);
        git(&seed, &["config", "user.name", "Guard"]);
        fs::write(seed.join("tracked.txt"), "alpha\n").unwrap();
        fs::write(seed.join("required.txt"), "present\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "--quiet", "-m", "initial"]);
        git(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
        git(&seed, &["push", "--quiet", "origin", "main"]);

        let clone = unique_dir(&format!("{name}-clone"));
        git(
            &clone,
            &["clone", "--quiet", bare.to_str().unwrap(), "."],
        );
        git(&clone, &["config", "user.email", "guard@example.invalid"]);
        git(&clone, &["config", "user.name", "Guard"]);
        (clone, bare)
    }

    fn owned(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn refs_and_refspecs_name_exactly_one_branch() {
        assert_eq!(
            remote_tracking_ref("origin", "main"),
            "refs/remotes/origin/main"
        );
        // One branch, never a wildcard, so a fetch cannot update refs the caller did not ask about.
        assert_eq!(
            fetch_refspec("origin", "main"),
            "refs/heads/main:refs/remotes/origin/main"
        );
    }

    #[test]
    fn a_fetch_branch_is_inferred_only_from_a_remote_qualified_ref() {
        assert_eq!(
            infer_fetch_branch("origin", "refs/remotes/origin/feature").unwrap(),
            "feature"
        );
        assert_eq!(infer_fetch_branch("origin", "origin/feature").unwrap(), "feature");

        // Guessing here would fetch a different branch than the caller asked about.
        let error = infer_fetch_branch("origin", "main").unwrap_err().to_string();
        assert!(error.starts_with("fetch=true requires target_branch"), "{error}");
    }

    #[test]
    fn an_expected_head_must_be_a_plausible_hash() {
        assert_eq!(expected_head("ABCDEF1").unwrap(), "abcdef1");
        assert_eq!(
            expected_head("abc").unwrap_err().to_string(),
            "expected_head must be a 7 to 40 character commit hash"
        );
        assert_eq!(
            expected_head("zzzzzzz").unwrap_err().to_string(),
            "expected_head must be hexadecimal"
        );
    }

    #[test]
    fn branch_actions_report_plans_and_results_in_different_tenses() {
        // A plan says what it would do; a result says what it did.
        assert_eq!(
            select_branch_action(false, false).planned_name(),
            "create_branch"
        );
        assert_eq!(
            select_branch_action(false, false).completed_name(),
            "created_branch"
        );
        assert_eq!(
            select_branch_action(true, false).planned_name(),
            "switch_existing_branch"
        );
        assert_eq!(
            select_branch_action(true, false).completed_name(),
            "switched_existing_branch"
        );
        assert_eq!(
            select_branch_action(true, true).planned_name(),
            "reset_existing_branch"
        );
        assert_eq!(
            select_branch_action(true, true).completed_name(),
            "reset_existing_branch"
        );
    }

    #[test]
    fn a_reset_on_the_current_branch_uses_reset_rather_than_a_forced_move() {
        let remote_ref = remote_tracking_ref("origin", "main");

        // Already on the branch: forcing the ref while checked out would leave the worktree behind.
        assert_eq!(
            BranchAction::ResetExisting.commands("feature", &remote_ref, true),
            vec![owned(&["reset", "--hard", &remote_ref])]
        );
        assert_eq!(
            BranchAction::ResetExisting.commands("feature", &remote_ref, false),
            vec![
                owned(&["branch", "-f", "feature", &remote_ref]),
                owned(&["switch", "feature"]),
            ]
        );
        assert_eq!(
            BranchAction::Create.commands("feature", &remote_ref, false),
            vec![owned(&["switch", "-c", "feature", &remote_ref])]
        );
    }

    #[test]
    fn a_fetch_that_changed_the_worktree_is_refused() {
        // A fetch writes refs, never files, so a changed status means something else is running.
        assert!(ensure_status_unchanged("", "").is_ok());
        let error = ensure_status_unchanged("", " M a.txt").unwrap_err().to_string();
        assert!(error.starts_with("source worktree changed during fetch"), "{error}");
        // Blank sides are named so the comparison is readable.
        assert!(error.contains("before:\n(empty)"), "{error}");
    }

    #[test]
    fn a_dirty_worktree_blocks_a_repository_scoped_mutation() {
        assert!(ensure_worktree_clean("   \n", "must be clean").is_ok());
        // `trim` strips the leading porcelain status column along with the newline, which is what the
        // existing refusal has always shown.
        assert_eq!(
            ensure_worktree_clean(" M a.txt\n", "worktree must be clean before push")
                .unwrap_err()
                .to_string(),
            "worktree must be clean before push\nM a.txt"
        );
    }

    #[test]
    fn a_required_file_path_also_rejects_a_colon() {
        // These paths are used as `ref:path` object names, so a colon would address a different object.
        assert!(required_file_path_syntax("src/lib.rs").is_ok());
        for hostile in ["main:src/lib.rs", "src/*.rs", "../escape", "/abs"] {
            assert!(required_file_path_syntax(hostile).is_err(), "{hostile}");
        }
        assert_eq!(
            required_file_path_syntax("").unwrap_err().to_string(),
            "required file path must not be empty or contain NUL"
        );
    }

    #[test]
    fn remotes_are_listed_with_their_fetch_and_push_kinds() {
        let (clone, bare) = clone_with_bare_remote("remote_listing");

        let remotes = list_remotes(&clone).unwrap();

        assert!(!remotes.is_empty(), "{remotes:?}");
        assert!(remotes.iter().all(|remote| remote.name == "origin"));
        assert!(remotes.iter().any(|remote| remote.kind == "fetch"));
        assert!(remotes.iter().any(|remote| remote.kind == "push"));
        assert!(remotes
            .iter()
            .all(|remote| remote.url.contains(bare.file_name().unwrap().to_str().unwrap())));
    }

    #[test]
    fn divergence_reports_both_directions_against_a_bare_remote() {
        let (clone, _bare) = clone_with_bare_remote("divergence");
        let remote_ref = remote_tracking_ref("origin", "main");

        let level = divergence(&clone, &remote_ref).unwrap();
        assert_eq!(level.remote_ahead_count, 0);
        assert_eq!(level.local_ahead_count, 0);
        assert!(level.remote_is_ancestor_of_head);
        assert!(level.head_is_ancestor_of_remote);
        assert_eq!(level.head, level.remote_head);

        fs::write(clone.join("local.txt"), "beta\n").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "--quiet", "-m", "local work"]);

        let ahead = divergence(&clone, &remote_ref).unwrap();
        assert_eq!(ahead.local_ahead_count, 1);
        assert_eq!(ahead.remote_ahead_count, 0);
        // Fast-forwardable: the remote is still an ancestor, so a push would be safe.
        assert!(ahead.remote_is_ancestor_of_head);
        assert!(!ahead.head_is_ancestor_of_remote);
    }

    #[test]
    fn changed_files_identify_the_overlap_between_two_branches() {
        let (clone, _bare) = clone_with_bare_remote("changed_files");
        let base = state::head(&clone).unwrap();

        fs::write(clone.join("tracked.txt"), "changed on main\n").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "--quiet", "-m", "main side"]);
        let main_side = state::head(&clone).unwrap();

        git(&clone, &["switch", "--quiet", "-c", "feature", &base]);
        fs::write(clone.join("tracked.txt"), "changed on feature\n").unwrap();
        fs::write(clone.join("only-feature.txt"), "gamma\n").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "--quiet", "-m", "feature side"]);
        let feature_side = state::head(&clone).unwrap();

        let main_changed = changed_files_between(&clone, &base, &main_side).unwrap();
        let feature_changed = changed_files_between(&clone, &base, &feature_side).unwrap();
        let both: BTreeSet<String> = main_changed.intersection(&feature_changed).cloned().collect();

        // The overlap is what a merge would have to reconcile.
        assert_eq!(both, ["tracked.txt".to_string()].into_iter().collect());
        assert!(feature_changed.contains("only-feature.txt"));
    }

    #[test]
    fn required_files_are_checked_against_the_ref_not_the_worktree() {
        let (clone, _bare) = clone_with_bare_remote("required_in_ref");

        assert_eq!(
            required_files_in_ref(&clone, "HEAD", &owned(&["required.txt"])).unwrap(),
            owned(&["required.txt"])
        );

        // Deleting it from the worktree does not change what the ref contains, which is the point: the
        // check runs before a switch, against the ref that would be landed on.
        fs::remove_file(clone.join("required.txt")).unwrap();
        assert!(required_files_in_ref(&clone, "HEAD", &owned(&["required.txt"])).is_ok());
        assert!(required_file_present(&clone, "required.txt").is_err());

        let missing = required_files_in_ref(&clone, "HEAD", &owned(&["absent.txt"]))
            .unwrap_err()
            .to_string();
        assert_eq!(missing, "required file `absent.txt` is missing from `HEAD`");
    }

    #[test]
    fn a_directory_is_not_accepted_as_a_required_file_in_a_ref() {
        let (clone, _bare) = clone_with_bare_remote("required_directory");
        fs::create_dir_all(clone.join("nested")).unwrap();
        fs::write(clone.join("nested").join("inner.txt"), "delta\n").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "--quiet", "-m", "add nested"]);

        let error = required_files_in_ref(&clone, "HEAD", &owned(&["nested"]))
            .unwrap_err()
            .to_string();

        assert_eq!(error, "required path `nested` in `HEAD` is not a file");
    }

    #[test]
    fn ancestry_answers_without_treating_a_negative_as_a_failure() {
        let (clone, _bare) = clone_with_bare_remote("ancestry");
        let base = state::head(&clone).unwrap();
        fs::write(clone.join("local.txt"), "beta\n").unwrap();
        git(&clone, &["add", "."]);
        git(&clone, &["commit", "--quiet", "-m", "local work"]);
        let tip = state::head(&clone).unwrap();

        assert!(is_ancestor(&clone, &base, &tip).unwrap());
        assert!(!is_ancestor(&clone, &tip, &base).unwrap());
    }
}
