//! Server adapters that bracket mutations with durable receipt evidence.

use contextpatch_core::fs::receipt::{self, Outcome};

use crate::tools::common::normalize_repo_relative_path;

/// Digest one repository file for receipt evidence, through the repository's own authority.
///
/// Absence is not a failure: a receipt records the before state of a file that may not exist yet. A symlink
/// is reported as absent rather than followed, which matches the uniform no-follow rule the rest of the file
/// surface applies, and replaces reading a joined pathname with `std::fs`.
fn digest_in_root(
    root: contextpatch_core::git::RepositoryRoot<'_>,
    relative: &str,
) -> Result<Option<String>, String> {
    use contextpatch_core::fs::rooted::{self, RootedEntryKind};

    match rooted::entry_kind(root, relative) {
        Ok(Some(RootedEntryKind::RegularFile)) => rooted::sha256(root, relative)
            .map(Some)
            .map_err(|error| format!("failed to read `{relative}` for receipt evidence: {error}")),
        Ok(_) => Ok(None),
        Err(error) => Err(format!(
            "failed to read `{relative}` for receipt evidence: {error}"
        )),
    }
}

fn settlement_error(tool: &str, error: impl std::fmt::Display) -> String {
    format!(
        "{tool} receipt settlement failed after the mutation ran: {error}; inspect current state \
         with read_write_receipts and file_info before retrying"
    )
}

pub(crate) fn recorded<'a, T>(
    repo_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    tool: &str,
    relative_path: &str,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let normalized = normalize_repo_relative_path(tool, relative_path)?;
    let root = repo_root.into();
    let before = digest_in_root(root, &normalized).map_err(|error| format!("{tool} refused: {error}"))?;
    let id = receipt::begin_file(root, tool, &normalized, before.as_deref())
        .map_err(|error| format!("{tool} refused before mutation: {error}"))?;

    let result = work();
    let after = digest_in_root(root, &normalized);
    let outcome = if result.is_ok() {
        Outcome::Applied
    } else {
        match &after {
            Ok(after) if *after == before => Outcome::Refused,
            Ok(_) | Err(_) => Outcome::Unknown,
        }
    };
    let settlement = match &after {
        Ok(after) => receipt::settle_file(root, &id, outcome, after.as_deref()),
        Err(_) => receipt::settle_file(root, &id, outcome, None),
    };

    let mut auxiliary_errors = Vec::new();
    if let Err(error) = after {
        auxiliary_errors.push(format!(
            "{tool} could not collect after-state receipt evidence: {error}"
        ));
    }
    if let Err(error) = settlement {
        auxiliary_errors.push(settlement_error(tool, error));
    }
    combine_result(result, auxiliary_errors)
}

pub(crate) fn begin_file_batch<'a>(
    repo_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    tool: &str,
    relative_paths: &[String],
) -> Result<receipt::FileReceiptBatch, String> {
    let normalized = relative_paths
        .iter()
        .map(|path| normalize_repo_relative_path(tool, path))
        .collect::<Result<Vec<_>, _>>()?;
    receipt::begin_file_batch(repo_root, tool, &normalized)
        .map_err(|error| format!("{tool} refused before mutation: {error}"))
}

pub(crate) fn recorded_in_batch<'a, T>(
    batch: &mut receipt::FileReceiptBatch,
    repo_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    tool: &str,
    relative_path: &str,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let normalized = normalize_repo_relative_path(tool, relative_path)?;
    let root = repo_root.into();
    let before = digest_in_root(root, &normalized).map_err(|error| format!("{tool} refused: {error}"))?;
    let id = batch
        .begin_file(&normalized, before.as_deref())
        .map_err(|error| format!("{tool} refused before mutation: {error}"))?;

    let result = work();
    let after = digest_in_root(root, &normalized);
    let outcome = if result.is_ok() {
        Outcome::Applied
    } else {
        match &after {
            Ok(after) if *after == before => Outcome::Refused,
            Ok(_) | Err(_) => Outcome::Unknown,
        }
    };
    let settlement = match &after {
        Ok(after) => batch.settle_file(&id, outcome, after.as_deref()),
        Err(_) => batch.settle_file(&id, outcome, None),
    };

    let mut auxiliary_errors = Vec::new();
    if let Err(error) = after {
        auxiliary_errors.push(format!(
            "{tool} could not collect after-state receipt evidence: {error}"
        ));
    }
    if let Err(error) = settlement {
        auxiliary_errors.push(settlement_error(tool, error));
    }
    combine_result(result, auxiliary_errors)
}

