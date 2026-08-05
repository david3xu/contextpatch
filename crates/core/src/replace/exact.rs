use std::collections::{BTreeMap, HashMap};
use std::env;
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::ContextPatchError;
use crate::fs::file_identity::FileIdentity;
use crate::fs::guarded_file::{open_regular_file_in_root, GuardedRegularFile};
use crate::fs::hash::{sha256_bytes, validate_sha256};
use crate::fs::mutation_lock::try_file_mutation_lock_for_open_file;

pub const MAX_BULK_REPLACE_ENTRIES: usize = 64;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReplaceExactSummary {
    pub path: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
    pub bytes_written: usize,
    /// Digest of the bytes this replacement wrote, so a caller can chain it as the next
    /// `expected_sha256` without a separate read.
    pub sha256: String,
}

#[derive(Debug, Clone, Copy)]
pub struct ReplaceExactEntry<'a> {
    pub path: &'a Path,
    pub old: &'a str,
    pub new: &'a str,
    pub expected_sha256: Option<&'a str>,
}

/// One anchor resolved inside a captured file snapshot.
///
/// `entry_index` is the position of the originating entry in the caller's list, so a refusal or a
/// summary names the entry the caller actually submitted rather than a position after grouping.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PlannedHunk {
    pub entry_index: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    replacement: String,
}

/// Result of applying every hunk planned for one file, written in a single atomic replacement.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReplaceExactFileSummary {
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub bytes_written: usize,
    /// Digest of the bytes this write produced, so a caller can chain it as the next
    /// `expected_sha256` without a separate read.
    pub sha256: String,
    pub hunks: Vec<PlannedHunk>,
}

/// A refusal paired with the entry that caused it.
///
/// Hunk resolution is shared between the single-file entry points and the bulk planner. Single-file
/// callers discard the index to keep their original bare refusal text; the bulk planner uses it to
/// name the offending entry.
type HunkRefusal = (usize, ContextPatchError);

/// Prefix a refusal with the entry that caused it, so a batch failure is attributable.
fn entry_error(index: usize, path: &Path, detail: &str) -> ContextPatchError {
    ContextPatchError::new(format!("entry {index} (`{}`): {detail}", path.display()))
}

pub fn replace_exact(
    path: &Path,
    old: &str,
    new: &str,
) -> Result<ReplaceExactSummary, ContextPatchError> {
    let repo_root = env::current_dir().map_err(|error| {
        ContextPatchError::new(format!("failed to read current directory: {error}"))
    })?;
    replace_exact_in_root(&repo_root, path, old, new)
}

pub fn replace_exact_in_root(
    repo_root: &Path,
    path: &Path,
    old: &str,
    new: &str,
) -> Result<ReplaceExactSummary, ContextPatchError> {
    replace_exact_in_root_with_sha256(repo_root, path, old, new, None)
}

pub fn replace_exact_in_root_with_sha256(
    repo_root: &Path,
    path: &Path,
    old: &str,
    new: &str,
    expected_sha256: Option<&str>,
) -> Result<ReplaceExactSummary, ContextPatchError> {
    validate_replace_request(old, expected_sha256)?;
    let target = open_regular_file_in_root(repo_root, path)?;
    let target_path = target.target_path();
    let _mutation_lock =
        try_file_mutation_lock_for_open_file(repo_root, &target_path, target.file())?;
    let current_bytes = target.read_all()?;
    let planned = plan_guarded_replacement(&target, current_bytes, old, new, expected_sha256)?;
    target.replace_atomic(planned.updated.as_bytes())?;
    Ok(planned.summary())
}

fn validate_replace_request(
    old: &str,
    expected_sha256: Option<&str>,
) -> Result<(), ContextPatchError> {
    if old.is_empty() {
        return Err(ContextPatchError::new("old text must not be empty"));
    }
    if let Some(expected_sha256) = expected_sha256 {
        validate_sha256(expected_sha256)?;
    }
    Ok(())
}

fn plan_guarded_replacement(
    target: &GuardedRegularFile,
    current_bytes: Vec<u8>,
    old: &str,
    new: &str,
    expected_sha256: Option<&str>,
) -> Result<PlannedReplacement, ContextPatchError> {
    let entries = [ReplaceExactEntry {
        path: target.relative_path(),
        old,
        new,
        expected_sha256,
    }];
    // Single-file callers report bare refusals, so the entry index is discarded here.
    plan_file_replacement(target, current_bytes, &entries, &[0]).map_err(|(_, error)| error)
}

