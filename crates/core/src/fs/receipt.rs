//! Durable evidence for mutations whose transport reply may be interrupted.
//!
//! Each attempt appends a `begin` record before mutation and a `settle` record afterwards. A begin
//! without a matching settle is therefore an interrupted operation whose effect must be recovered
//! from the recorded file digest or Git HEAD.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde_json::{json, Value};

use crate::error::ContextPatchError;
use crate::fs::atomic_write::write_atomic;
use crate::fs::scratch::ensure_scratch_root;

pub const RECEIPT_FILENAME: &str = "write-receipts.jsonl";
const RECEIPT_LOCK_FILENAME: &str = "write-receipts.lock";
pub const DEFAULT_RECENT_LIMIT: usize = 25;
pub const MAX_RECENT_LIMIT: usize = 100;
pub const MAX_JOURNAL_BYTES: u64 = 512 * 1024;

const RECORD_BEGIN: &str = "begin";
const RECORD_SETTLE: &str = "settle";
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    Refused,
    Unknown,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Applied => "applied",
            Outcome::Refused => "refused",
            Outcome::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub id: String,
    pub tool: String,
    pub path: Option<String>,
    pub before_sha256: Option<String>,
    pub after_sha256: Option<String>,
    pub before_git_head: Option<String>,
    pub after_git_head: Option<String>,
    pub outcome: Option<String>,
    pub began_at_ms: u128,
    pub settled_at_ms: Option<u128>,
}

impl Receipt {
    pub fn interrupted(&self) -> bool {
        self.outcome.is_none()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "tool": self.tool,
            "path": self.path,
            "before_sha256": self.before_sha256,
            "after_sha256": self.after_sha256,
            "before_git_head": self.before_git_head,
            "after_git_head": self.after_git_head,
            "outcome": self.outcome.clone().unwrap_or_else(|| "interrupted".to_string()),
            "interrupted": self.interrupted(),
            "began_at_ms": self.began_at_ms.to_string(),
            "settled_at_ms": self.settled_at_ms.map(|value| value.to_string()),
        })
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default()
}

pub fn journal_path(repo_root: &Path) -> Result<PathBuf, ContextPatchError> {
    Ok(ensure_scratch_root(repo_root)?.join(RECEIPT_FILENAME))
}

fn append(path: &Path, record: &Value) -> Result<(), ContextPatchError> {
    let mut serialized = serde_json::to_vec(record).map_err(|error| {
        ContextPatchError::new(format!("write receipt could not be serialized: {error}"))
    })?;
    serialized.push(b'\n');
    let mut handle = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| {
            ContextPatchError::new(format!(
                "write receipt journal could not be opened: {error}"
            ))
        })?;
    handle.write_all(&serialized).map_err(|error| {
        ContextPatchError::new(format!("write receipt could not be appended: {error}"))
    })?;
    handle.flush().map_err(|error| {
        ContextPatchError::new(format!("write receipt could not be flushed: {error}"))
    })?;
    handle.sync_data().map_err(|error| {
        ContextPatchError::new(format!("write receipt could not be persisted: {error}"))
    })
}

fn lock_file(repo_root: &Path) -> Result<File, ContextPatchError> {
    let path = ensure_scratch_root(repo_root)?.join(RECEIPT_LOCK_FILENAME);
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            ContextPatchError::new(format!("write receipt lock could not be opened: {error}"))
        })
}

fn with_exclusive_lock<T>(
    repo_root: &Path,
    work: impl FnOnce() -> Result<T, ContextPatchError>,
) -> Result<T, ContextPatchError> {
    let lock = lock_file(repo_root)?;
    lock.lock_exclusive().map_err(|error| {
        ContextPatchError::new(format!(
            "write receipt journal could not be locked: {error}"
        ))
    })?;
    let result = work();
    FileExt::unlock(&lock).map_err(|error| {
        ContextPatchError::new(format!(
            "write receipt journal could not be unlocked: {error}"
        ))
    })?;
    result
}

