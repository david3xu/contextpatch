//! Server adapters that bracket mutations with durable receipt evidence.

use std::io::ErrorKind;
use std::path::Path;

use contextpatch_core::fs::hash::sha256_bytes;
use contextpatch_core::fs::receipt::{self, Outcome};

use crate::tools::common::normalize_repo_relative_path;

fn digest_of(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(sha256_bytes(&bytes))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "failed to read `{}` for receipt evidence: {error}",
            path.display()
        )),
    }
}

fn settlement_error(tool: &str, error: impl std::fmt::Display) -> String {
    format!(
        "{tool} receipt settlement failed after the mutation ran: {error}; inspect current state \
         with read_write_receipts and file_info before retrying"
    )
}

pub(crate) fn recorded<T>(
    repo_root: &Path,
    tool: &str,
    relative_path: &str,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let normalized = normalize_repo_relative_path(tool, relative_path)?;
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("{tool} refused: failed to resolve repo root: {error}"))?;
    let target = root.join(&normalized);
    let before = digest_of(&target).map_err(|error| format!("{tool} refused: {error}"))?;
    let id = receipt::begin_file(&root, tool, &normalized, before.as_deref())
        .map_err(|error| format!("{tool} refused before mutation: {error}"))?;

    let result = work();
    let after = digest_of(&target);
    let outcome = if result.is_ok() {
        Outcome::Applied
    } else {
        match &after {
            Ok(after) if *after == before => Outcome::Refused,
            Ok(_) | Err(_) => Outcome::Unknown,
        }
    };
    let settlement = match &after {
        Ok(after) => receipt::settle_file(&root, &id, outcome, after.as_deref()),
        Err(_) => receipt::settle_file(&root, &id, outcome, None),
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

pub(crate) fn recorded_deletions<T>(
    repo_root: &Path,
    tool: &str,
    relative_paths: &[String],
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("{tool} refused: failed to resolve repo root: {error}"))?;
    let mut planned = Vec::with_capacity(relative_paths.len());

    for relative in relative_paths {
        let normalized = normalize_repo_relative_path(tool, relative)?;
        let before = digest_of(&root.join(&normalized))
            .map_err(|error| format!("{tool} refused: {error}"))?;
        match receipt::begin_file(&root, tool, &normalized, before.as_deref()) {
            Ok(id) => planned.push((normalized, before, id)),
            Err(error) => {
                let mut settlement_errors = Vec::new();
                for (_, before, id) in &planned {
                    if let Err(settle_error) =
                        receipt::settle_file(&root, id, Outcome::Refused, before.as_deref())
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
        let after = digest_of(&root.join(&relative));
        let outcome = if work_succeeded {
            Outcome::Applied
        } else {
            match &after {
                Ok(after) if *after == before => Outcome::Refused,
                Ok(_) | Err(_) => Outcome::Unknown,
            }
        };
        let settlement = match &after {
            Ok(after) => receipt::settle_file(&root, &id, outcome, after.as_deref()),
            Err(_) => receipt::settle_file(&root, &id, outcome, None),
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

pub(crate) fn begin_git(
    repo_root: &Path,
    tool: &str,
    before_git_head: &str,
) -> Result<String, String> {
    receipt::begin_git(repo_root, tool, before_git_head)
        .map_err(|error| format!("{tool} refused before Git mutation: {error}"))
}

pub(crate) fn settle_git<T>(
    repo_root: &Path,
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
}