/// Resolve every hunk for one file against a single captured snapshot.
///
/// Resolving against one snapshot rather than against progressively rewritten text is what makes a
/// hunk's validity independent of the order its siblings would be applied in, and it is what allows
/// overlap to be detected before anything is written. The result is one replacement covering every
/// hunk, so a file is written exactly once.
fn plan_file_replacement(
    target: &GuardedRegularFile,
    snapshot: Vec<u8>,
    entries: &[ReplaceExactEntry<'_>],
    indices: &[usize],
) -> Result<PlannedReplacement, HunkRefusal> {
    let first = indices.first().copied().unwrap_or_default();
    let target_path = target.target_path();

    // One file cannot have two different expected digests. Refuse the contradiction rather than let
    // whichever entry happens to be checked first decide the outcome.
    let mut expected: Option<(usize, &str)> = None;
    for &index in indices {
        let Some(candidate) = entries[index].expected_sha256 else {
            continue;
        };
        match expected {
            None => expected = Some((index, candidate)),
            Some((earlier, existing)) if existing != candidate => {
                return Err((
                    index,
                    ContextPatchError::new(format!(
                        "expected_sha256 {candidate} conflicts with {existing} already required by \
                         entry {earlier} for the same file"
                    )),
                ));
            }
            Some(_) => {}
        }
    }
    if let Some((index, expected_sha256)) = expected {
        let current_sha256 = sha256_bytes(&snapshot);
        if current_sha256 != expected_sha256 {
            return Err((
                index,
                ContextPatchError::new(format!(
                    "current file SHA-256 mismatch; current_sha256={current_sha256}, \
                     expected_sha256={expected_sha256}"
                )),
            ));
        }
    }

    let current = String::from_utf8(snapshot.clone()).map_err(|error| {
        (
            first,
            ContextPatchError::new(format!(
                "failed to read {} as UTF-8 text: {error}",
                target_path.display()
            )),
        )
    })?;

    let mut hunks = Vec::with_capacity(indices.len());
    for &index in indices {
        let matches: Vec<(usize, &str)> = current.match_indices(entries[index].old).collect();
        let (start_byte, matched) = match matches.as_slice() {
            [] => {
                return Err((
                    index,
                    ContextPatchError::new("old text was not found in target file"),
                ))
            }
            [single] => *single,
            _ => {
                return Err((
                    index,
                    ContextPatchError::new(format!(
                        "old text matched {} times; expected exactly one match",
                        matches.len()
                    )),
                ))
            }
        };
        hunks.push(PlannedHunk {
            entry_index: index,
            start_byte,
            end_byte: start_byte + matched.len(),
            replacement: entries[index].new.to_string(),
        });
    }

    hunks.sort_by_key(|hunk| (hunk.start_byte, hunk.end_byte));
    reject_conflicting_hunks(&hunks)?;

    let mut updated = String::with_capacity(current.len());
    let mut cursor = 0;
    for hunk in &hunks {
        updated.push_str(&current[cursor..hunk.start_byte]);
        updated.push_str(&hunk.replacement);
        cursor = hunk.end_byte;
    }
    updated.push_str(&current[cursor..]);

    let identity = FileIdentity::from_file(target.file()).map_err(|error| (first, error))?;

    Ok(PlannedReplacement {
        repo_root: target.repository_root().to_path_buf(),
        path: target_path,
        relative_path: target.relative_path().to_path_buf(),
        identity,
        hunks,
        original: snapshot,
        updated,
    })
}

/// Refuse hunks that cannot all be applied to one snapshot.
///
/// Expects `hunks` ordered by position. An identical pair is a duplicate entry rather than two edits;
/// any other intersection means two entries claim the same bytes.
fn reject_conflicting_hunks(hunks: &[PlannedHunk]) -> Result<(), HunkRefusal> {
    for pair in hunks.windows(2) {
        let (earlier, later) = (&pair[0], &pair[1]);
        if earlier.start_byte == later.start_byte && earlier.end_byte == later.end_byte {
            return Err((
                later.entry_index,
                ContextPatchError::new(format!(
                    "duplicates entry {}; both resolve to bytes {}..{}",
                    earlier.entry_index, earlier.start_byte, earlier.end_byte
                )),
            ));
        }
        if later.start_byte < earlier.end_byte {
            return Err((
                later.entry_index,
                ContextPatchError::new(format!(
                    "overlaps entry {}; bytes {}..{} intersect {}..{}",
                    earlier.entry_index,
                    later.start_byte,
                    later.end_byte,
                    earlier.start_byte,
                    earlier.end_byte
                )),
            ));
        }
    }
    Ok(())
}

/// One validated replacement, resolved and computed but not yet written.
///
/// The original bytes are retained so apply can refuse if the file changed after planning. This closes
/// the plan/apply window for cooperating ContextPatch writers; external writers that ignore advisory
/// locks can still race the final comparison and rename.
#[derive(Debug)]
pub struct PlannedReplacement {
    repo_root: PathBuf,
    pub path: PathBuf,
    relative_path: PathBuf,
    identity: FileIdentity,
    hunks: Vec<PlannedHunk>,
    original: Vec<u8>,
    updated: String,
}

impl PlannedReplacement {
    pub fn bytes_to_write(&self) -> usize {
        self.updated.len()
    }

    /// Hunks resolved for this file, ordered by position in the snapshot.
    pub fn hunks(&self) -> &[PlannedHunk] {
        &self.hunks
    }

    /// The file this plan targets, relative to the repository root.
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    fn summary(&self) -> ReplaceExactSummary {
        // Reached only from the single-file entry points, which plan exactly one hunk.
        debug_assert_eq!(self.hunks.len(), 1);
        let (start_byte, end_byte) = self
            .hunks
            .first()
            .map_or((0, 0), |hunk| (hunk.start_byte, hunk.end_byte));
        ReplaceExactSummary {
            path: self.path.clone(),
            start_byte,
            end_byte,
            bytes_written: self.updated.len(),
            sha256: sha256_bytes(self.updated.as_bytes()),
        }
    }

    fn file_summary(&self) -> ReplaceExactFileSummary {
        ReplaceExactFileSummary {
            path: self.path.clone(),
            relative_path: self.relative_path.clone(),
            bytes_written: self.updated.len(),
            sha256: sha256_bytes(self.updated.as_bytes()),
            hunks: self.hunks.clone(),
        }
    }
}

/// Validate one replacement without writing it.
pub fn plan_replace_exact_in_root(
    repo_root: &Path,
    path: &Path,
    old: &str,
    new: &str,
    expected_sha256: Option<&str>,
) -> Result<PlannedReplacement, ContextPatchError> {
    validate_replace_request(old, expected_sha256)?;
    let target = open_regular_file_in_root(repo_root, path)?;
    let target_path = target.target_path();
    let _mutation_lock =
        try_file_mutation_lock_for_open_file(repo_root, &target_path, target.file())?;
    let current_bytes = target.read_all()?;
    plan_guarded_replacement(&target, current_bytes, old, new, expected_sha256)
}

/// Validate a bounded batch before its caller starts applying entries.
///
/// Entries naming the same normalized path are grouped into one plan and resolved against a single
/// captured snapshot of that file, so several hunks reach one file in one atomic write. Every refusal
/// happens here, before any write, which is what guarantees that a validation failure leaves every
/// target unchanged.
///
/// Plans are returned ordered by normalized path, so grouping, lock acquisition, and refusal all stay
/// deterministic regardless of the order entries were submitted in. Applying the returned plans is
/// still per-file rather than transactional: a later apply failure can leave an already-applied
/// prefix.
pub fn plan_bulk_replace_exact_in_root(
    repo_root: &Path,
    entries: &[ReplaceExactEntry<'_>],
) -> Result<Vec<PlannedReplacement>, ContextPatchError> {
    if entries.is_empty() {
        return Err(ContextPatchError::new(
            "bulk replacement requires at least one entry",
        ));
    }
    if entries.len() > MAX_BULK_REPLACE_ENTRIES {
        return Err(ContextPatchError::new(format!(
            "bulk replacement has {} entries; maximum is {MAX_BULK_REPLACE_ENTRIES}",
            entries.len()
        )));
    }

    // Group by path first. A sorted map keeps the plan order, and therefore the order in which locks
    // are taken and refusals are reported, independent of submission order.
    let mut grouped: BTreeMap<&Path, Vec<usize>> = BTreeMap::new();
    for (index, entry) in entries.iter().enumerate() {
        validate_replace_request(entry.old, entry.expected_sha256)
            .map_err(|error| entry_error(index, entry.path, &error.to_string()))?;
        grouped.entry(entry.path).or_default().push(index);
    }

    // Open every distinct target before planning any of them, and refuse two spellings that resolve
    // to one file. Grouping is by path, so an alias would otherwise produce two independent snapshots
    // of the same bytes and the second write would silently discard the first.
    let mut identities: HashMap<FileIdentity, &Path> = HashMap::with_capacity(grouped.len());
    let mut opened = Vec::with_capacity(grouped.len());
    for (path, indices) in grouped {
        let first = indices[0];
        let target = open_regular_file_in_root(repo_root, path)
            .map_err(|error| entry_error(first, path, &error.to_string()))?;
        let identity = FileIdentity::from_file(target.file())
            .map_err(|error| entry_error(first, path, &error.to_string()))?;
        if let Some(existing) = identities.insert(identity, path) {
            return Err(entry_error(
                first,
                path,
                &format!(
                    "resolves to the same file as `{}`; a filesystem alias or hard link cannot be \
                     targeted twice in one bulk replacement",
                    existing.display()
                ),
            ));
        }
        opened.push((path, indices, target));
    }

    opened
        .into_iter()
        .map(|(path, indices, target)| {
            let target_path = target.target_path();
            let _mutation_lock =
                try_file_mutation_lock_for_open_file(repo_root, &target_path, target.file())
                    .map_err(|error| entry_error(indices[0], path, &error.to_string()))?;
            let snapshot = target
                .read_all()
                .map_err(|error| entry_error(indices[0], path, &error.to_string()))?;
            plan_file_replacement(&target, snapshot, entries, &indices)
                .map_err(|(index, error)| entry_error(index, path, &error.to_string()))
        })
        .collect()
}

/// Write one previously planned replacement, covering every hunk planned for that file.
///
/// The exact bytes captured by planning are compared under the target mutation lock before the atomic
/// write, and the filesystem identity is revalidated, so a file that changed after planning is refused
/// rather than overwritten. `expected_sha256` is not required for this revalidation. Every hunk lands
/// in one atomic write, so a file is never observed partially edited.
pub fn apply_planned_replacement(
    repo_root: &Path,
    planned: &PlannedReplacement,
) -> Result<ReplaceExactFileSummary, ContextPatchError> {
    let current_root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {} while applying replacement plan: {error}",
            repo_root.display()
        ))
    })?;
    if current_root != planned.repo_root {
        return Err(ContextPatchError::new(format!(
            "replacement plan belongs to repository root {}; refusing apply under {}",
            planned.repo_root.display(),
            current_root.display()
        )));
    }
    let target = open_regular_file_in_root(&current_root, &planned.relative_path)?;
    let target_path = target.target_path();
    let _mutation_lock =
        try_file_mutation_lock_for_open_file(repo_root, &target_path, target.file())?;
    let current_identity = FileIdentity::from_file(target.file())?;
    if current_identity != planned.identity {
        return Err(ContextPatchError::new(format!(
            "target file changed after replacement was planned; expected filesystem identity for {}, found a different target at {}",
            planned.path.display(),
            target_path.display()
        )));
    }
    let current = target.read_all()?;
    if current != planned.original {
        return Err(ContextPatchError::new(format!(
            "target file changed after replacement was planned; current_sha256={}, \
             planned_sha256={}",
            sha256_bytes(&current),
            sha256_bytes(&planned.original)
        )));
    }
    target.replace_atomic(planned.updated.as_bytes())?;
    Ok(planned.file_summary())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn replaces_exactly_one_match() {
        let root = test_root("replaces_exactly_one_match");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();

        let summary =
            replace_exact_in_root(&root, Path::new("sample.txt"), "beta", "delta").unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha delta gamma");
        assert_eq!(summary.start_byte, 6);
        assert_eq!(summary.end_byte, 10);
    }

    #[test]
    fn refuses_empty_old_text() {
        let root = test_root("refuses_empty_old_text");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha").unwrap();

        let error = replace_exact_in_root(&root, Path::new("sample.txt"), "", "delta").unwrap_err();

        assert_eq!(error.to_string(), "old text must not be empty");
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha");
    }

    #[test]
    fn refuses_zero_matches() {
        let root = test_root("refuses_zero_matches");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha").unwrap();

        let error =
            replace_exact_in_root(&root, Path::new("sample.txt"), "missing", "delta").unwrap_err();

        assert_eq!(error.to_string(), "old text was not found in target file");
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha");
    }

    #[test]
    fn refuses_multiple_matches() {
        let root = test_root("refuses_multiple_matches");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta beta").unwrap();

        let error =
            replace_exact_in_root(&root, Path::new("sample.txt"), "beta", "delta").unwrap_err();

        assert_eq!(
            error.to_string(),
            "old text matched 2 times; expected exactly one match"
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha beta beta");
    }

    #[test]
    fn refuses_paths_outside_root() {
        let root = test_root("refuses_paths_outside_root");
        let outside_root = test_root("refuses_paths_outside_root_outside");
        let outside_file = outside_root.join("outside.txt");
        fs::write(&outside_file, "alpha").unwrap();

        let error = replace_exact_in_root(&root, &outside_file, "alpha", "delta").unwrap_err();

        assert!(error
            .to_string()
            .contains("normalized repository-relative path"));
        assert_eq!(fs::read_to_string(&outside_file).unwrap(), "alpha");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_intermediate_symlinks_without_mutating_outside_files() {
        let root = test_root("refuses_intermediate_symlinks");
        let outside = test_root("refuses_intermediate_symlinks_outside");
        let outside_file = outside.join("sample.txt");
        fs::write(&outside_file, "alpha beta gamma").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();

        let error = replace_exact_in_root(&root, Path::new("linked/sample.txt"), "beta", "delta")
            .unwrap_err();

        assert!(error.to_string().contains("symlink"));
        assert_eq!(
            fs::read_to_string(outside_file).unwrap(),
            "alpha beta gamma"
        );
    }

    #[test]
    fn replaces_when_expected_sha256_matches_exact_bytes() {
        let root = test_root("replaces_when_expected_sha256_matches_exact_bytes");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let expected = sha256_bytes(b"alpha beta gamma");

        replace_exact_in_root_with_sha256(
            &root,
            Path::new("sample.txt"),
            "beta",
            "delta",
            Some(&expected),
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha delta gamma");
    }

    #[test]
    fn refuses_mismatched_expected_sha256_without_mutating() {
        let root = test_root("refuses_mismatched_expected_sha256_without_mutating");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let expected = sha256_bytes(b"stale content");

        let error = replace_exact_in_root_with_sha256(
            &root,
            Path::new("sample.txt"),
            "beta",
            "delta",
            Some(&expected),
        )
        .unwrap_err();

        assert!(error.to_string().contains("SHA-256 mismatch"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha beta gamma");
    }

    #[test]
    fn refuses_invalid_expected_sha256_without_mutating() {
        let root = test_root("refuses_invalid_expected_sha256_without_mutating");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();

        let error = replace_exact_in_root_with_sha256(
            &root,
            Path::new("sample.txt"),
            "beta",
            "delta",
            Some("not-a-digest"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("64 hexadecimal"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha beta gamma");
    }

    #[test]
    fn concurrent_hash_guarded_replacements_do_not_overwrite_each_other() {
        let root = test_root("concurrent_hash_guarded_replacements_do_not_overwrite_each_other");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let expected = sha256_bytes(b"alpha beta gamma");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));

        let workers = ["delta", "epsilon"].map(|replacement| {
            let root = root.clone();
            let expected = expected.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                replace_exact_in_root_with_sha256(
                    &root,
                    Path::new("sample.txt"),
                    "beta",
                    replacement,
                    Some(&expected),
                )
            })
        });
        barrier.wait();
        let outcomes = workers.map(|worker| worker.join().unwrap());

        assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
        let final_text = fs::read_to_string(&file).unwrap();
        assert!(matches!(
            final_text.as_str(),
            "alpha delta gamma" | "alpha epsilon gamma"
        ));
    }

    #[test]
    fn bulk_validation_failure_leaves_every_file_unchanged() {
        let root = test_root("bulk_validation_failure_leaves_every_file_unchanged");
        let one = root.join("one.txt");
        let two = root.join("two.txt");
        fs::write(&one, "alpha beta gamma").unwrap();
        fs::write(&two, "delta delta").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("one.txt"),
                old: "beta",
                new: "BETA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("two.txt"),
                old: "delta",
                new: "DELTA",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("entry 1"));
        assert!(error.to_string().contains("matched 2 times"));
        assert_eq!(fs::read_to_string(one).unwrap(), "alpha beta gamma");
        assert_eq!(fs::read_to_string(two).unwrap(), "delta delta");
    }

    #[test]
    fn bulk_stale_digest_leaves_every_file_unchanged() {
        let root = test_root("bulk_stale_digest_leaves_every_file_unchanged");
        let one = root.join("one.txt");
        let two = root.join("two.txt");
        fs::write(&one, "alpha beta gamma").unwrap();
        fs::write(&two, "delta epsilon").unwrap();
        let stale = sha256_bytes(b"stale");
        let entries = [
            ReplaceExactEntry {
                path: Path::new("one.txt"),
                old: "beta",
                new: "BETA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("two.txt"),
                old: "epsilon",
                new: "EPSILON",
                expected_sha256: Some(&stale),
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("entry 1"));
        assert!(error.to_string().contains("SHA-256 mismatch"));
        assert_eq!(fs::read_to_string(one).unwrap(), "alpha beta gamma");
        assert_eq!(fs::read_to_string(two).unwrap(), "delta epsilon");
    }

    #[test]
    fn bulk_zero_match_leaves_every_file_unchanged() {
        let root = test_root("bulk_zero_match_leaves_every_file_unchanged");
        let one = root.join("one.txt");
        let two = root.join("two.txt");
        fs::write(&one, "alpha beta gamma").unwrap();
        fs::write(&two, "delta epsilon").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("one.txt"),
                old: "beta",
                new: "BETA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("two.txt"),
                old: "missing",
                new: "MISSING",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("entry 1"));
        assert!(error.to_string().contains("old text was not found"));
        assert_eq!(fs::read_to_string(one).unwrap(), "alpha beta gamma");
        assert_eq!(fs::read_to_string(two).unwrap(), "delta epsilon");
    }

    #[test]
    fn bulk_applies_multiple_hunks_to_one_file_in_one_write() {
        let root = test_root("bulk_applies_multiple_hunks_to_one_file_in_one_write");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: None,
            },
        ];

        let planned = plan_bulk_replace_exact_in_root(&root, &entries).unwrap();

        // Both hunks belong to one plan, so they reach the file in one atomic write.
        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].hunks().len(), 2);

        let summary = apply_planned_replacement(&root, &planned[0]).unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "ALPHA beta GAMMA");
        assert_eq!(summary.hunks.len(), 2);
        assert_eq!(summary.bytes_written, "ALPHA beta GAMMA".len());
    }

    #[test]
    fn bulk_hunks_resolve_against_one_snapshot_not_rewritten_text() {
        let root = test_root("bulk_hunks_resolve_against_one_snapshot_not_rewritten_text");
        let file = root.join("sample.txt");
        fs::write(&file, "one two three").unwrap();
        // Swapping the two words is only expressible against a single snapshot. Resolved against
        // progressively rewritten text, the second anchor would match the text the first hunk just
        // inserted and become ambiguous.
        let entries = [
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "one",
                new: "three",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "three",
                new: "one",
                expected_sha256: None,
            },
        ];

        let planned = plan_bulk_replace_exact_in_root(&root, &entries).unwrap();
        apply_planned_replacement(&root, &planned[0]).unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "three two one");
    }

    #[test]
    fn bulk_hunk_result_is_independent_of_submission_order() {
        let root = test_root("bulk_hunk_result_is_independent_of_submission_order");
        let forward = root.join("forward.txt");
        let reverse = root.join("reverse.txt");
        fs::write(&forward, "alpha beta gamma").unwrap();
        fs::write(&reverse, "alpha beta gamma").unwrap();

        let ascending = [
            ReplaceExactEntry {
                path: Path::new("forward.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("forward.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: None,
            },
        ];
        // The same two hunks, submitted in the opposite order.
        let descending = [
            ReplaceExactEntry {
                path: Path::new("reverse.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("reverse.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
        ];

        let ascending_plan = plan_bulk_replace_exact_in_root(&root, &ascending).unwrap();
        let descending_plan = plan_bulk_replace_exact_in_root(&root, &descending).unwrap();
        apply_planned_replacement(&root, &ascending_plan[0]).unwrap();
        apply_planned_replacement(&root, &descending_plan[0]).unwrap();

        assert_eq!(fs::read_to_string(&forward).unwrap(), "ALPHA beta GAMMA");
        assert_eq!(fs::read_to_string(&reverse).unwrap(), "ALPHA beta GAMMA");
    }

    #[test]
    fn bulk_refuses_overlapping_hunks_without_writing() {
        let root = test_root("bulk_refuses_overlapping_hunks_without_writing");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "alpha beta",
                new: "AB",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "beta gamma",
                new: "BG",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("overlaps entry 0"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha beta gamma");
    }

    #[test]
    fn bulk_refuses_duplicate_hunks_without_writing() {
        let root = test_root("bulk_refuses_duplicate_hunks_without_writing");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "beta",
                new: "BETA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "beta",
                new: "OTHER",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("duplicates entry 0"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha beta gamma");
    }

    #[test]
    fn bulk_refuses_an_ambiguous_hunk_among_siblings_without_writing() {
        let root = test_root("bulk_refuses_an_ambiguous_hunk_among_siblings_without_writing");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta beta gamma").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "beta",
                new: "BETA",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("entry 1"));
        assert!(error.to_string().contains("matched 2 times"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha beta beta gamma");
    }

    #[test]
    fn bulk_refuses_a_missing_hunk_among_siblings_without_writing() {
        let root = test_root("bulk_refuses_a_missing_hunk_among_siblings_without_writing");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "missing",
                new: "MISSING",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("entry 1"));
        assert!(error.to_string().contains("old text was not found"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha beta gamma");
    }

    #[test]
    fn bulk_refuses_conflicting_expected_digests_for_one_file() {
        let root = test_root("bulk_refuses_conflicting_expected_digests_for_one_file");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let current = sha256_bytes(b"alpha beta gamma");
        let other = sha256_bytes(b"something else");
        let entries = [
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: Some(&current),
            },
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: Some(&other),
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("conflicts with"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "alpha beta gamma");
    }

    #[test]
    fn bulk_accepts_one_expected_digest_shared_by_sibling_hunks() {
        let root = test_root("bulk_accepts_one_expected_digest_shared_by_sibling_hunks");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let current = sha256_bytes(b"alpha beta gamma");
        let entries = [
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: Some(&current),
            },
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: Some(&current),
            },
        ];

        let planned = plan_bulk_replace_exact_in_root(&root, &entries).unwrap();
        apply_planned_replacement(&root, &planned[0]).unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "ALPHA beta GAMMA");
    }

    #[test]
    fn bulk_multi_hunk_refusal_leaves_every_file_unchanged() {
        let root = test_root("bulk_multi_hunk_refusal_leaves_every_file_unchanged");
        let one = root.join("one.txt");
        let two = root.join("two.txt");
        fs::write(&one, "alpha beta gamma").unwrap();
        fs::write(&two, "delta epsilon").unwrap();
        // Two valid hunks for the first file, then an unresolvable hunk for the second.
        let entries = [
            ReplaceExactEntry {
                path: Path::new("one.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("one.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("two.txt"),
                old: "missing",
                new: "MISSING",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("entry 2"));
        assert_eq!(fs::read_to_string(&one).unwrap(), "alpha beta gamma");
        assert_eq!(fs::read_to_string(&two).unwrap(), "delta epsilon");
    }

    #[cfg(unix)]
    #[test]
    fn bulk_multi_hunk_write_preserves_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;

        let root = test_root("bulk_multi_hunk_write_preserves_the_executable_bit");
        let file = root.join("script.sh");
        fs::write(&file, "alpha beta gamma").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("script.sh"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("script.sh"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: None,
            },
        ];

        let planned = plan_bulk_replace_exact_in_root(&root, &entries).unwrap();
        apply_planned_replacement(&root, &planned[0]).unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "ALPHA beta GAMMA");
        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn bulk_plans_are_ordered_by_path_regardless_of_submission_order() {
        let root = test_root("bulk_plans_are_ordered_by_path_regardless_of_submission_order");
        fs::write(root.join("a.txt"), "alpha").unwrap();
        fs::write(root.join("b.txt"), "beta").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("b.txt"),
                old: "beta",
                new: "BETA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("a.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
        ];

        let planned = plan_bulk_replace_exact_in_root(&root, &entries).unwrap();

        // Deterministic plan order keeps lock acquisition and refusal reporting stable.
        assert_eq!(planned[0].relative_path(), Path::new("a.txt"));
        assert_eq!(planned[1].relative_path(), Path::new("b.txt"));
        assert_eq!(planned[0].hunks()[0].entry_index, 1);
        assert_eq!(planned[1].hunks()[0].entry_index, 0);
    }

    #[test]
    fn bulk_hard_link_aliases_are_refused_before_writing() {
        let root = test_root("bulk_hard_link_aliases_are_refused_before_writing");
        let original = root.join("original.txt");
        let alias = root.join("alias.txt");
        fs::write(&original, "alpha beta gamma").unwrap();
        fs::hard_link(&original, &alias).unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("original.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("alias.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("filesystem alias"));
        assert_eq!(fs::read_to_string(&original).unwrap(), "alpha beta gamma");
        assert_eq!(fs::read_to_string(&alias).unwrap(), "alpha beta gamma");
    }

    #[test]
    fn bulk_case_aliases_are_refused_before_writing_when_supported() {
        let root = test_root("bulk_case_aliases_are_refused_before_writing_when_supported");
        let mixed_case = root.join("CaseAlias.txt");
        let lower_case = root.join("casealias.txt");
        fs::write(&mixed_case, "alpha beta gamma").unwrap();
        if !lower_case.exists() {
            return;
        }
        let entries = [
            ReplaceExactEntry {
                path: Path::new("CaseAlias.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("casealias.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("filesystem alias"));
        assert_eq!(fs::read_to_string(&mixed_case).unwrap(), "alpha beta gamma");
        assert_eq!(fs::read_to_string(&lower_case).unwrap(), "alpha beta gamma");
    }

    #[test]
    fn planned_replacement_refuses_changed_original_bytes() {
        let root = test_root("planned_replacement_refuses_changed_original_bytes");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let planned =
            plan_replace_exact_in_root(&root, Path::new("sample.txt"), "beta", "BETA", None)
                .unwrap();
        fs::write(&file, "external change").unwrap();

        let error = apply_planned_replacement(&root, &planned).unwrap_err();

        assert!(error
            .to_string()
            .contains("changed after replacement was planned"));
        assert_eq!(fs::read_to_string(file).unwrap(), "external change");
    }

    #[test]
    fn planned_replacement_refuses_a_different_repository_root() {
        let planning_root = test_root("planned_replacement_planning_root");
        let other_root = test_root("planned_replacement_other_root");
        let planning_file = planning_root.join("sample.txt");
        let other_file = other_root.join("sample.txt");
        fs::write(&planning_file, "alpha beta gamma").unwrap();
        fs::hard_link(&planning_file, &other_file).unwrap();
        let planned = plan_replace_exact_in_root(
            &planning_root,
            Path::new("sample.txt"),
            "beta",
            "BETA",
            None,
        )
        .unwrap();

        let error = apply_planned_replacement(&other_root, &planned).unwrap_err();

        assert!(error
            .to_string()
            .contains("replacement plan belongs to repository root"));
        assert_eq!(
            fs::read_to_string(planning_file).unwrap(),
            "alpha beta gamma"
        );
        assert_eq!(fs::read_to_string(other_file).unwrap(), "alpha beta gamma");
    }

    #[test]
    fn later_apply_failure_can_leave_a_validated_prefix_applied() {
        let root = test_root("later_apply_failure_can_leave_a_validated_prefix_applied");
        let one = root.join("one.txt");
        let two = root.join("two.txt");
        fs::write(&one, "alpha beta").unwrap();
        fs::write(&two, "gamma delta").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("one.txt"),
                old: "beta",
                new: "BETA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("two.txt"),
                old: "delta",
                new: "DELTA",
                expected_sha256: None,
            },
        ];
        let planned = plan_bulk_replace_exact_in_root(&root, &entries).unwrap();

        apply_planned_replacement(&root, &planned[0]).unwrap();
        fs::write(&two, "external change").unwrap();
        let error = apply_planned_replacement(&root, &planned[1]).unwrap_err();

        assert!(error
            .to_string()
            .contains("changed after replacement was planned"));
        assert_eq!(fs::read_to_string(one).unwrap(), "alpha BETA");
        assert_eq!(fs::read_to_string(two).unwrap(), "external change");
    }

    #[test]
    fn a_single_replacement_reports_the_digest_of_the_bytes_on_disk() {
        let root = test_root("a_single_replacement_reports_the_digest_of_the_bytes_on_disk");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();

        let summary =
            replace_exact_in_root(&root, Path::new("sample.txt"), "beta", "delta").unwrap();

        assert_eq!(summary.sha256, sha256_bytes(&fs::read(&file).unwrap()));
    }

    #[test]
    fn a_reported_digest_chains_into_the_next_replacement() {
        let root = test_root("a_reported_digest_chains_into_the_next_replacement");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();

        // The digest one write reports must be usable as the guard for the next, with no intervening
        // read. That is the whole reason for returning it.
        let first = replace_exact_in_root(&root, Path::new("sample.txt"), "alpha", "ALPHA").unwrap();
        let second = replace_exact_in_root_with_sha256(
            &root,
            Path::new("sample.txt"),
            "gamma",
            "GAMMA",
            Some(&first.sha256),
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&file).unwrap(), "ALPHA beta GAMMA");
        assert_eq!(second.sha256, sha256_bytes(&fs::read(&file).unwrap()));
    }

    #[test]
    fn a_reported_digest_stops_guarding_once_the_file_changes() {
        let root = test_root("a_reported_digest_stops_guarding_once_the_file_changes");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();

        let first = replace_exact_in_root(&root, Path::new("sample.txt"), "alpha", "ALPHA").unwrap();
        fs::write(&file, "external change gamma").unwrap();

        let error = replace_exact_in_root_with_sha256(
            &root,
            Path::new("sample.txt"),
            "gamma",
            "GAMMA",
            Some(&first.sha256),
        )
        .unwrap_err();

        assert!(error.to_string().contains("SHA-256 mismatch"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "external change gamma");
    }

    #[test]
    fn a_multi_hunk_write_reports_one_digest_matching_the_file_on_disk() {
        let root = test_root("a_multi_hunk_write_reports_one_digest_matching_the_file_on_disk");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta gamma").unwrap();
        let entries = [
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "alpha",
                new: "ALPHA",
                expected_sha256: None,
            },
            ReplaceExactEntry {
                path: Path::new("sample.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: None,
            },
        ];

        let planned = plan_bulk_replace_exact_in_root(&root, &entries).unwrap();
        let summary = apply_planned_replacement(&root, &planned[0]).unwrap();

        // One write means one digest, covering every hunk, and it must match the file.
        assert_eq!(summary.hunks.len(), 2);
        assert_eq!(summary.sha256, sha256_bytes(&fs::read(&file).unwrap()));
    }

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
