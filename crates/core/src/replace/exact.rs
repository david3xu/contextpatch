use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::{env, fs};

use crate::error::ContextPatchError;
use crate::fs::atomic_write::write_atomic;
use crate::fs::file_identity::FileIdentity;
use crate::fs::hash::{sha256_bytes, validate_sha256};
use crate::fs::mutation_lock::try_file_mutation_lock;
use crate::fs::path::resolve_existing_file;

pub const MAX_BULK_REPLACE_ENTRIES: usize = 64;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReplaceExactSummary {
    pub path: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
    pub bytes_written: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct ReplaceExactEntry<'a> {
    pub path: &'a Path,
    pub old: &'a str,
    pub new: &'a str,
    pub expected_sha256: Option<&'a str>,
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
    let target_path = resolve_existing_file(repo_root, path)?;
    let _mutation_lock = try_file_mutation_lock(repo_root, &target_path)?;
    let current_bytes = read_target(&target_path)?;
    let planned = plan_resolved_replacement(target_path, current_bytes, old, new, expected_sha256)?;
    write_atomic(&planned.path, planned.updated.as_bytes())?;
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

fn read_target(target_path: &Path) -> Result<Vec<u8>, ContextPatchError> {
    fs::read(target_path).map_err(|error| {
        ContextPatchError::new(format!("failed to read {}: {error}", target_path.display()))
    })
}

fn plan_resolved_replacement(
    target_path: PathBuf,
    current_bytes: Vec<u8>,
    old: &str,
    new: &str,
    expected_sha256: Option<&str>,
) -> Result<PlannedReplacement, ContextPatchError> {
    if let Some(expected_sha256) = expected_sha256 {
        let current_sha256 = sha256_bytes(&current_bytes);
        if current_sha256 != expected_sha256 {
            return Err(ContextPatchError::new(format!(
                "current file SHA-256 mismatch; current_sha256={current_sha256}, \
                 expected_sha256={expected_sha256}"
            )));
        }
    }
    let current = String::from_utf8(current_bytes.clone()).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to read {} as UTF-8 text: {error}",
            target_path.display()
        ))
    })?;

    let matches: Vec<(usize, &str)> = current.match_indices(old).collect();
    let (start_byte, matched) = match matches.as_slice() {
        [] => {
            return Err(ContextPatchError::new(
                "old text was not found in target file",
            ))
        }
        [single] => *single,
        _ => {
            return Err(ContextPatchError::new(format!(
                "old text matched {} times; expected exactly one match",
                matches.len()
            )))
        }
    };

    let end_byte = start_byte + matched.len();
    let mut updated = String::with_capacity(current.len() - matched.len() + new.len());
    updated.push_str(&current[..start_byte]);
    updated.push_str(new);
    updated.push_str(&current[end_byte..]);

    Ok(PlannedReplacement {
        path: target_path,
        start_byte,
        end_byte,
        original: current_bytes,
        updated,
    })
}

/// One validated replacement, resolved and computed but not yet written.
///
/// The original bytes are retained so apply can refuse if the file changed after planning. This closes
/// the plan/apply window for cooperating ContextPatch writers; external writers that ignore advisory
/// locks can still race the final comparison and rename.
#[derive(Debug)]
pub struct PlannedReplacement {
    pub path: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
    original: Vec<u8>,
    updated: String,
}

impl PlannedReplacement {
    pub fn bytes_to_write(&self) -> usize {
        self.updated.len()
    }