/// Journal a `refused` receipt for every target of a batch whose validation failed.
///
/// Batch validation runs to completion before the first write, so a refusal there is the one case
/// where a caller learns nothing from the journal: the tool was invoked, it touched nothing, and it
/// left no evidence of either fact. Recording the attempt makes a refusal indistinguishable from a
/// successful no-op only in outcome, never in whether it happened.
///
/// Semantics match the single-file path exactly. Each receipt captures the digest before and after and
/// reports `unknown` rather than `refused` when those differ, because an external writer during
/// validation means this tool cannot claim the file is untouched. Receipts follow the order of
/// `relative_paths`, so a caller that sorts its targets gets a deterministic journal.
///
/// Paths must already be repository-relative and normalized. Auxiliary problems are returned rather
/// than raised, so a journal failure never masks the validation refusal that caused it.
pub(crate) fn record_refused_batch<'a>(
    repo_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    tool: &str,
    relative_paths: &[String],
) -> Vec<String> {
    let mut auxiliary_errors = Vec::new();
    if relative_paths.is_empty() {
        return auxiliary_errors;
    }

    let root = repo_root.into();
    let mut batch = match receipt::begin_file_batch(root, tool, relative_paths) {
        Ok(batch) => batch,
        Err(error) => {
            auxiliary_errors.push(format!(
                "{tool} could not reserve refused receipt capacity: {error}"
            ));
            return auxiliary_errors;
        }
    };

    for normalized in relative_paths {
        let before = match digest_in_root(root, normalized) {
            Ok(before) => before,
            Err(error) => {
                auxiliary_errors.push(format!(
                    "{tool} could not collect before-state receipt evidence for `{normalized}`: {error}"
                ));
                continue;
            }
        };
        let id = match batch.begin_file(normalized, before.as_deref()) {
            Ok(id) => id,
            Err(error) => {
                auxiliary_errors.push(format!(
                    "{tool} could not journal a refused receipt for `{normalized}`: {error}"
                ));
                continue;
            }
        };

        let after = digest_in_root(root, normalized);
        let outcome = match &after {
            Ok(after) if *after == before => Outcome::Refused,
            Ok(_) | Err(_) => Outcome::Unknown,
        };
        let settlement = match &after {
            Ok(after) => batch.settle_file(&id, outcome, after.as_deref()),
            Err(_) => batch.settle_file(&id, outcome, None),
        };

        if let Err(error) = after {
            auxiliary_errors.push(format!(
                "{tool} could not collect after-state receipt evidence for `{normalized}`: {error}"
            ));
        }
        if let Err(error) = settlement {
            auxiliary_errors.push(settlement_error(tool, error));
        }
    }

    auxiliary_errors
}

pub(crate) fn recorded_deletions<'a, T>(
    repo_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    tool: &str,
    relative_paths: &[String],
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let root = repo_root.into();
    let mut planned = Vec::with_capacity(relative_paths.len());

    for relative in relative_paths {
        let normalized = normalize_repo_relative_path(tool, relative)?;
        let before =
            digest_in_root(root, &normalized).map_err(|error| format!("{tool} refused: {error}"))?;
        match receipt::begin_file(root, tool, &normalized, before.as_deref()) {
            Ok(id) => planned.push((normalized, before, id)),
            Err(error) => {
                let mut settlement_errors = Vec::new();
                for (_, before, id) in &planned {
                    if let Err(settle_error) =
                        receipt::settle_file(root, id, Outcome::Refused, before.as_deref())
                    {
                        settlement_errors.push(settlement_error(tool, settle_error));
                    }
                }
                let mut message = format!("{tool} refused before mutation: {error}");
                if !settlement_errors.is_empty() {
                    message.push('\n');
                    message.push_str(&settlement_errors.join("\n"));
                }
                return Err(message);
            }
        }
    }

    let result = work();
    let work_succeeded = result.is_ok();
    let mut auxiliary_errors = Vec::new();
    for (relative, before, id) in planned {
        let after = digest_in_root(root, &relative);
        let outcome = if work_succeeded {
            Outcome::Applied
        } else {
            match &after {
                Ok(after) if *after == before => Outcome::Refused,
                Ok(_) | Err(_) => Outcome::Unknown,
            }
        };
        let settlement = match &after {
            Ok(after) => receipt::settle_file(root, &id, outcome, after.as_deref()),
            Err(_) => receipt::settle_file(root, &id, outcome, None),
        };
        if let Err(error) = after {
            auxiliary_errors.push(format!(
                "{tool} could not collect after-state receipt evidence for `{relative}`: {error}"
            ));
        }
        if let Err(error) = settlement {
            auxiliary_errors.push(settlement_error(tool, error));
        }
    }
    combine_result(result, auxiliary_errors)
}