fn with_shared_lock<T>(
    repo_root: &Path,
    work: impl FnOnce() -> Result<T, ContextPatchError>,
) -> Result<T, ContextPatchError> {
    let lock = lock_file(repo_root)?;
    lock.lock_shared().map_err(|error| {
        ContextPatchError::new(format!(
            "write receipt journal could not be locked for reading: {error}"
        ))
    })?;
    let result = work();
    FileExt::unlock(&lock).map_err(|error| {
        ContextPatchError::new(format!(
            "write receipt journal could not be unlocked: {error}"
        ))
    })?;
    result
}

fn begin_value(receipt: &Receipt) -> Value {
    json!({
        "record": RECORD_BEGIN,
        "id": receipt.id,
        "tool": receipt.tool,
        "path": receipt.path,
        "before_sha256": receipt.before_sha256,
        "before_git_head": receipt.before_git_head,
        "at_ms": receipt.began_at_ms.to_string(),
    })
}

fn rotate_if_oversized(path: &Path) -> Result<(), ContextPatchError> {
    let oversized = match std::fs::metadata(path) {
        Ok(metadata) => metadata.len() > MAX_JOURNAL_BYTES,
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => {
            return Err(ContextPatchError::new(format!(
                "write receipt journal metadata could not be read: {error}"
            )))
        }
    };
    if !oversized {
        return Ok(());
    }

    let unsettled = all_from_path(path)?
        .into_iter()
        .filter(Receipt::interrupted)
        .collect::<Vec<_>>();
    let mut preserved = Vec::new();
    for receipt in unsettled {
        serde_json::to_writer(&mut preserved, &begin_value(&receipt)).map_err(|error| {
            ContextPatchError::new(format!(
                "write receipt journal could not preserve interrupted evidence: {error}"
            ))
        })?;
        preserved.push(b'\n');
    }
    write_atomic(path, &preserved).map_err(|error| {
        ContextPatchError::new(format!(
            "write receipt journal could not be rotated: {error}"
        ))
    })
}

fn begin_record(
    repo_root: &Path,
    tool: &str,
    path: Option<&str>,
    before_sha256: Option<&str>,
    before_git_head: Option<&str>,
) -> Result<String, ContextPatchError> {
    let journal = journal_path(repo_root)?;
    with_exclusive_lock(repo_root, || {
        rotate_if_oversized(&journal)?;
        let began_at_ms = now_ms();
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let id = format!("{began_at_ms:x}-{:x}-{sequence:x}", std::process::id());
        let receipt = Receipt {
            id: id.clone(),
            tool: tool.to_string(),
            path: path.map(str::to_string),
            before_sha256: before_sha256.map(str::to_string),
            after_sha256: None,
            before_git_head: before_git_head.map(str::to_string),
            after_git_head: None,
            outcome: None,
            began_at_ms,
            settled_at_ms: None,
        };
        append(&journal, &begin_value(&receipt))?;
        Ok(id)
    })
}

pub fn begin_file(
    repo_root: &Path,
    tool: &str,
    path: &str,
    before_sha256: Option<&str>,
) -> Result<String, ContextPatchError> {
    begin_record(repo_root, tool, Some(path), before_sha256, None)
}

pub fn begin_git(
    repo_root: &Path,
    tool: &str,
    before_git_head: &str,
) -> Result<String, ContextPatchError> {
    begin_record(repo_root, tool, None, None, Some(before_git_head))
}

fn settle_record(
    repo_root: &Path,
    id: &str,
    outcome: Outcome,
    after_sha256: Option<&str>,
    after_git_head: Option<&str>,
) -> Result<(), ContextPatchError> {
    let journal = journal_path(repo_root)?;
    with_exclusive_lock(repo_root, || {
        append(
            &journal,
            &json!({
                "record": RECORD_SETTLE,
                "id": id,
                "outcome": outcome.as_str(),
                "after_sha256": after_sha256,
                "after_git_head": after_git_head,
                "at_ms": now_ms().to_string(),
            }),
        )
    })
}

pub fn settle_file(
    repo_root: &Path,
    id: &str,
    outcome: Outcome,
    after_sha256: Option<&str>,
) -> Result<(), ContextPatchError> {
    settle_record(repo_root, id, outcome, after_sha256, None)
}

pub fn settle_git(
    repo_root: &Path,
    id: &str,
    outcome: Outcome,
    after_git_head: Option<&str>,
) -> Result<(), ContextPatchError> {
    settle_record(repo_root, id, outcome, None, after_git_head)
}