    fn summary(&self) -> ReplaceExactSummary {
        ReplaceExactSummary {
            path: self.path.clone(),
            start_byte: self.start_byte,
            end_byte: self.end_byte,
            bytes_written: self.updated.len(),
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
    let target_path = resolve_existing_file(repo_root, path)?;
    let _mutation_lock = try_file_mutation_lock(repo_root, &target_path)?;
    let current_bytes = read_target(&target_path)?;
    plan_resolved_replacement(target_path, current_bytes, old, new, expected_sha256)
}

/// Validate a bounded batch before its caller starts applying entries.
///
/// This guarantees that a validation refusal leaves every target unchanged. Applying the returned
/// plans is intentionally per-file rather than transactional: a later apply failure can leave an
/// already-applied prefix.
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

    let mut targets = HashSet::with_capacity(entries.len());
    let mut resolved = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        validate_replace_request(entry.old, entry.expected_sha256).map_err(|error| {
            ContextPatchError::new(format!(
                "entry {index} (`{}`): {error}",
                entry.path.display()
            ))
        })?;
        let target = resolve_existing_file(repo_root, entry.path).map_err(|error| {
            ContextPatchError::new(format!(
                "entry {index} (`{}`): {error}",
                entry.path.display()
            ))
        })?;
        let identity = FileIdentity::from_path(&target).map_err(|error| {
            ContextPatchError::new(format!(
                "entry {index} (`{}`): {error}",
                entry.path.display()
            ))
        })?;
        if !targets.insert(identity) {
            return Err(ContextPatchError::new(format!(
                "entry {index} (`{}`): duplicate target path or filesystem alias in bulk replacement",
                entry.path.display()
            )));
        }
        resolved.push(target);
    }

    entries
        .iter()
        .zip(resolved)
        .enumerate()
        .map(|(index, (entry, target))| {
            let _mutation_lock = try_file_mutation_lock(repo_root, &target).map_err(|error| {
                ContextPatchError::new(format!(
                    "entry {index} (`{}`): {error}",
                    entry.path.display()
                ))
            })?;
            let current_bytes = read_target(&target).map_err(|error| {
                ContextPatchError::new(format!(
                    "entry {index} (`{}`): {error}",
                    entry.path.display()
                ))
            })?;
            plan_resolved_replacement(
                target,
                current_bytes,
                entry.old,
                entry.new,
                entry.expected_sha256,
            )
            .map_err(|error| {
                ContextPatchError::new(format!(
                    "entry {index} (`{}`): {error}",
                    entry.path.display()
                ))
            })
        })
        .collect()
}

/// Write one previously planned replacement.
///
/// The exact bytes captured by planning are compared under the target mutation lock before the atomic
/// write. `expected_sha256` is not required for this revalidation.
pub fn apply_planned_replacement(
    repo_root: &Path,
    planned: &PlannedReplacement,
) -> Result<ReplaceExactSummary, ContextPatchError> {
    let _mutation_lock = try_file_mutation_lock(repo_root, &planned.path)?;
    let resolved = resolve_existing_file(repo_root, &planned.path)?;
    if resolved != planned.path {
        return Err(ContextPatchError::new(format!(
            "target file changed after replacement was planned; expected {}, resolved {}",
            planned.path.display(),
            resolved.display()
        )));
    }
    let current = read_target(&resolved)?;
    if current != planned.original {
        return Err(ContextPatchError::new(format!(
            "target file changed after replacement was planned; current_sha256={}, \
             planned_sha256={}",
            sha256_bytes(&current),
            sha256_bytes(&planned.original)
        )));
    }
    write_atomic(&planned.path, planned.updated.as_bytes())?;
    Ok(planned.summary())
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

        assert!(error.to_string().contains("is outside repository root"));
        assert_eq!(fs::read_to_string(&outside_file).unwrap(), "alpha");
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
    fn bulk_duplicate_target_paths_are_refused_before_writing() {
        let root = test_root("bulk_duplicate_target_paths_are_refused_before_writing");
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
                path: Path::new("./sample.txt"),
                old: "gamma",
                new: "GAMMA",
                expected_sha256: None,
            },
        ];

        let error = plan_bulk_replace_exact_in_root(&root, &entries).unwrap_err();

        assert!(error.to_string().contains("duplicate target path"));
        assert_eq!(fs::read_to_string(file).unwrap(), "alpha beta gamma");
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