pub(crate) fn begin_git<'a>(
    repo_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    tool: &str,
    before_git_head: &str,
) -> Result<String, String> {
    receipt::begin_git(repo_root, tool, before_git_head)
        .map_err(|error| format!("{tool} refused before Git mutation: {error}"))
}

pub(crate) fn settle_git<'a, T>(
    repo_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    tool: &str,
    id: &str,
    before_git_head: &str,
    after_git_head: Result<String, String>,
    result: Result<T, String>,
) -> Result<T, String> {
    let outcome = if result.is_ok() {
        Outcome::Applied
    } else {
        match &after_git_head {
            Ok(after) if after == before_git_head => Outcome::Refused,
            Ok(_) | Err(_) => Outcome::Unknown,
        }
    };
    let settlement = receipt::settle_git(repo_root, id, outcome, after_git_head.as_deref().ok());

    let mut auxiliary_errors = Vec::new();
    if let Err(error) = after_git_head {
        auxiliary_errors.push(format!(
            "{tool} could not collect the post-mutation Git HEAD for receipt evidence: {error}"
        ));
    }
    if let Err(error) = settlement {
        auxiliary_errors.push(format!(
            "{tool} Git receipt settlement failed after mutation: {error}; inspect HEAD and \
             read_write_receipts before retrying"
        ));
    }
    combine_result(result, auxiliary_errors)
}

fn combine_result<T>(
    result: Result<T, String>,
    auxiliary_errors: Vec<String>,
) -> Result<T, String> {
    if auxiliary_errors.is_empty() {
        return result;
    }

    let auxiliary = auxiliary_errors.join("\n");
    match result {
        Ok(_) => Err(auxiliary),
        Err(error) => Err(format!("{error}\n{auxiliary}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-journal-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn failed_unchanged_operation_is_refused() {
        let root = temp_repo("refused");
        fs::write(root.join("sample.txt"), "before").unwrap();
        let result = recorded(&root, "replace_exact", "sample.txt", || {
            Err::<(), _>("deliberate refusal".to_string())
        });
        assert!(result.is_err());
        let receipt = receipt::all(&root).unwrap().remove(0);
        assert_eq!(receipt.outcome.as_deref(), Some("refused"));
    }

    #[test]
    fn failed_operation_with_changed_state_is_unknown() {
        let root = temp_repo("unknown");
        let target = root.join("sample.txt");
        fs::write(&target, "before").unwrap();
        let result = recorded(&root, "replace_exact", "sample.txt", || {
            fs::write(&target, "changed").unwrap();
            Err::<(), _>("failure after a state change".to_string())
        });
        assert!(result.is_err());
        let receipt = receipt::all(&root).unwrap().remove(0);
        assert_eq!(receipt.outcome.as_deref(), Some("unknown"));
    }

    #[test]
    fn a_refused_batch_journals_one_unchanged_receipt_per_target() {
        let root = temp_repo("refused_batch");
        fs::write(root.join("one.txt"), "first").unwrap();
        fs::write(root.join("two.txt"), "second").unwrap();
        // Sorted and deduplicated by the caller, which is what makes the journal deterministic.
        let targets = vec!["one.txt".to_string(), "two.txt".to_string()];

        let auxiliary = record_refused_batch(&root, "bulk_replace_exact", &targets);

        assert!(auxiliary.is_empty(), "{auxiliary:?}");
        let receipts = receipt::all(&root).unwrap();
        assert_eq!(receipts.len(), 2);
        for entry in &receipts {
            assert_eq!(entry.outcome.as_deref(), Some("refused"));
            assert!(!entry.interrupted());
            // Nothing was written, so the digests must bracket an unchanged file.
            assert!(entry.before_sha256.is_some());
            assert_eq!(entry.before_sha256, entry.after_sha256);
        }
        let mut paths: Vec<&str> = receipts
            .iter()
            .filter_map(|entry| entry.path.as_deref())
            .collect();
        paths.sort_unstable();
        assert_eq!(paths, ["one.txt", "two.txt"]);
        assert_eq!(fs::read_to_string(root.join("one.txt")).unwrap(), "first");
        assert_eq!(fs::read_to_string(root.join("two.txt")).unwrap(), "second");
    }

    #[test]
    fn a_refused_batch_with_no_targets_journals_nothing() {
        let root = temp_repo("refused_batch_empty");

        let auxiliary = record_refused_batch(&root, "bulk_replace_exact", &[]);

        assert!(auxiliary.is_empty(), "{auxiliary:?}");
        assert!(receipt::all(&root).unwrap().is_empty());
    }
}