fn parse_ms(record: &Value, key: &str) -> u128 {
    record
        .get(key)
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<u128>().ok())
        .unwrap_or_default()
}

fn optional_text(record: &Value, key: &str) -> Option<String> {
    record.get(key).and_then(Value::as_str).map(str::to_string)
}

fn all_from_path(journal: &Path) -> Result<Vec<Receipt>, ContextPatchError> {
    let handle = match File::open(journal) {
        Ok(handle) => handle,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ContextPatchError::new(format!(
                "write receipt journal could not be opened for reading: {error}"
            )))
        }
    };

    let mut order = Vec::new();
    let mut receipts = HashMap::new();
    for line in BufReader::new(handle).lines() {
        let line = line.map_err(|error| {
            ContextPatchError::new(format!("write receipt journal could not be read: {error}"))
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let Some(id) = optional_text(&record, "id") else {
            continue;
        };
        match record.get("record").and_then(Value::as_str) {
            Some(RECORD_BEGIN) => {
                order.push(id.clone());
                receipts.insert(
                    id.clone(),
                    Receipt {
                        id,
                        tool: optional_text(&record, "tool").unwrap_or_default(),
                        path: optional_text(&record, "path"),
                        before_sha256: optional_text(&record, "before_sha256"),
                        after_sha256: None,
                        before_git_head: optional_text(&record, "before_git_head"),
                        after_git_head: None,
                        outcome: None,
                        began_at_ms: parse_ms(&record, "at_ms"),
                        settled_at_ms: None,
                    },
                );
            }
            Some(RECORD_SETTLE) => {
                if let Some(receipt) = receipts.get_mut(&id) {
                    receipt.outcome = optional_text(&record, "outcome");
                    receipt.after_sha256 = optional_text(&record, "after_sha256");
                    receipt.after_git_head = optional_text(&record, "after_git_head");
                    receipt.settled_at_ms = Some(parse_ms(&record, "at_ms"));
                }
            }
            _ => {}
        }
    }

    Ok(order
        .into_iter()
        .filter_map(|id| receipts.remove(&id))
        .collect())
}

pub fn all(repo_root: &Path) -> Result<Vec<Receipt>, ContextPatchError> {
    let journal = journal_path(repo_root)?;
    with_shared_lock(repo_root, || all_from_path(&journal))
}

fn validate_limit(limit: usize) -> Result<(), ContextPatchError> {
    if !(1..=MAX_RECENT_LIMIT).contains(&limit) {
        return Err(ContextPatchError::invalid(format!(
            "receipt limit must be between 1 and {MAX_RECENT_LIMIT}"
        )));
    }
    Ok(())
}

pub fn recent(repo_root: &Path, limit: usize) -> Result<Vec<Receipt>, ContextPatchError> {
    validate_limit(limit)?;
    let mut receipts = all(repo_root)?;
    receipts.reverse();
    receipts.truncate(limit);
    Ok(receipts)
}

pub fn unsettled(repo_root: &Path, limit: usize) -> Result<Vec<Receipt>, ContextPatchError> {
    validate_limit(limit)?;
    Ok(all(repo_root)?
        .into_iter()
        .rev()
        .filter(Receipt::interrupted)
        .take(limit)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-receipt-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn an_absent_journal_reports_no_attempts() {
        let root = temp_repo("absent");
        assert!(all(&root).unwrap().is_empty());
        assert!(unsettled(&root, DEFAULT_RECENT_LIMIT).unwrap().is_empty());
    }

    #[test]
    fn file_evidence_folds_begin_and_settle_records() {
        let root = temp_repo("file");
        let id = begin_file(&root, "replace_exact", "src/lib.rs", Some("aaa")).unwrap();
        settle_file(&root, &id, Outcome::Applied, Some("bbb")).unwrap();

        let receipt = all(&root).unwrap().remove(0);
        assert_eq!(receipt.path.as_deref(), Some("src/lib.rs"));
        assert_eq!(receipt.before_sha256.as_deref(), Some("aaa"));
        assert_eq!(receipt.after_sha256.as_deref(), Some("bbb"));
        assert_eq!(receipt.outcome.as_deref(), Some("applied"));
        assert!(!receipt.interrupted());
    }

    #[test]
    fn git_evidence_records_head_transition() {
        let root = temp_repo("git");
        let id = begin_git(&root, "git_commit_exact", "before").unwrap();
        settle_git(&root, &id, Outcome::Applied, Some("after")).unwrap();

        let receipt = all(&root).unwrap().remove(0);
        assert_eq!(receipt.path, None);
        assert_eq!(receipt.before_git_head.as_deref(), Some("before"));
        assert_eq!(receipt.after_git_head.as_deref(), Some("after"));
    }

    #[test]
    fn unknown_outcome_is_settled_but_requires_reconciliation() {
        let root = temp_repo("unknown");
        let id = begin_file(&root, "replace_exact", "src/lib.rs", Some("aaa")).unwrap();
        settle_file(&root, &id, Outcome::Unknown, Some("bbb")).unwrap();

        let receipt = all(&root).unwrap().remove(0);
        assert_eq!(receipt.outcome.as_deref(), Some("unknown"));
        assert!(!receipt.interrupted());
    }

    #[test]
    fn interrupted_queries_are_newest_first_and_bounded() {
        let root = temp_repo("interrupted");
        for index in 0..5 {
            begin_file(&root, "replace_exact", &format!("file{index}.txt"), None).unwrap();
        }
        let receipts = unsettled(&root, 2).unwrap();
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0].path.as_deref(), Some("file4.txt"));
        assert_eq!(receipts[1].path.as_deref(), Some("file3.txt"));
    }

    #[test]
    fn recent_rejects_unbounded_queries() {
        let root = temp_repo("limit");
        assert!(recent(&root, 0)
            .unwrap_err()
            .to_string()
            .contains("between 1"));
        assert!(unsettled(&root, MAX_RECENT_LIMIT + 1)
            .unwrap_err()
            .to_string()
            .contains("between 1"));
    }

    #[test]
    fn malformed_lines_are_skipped() {
        let root = temp_repo("malformed");
        let id = begin_file(&root, "replace_exact", "good.txt", None).unwrap();
        settle_file(&root, &id, Outcome::Applied, None).unwrap();
        let journal = journal_path(&root).unwrap();
        writeln!(
            OpenOptions::new().append(true).open(&journal).unwrap(),
            "this is not json"
        )
        .unwrap();
        assert_eq!(all(&root).unwrap().len(), 1);
    }

    #[test]
    fn rotation_preserves_interrupted_evidence() {
        let root = temp_repo("rotation");
        begin_file(&root, "replace_exact", "interrupted.txt", Some("aaa")).unwrap();
        let journal = journal_path(&root).unwrap();
        let mut handle = OpenOptions::new().append(true).open(&journal).unwrap();
        writeln!(handle, "{}", "x".repeat(MAX_JOURNAL_BYTES as usize)).unwrap();
        drop(handle);

        begin_file(&root, "replace_exact", "new.txt", Some("bbb")).unwrap();

        let unsettled = unsettled(&root, MAX_RECENT_LIMIT).unwrap();
        assert_eq!(unsettled.len(), 2);
        assert!(unsettled
            .iter()
            .any(|receipt| receipt.path.as_deref() == Some("interrupted.txt")));
    }

    #[test]
    fn the_journal_lives_outside_the_repository() {
        let root = temp_repo("placement");
        let id = begin_file(&root, "replace_exact", "x.txt", None).unwrap();
        settle_file(&root, &id, Outcome::Applied, None).unwrap();
        let journal = journal_path(&root).unwrap();
        assert!(journal.exists());
        assert!(!journal.starts_with(&root));
    }

    #[test]
    fn concurrent_writers_keep_complete_receipts() {
        let root = temp_repo("concurrent");
        let mut writers = Vec::new();
        for index in 0..16 {
            let root = root.clone();
            writers.push(std::thread::spawn(move || {
                let path = format!("file{index}.txt");
                let id = begin_file(&root, "replace_exact", &path, None).unwrap();
                settle_file(&root, &id, Outcome::Applied, Some("after")).unwrap();
            }));
        }
        for writer in writers {
            writer.join().unwrap();
        }

        let receipts = all(&root).unwrap();
        assert_eq!(receipts.len(), 16);
        assert!(receipts.iter().all(|receipt| !receipt.interrupted()));
    }
}
