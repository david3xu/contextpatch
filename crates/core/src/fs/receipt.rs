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
const MAX_RECEIPT_ID_BYTES: usize = 80;
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

pub fn journal_path<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
) -> Result<PathBuf, ContextPatchError> {
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

fn lock_file<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
) -> Result<File, ContextPatchError> {
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

fn with_exclusive_lock<'a, T>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
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

fn with_shared_lock<'a, T>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
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

fn settle_value(
    id: &str,
    outcome: Outcome,
    after_sha256: Option<&str>,
    after_git_head: Option<&str>,
    at_ms: u128,
) -> Value {
    json!({
        "record": RECORD_SETTLE,
        "id": id,
        "outcome": outcome.as_str(),
        "after_sha256": after_sha256,
        "after_git_head": after_git_head,
        "at_ms": at_ms.to_string(),
    })
}

fn journal_bytes(path: &Path) -> Result<u64, ContextPatchError> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(0),
        Err(error) => Err(ContextPatchError::new(format!(
            "write receipt journal metadata could not be read: {error}"
        ))),
    }
}

fn rotate(path: &Path) -> Result<(), ContextPatchError> {
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

fn rotate_if_oversized(path: &Path) -> Result<(), ContextPatchError> {
    let oversized = match journal_bytes(path) {
        Ok(bytes) => bytes > MAX_JOURNAL_BYTES,
        Err(error) => {
            return Err(error);
        }
    };
    if !oversized {
        return Ok(());
    }

    rotate(path)
}

fn serialized_record_bytes(record: &Value) -> Result<u64, ContextPatchError> {
    let bytes = serde_json::to_vec(record).map_err(|error| {
        ContextPatchError::new(format!(
            "write receipt reservation could not be serialized: {error}"
        ))
    })?;
    u64::try_from(bytes.len())
        .ok()
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| ContextPatchError::new("write receipt reservation size overflow"))
}

fn file_batch_reservation(tool: &str, paths: &[String]) -> Result<u64, ContextPatchError> {
    let id = "f".repeat(MAX_RECEIPT_ID_BYTES);
    let digest = "f".repeat(64);
    let mut total = 0u64;
    for path in paths {
        let receipt = Receipt {
            id: id.clone(),
            tool: tool.to_string(),
            path: Some(path.clone()),
            before_sha256: Some(digest.clone()),
            after_sha256: None,
            before_git_head: None,
            after_git_head: None,
            outcome: None,
            began_at_ms: u128::MAX,
            settled_at_ms: None,
        };
        let begin = serialized_record_bytes(&begin_value(&receipt))?;
        let settle = serialized_record_bytes(&settle_value(
            &id,
            Outcome::Refused,
            Some(&digest),
            None,
            u128::MAX,
        ))?;
        total = total
            .checked_add(begin)
            .and_then(|bytes| bytes.checked_add(settle))
            .ok_or_else(|| ContextPatchError::new("write receipt reservation size overflow"))?;
    }
    Ok(total)
}

fn reserve_file_batch(path: &Path, reserved_bytes: u64) -> Result<(), ContextPatchError> {
    if reserved_bytes > MAX_JOURNAL_BYTES {
        return Err(ContextPatchError::new(format!(
            "write receipt batch requires {reserved_bytes} bytes; journal capacity is \
             {MAX_JOURNAL_BYTES} bytes"
        )));
    }

    let current_bytes = journal_bytes(path)?;
    if current_bytes
        .checked_add(reserved_bytes)
        .is_none_or(|total| total > MAX_JOURNAL_BYTES)
    {
        rotate(path)?;
    }

    let retained_bytes = journal_bytes(path)?;
    if retained_bytes
        .checked_add(reserved_bytes)
        .is_none_or(|total| total > MAX_JOURNAL_BYTES)
    {
        return Err(ContextPatchError::new(format!(
            "write receipt journal has {retained_bytes} bytes of interrupted evidence and cannot \
             reserve {reserved_bytes} bytes for this batch without exceeding \
             {MAX_JOURNAL_BYTES} bytes"
        )));
    }
    Ok(())
}

fn append_begin_record(
    journal: &Path,
    tool: &str,
    path: Option<&str>,
    before_sha256: Option<&str>,
    before_git_head: Option<&str>,
) -> Result<String, ContextPatchError> {
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
    append(journal, &begin_value(&receipt))?;
    Ok(id)
}

fn append_settle_record(
    journal: &Path,
    id: &str,
    outcome: Outcome,
    after_sha256: Option<&str>,
    after_git_head: Option<&str>,
) -> Result<(), ContextPatchError> {
    append(
        journal,
        &settle_value(id, outcome, after_sha256, after_git_head, now_ms()),
    )
}

fn begin_record<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    tool: &str,
    path: Option<&str>,
    before_sha256: Option<&str>,
    before_git_head: Option<&str>,
) -> Result<String, ContextPatchError> {
    let repo_root = repo_root.into();
    let journal = journal_path(repo_root)?;
    with_exclusive_lock(repo_root, || {
        rotate_if_oversized(&journal)?;
        append_begin_record(&journal, tool, path, before_sha256, before_git_head)
    })
}

pub fn begin_file<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    tool: &str,
    path: &str,
    before_sha256: Option<&str>,
) -> Result<String, ContextPatchError> {
    begin_record(repo_root, tool, Some(path), before_sha256, None)
}

pub fn begin_git<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    tool: &str,
    before_git_head: &str,
) -> Result<String, ContextPatchError> {
    begin_record(repo_root, tool, None, None, Some(before_git_head))
}

/// A bounded file-receipt batch that prevents journal rotation between entries.
///
/// The journal lock is held for the batch lifetime. Capacity is reserved before the caller may start
/// mutating, so a later interruption leaves the already-settled prefix beside the interrupted entry.
pub struct FileReceiptBatch {
    journal: PathBuf,
    _lock: File,
    tool: String,
    paths: Vec<String>,
    next_path: usize,
}

pub fn begin_file_batch<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    tool: &str,
    paths: &[String],
) -> Result<FileReceiptBatch, ContextPatchError> {
    if paths.is_empty() {
        return Err(ContextPatchError::new(
            "write receipt batch requires at least one path",
        ));
    }
    let repo_root = repo_root.into();
    let reserved_bytes = file_batch_reservation(tool, paths)?;
    let journal = journal_path(repo_root)?;
    let lock = lock_file(repo_root)?;
    lock.lock_exclusive().map_err(|error| {
        ContextPatchError::new(format!(
            "write receipt journal could not be locked for a batch: {error}"
        ))
    })?;
    reserve_file_batch(&journal, reserved_bytes)?;
    Ok(FileReceiptBatch {
        journal,
        _lock: lock,
        tool: tool.to_string(),
        paths: paths.to_vec(),
        next_path: 0,
    })
}

impl FileReceiptBatch {
    pub fn begin_file(
        &mut self,
        path: &str,
        before_sha256: Option<&str>,
    ) -> Result<String, ContextPatchError> {
        let expected = self.paths.get(self.next_path).ok_or_else(|| {
            ContextPatchError::new("write receipt batch has no remaining reserved paths")
        })?;
        if expected != path {
            return Err(ContextPatchError::new(format!(
                "write receipt batch expected path `{expected}`, got `{path}`"
            )));
        }
        let id = append_begin_record(&self.journal, &self.tool, Some(path), before_sha256, None)?;
        self.next_path += 1;
        Ok(id)
    }

    pub fn settle_file(
        &mut self,
        id: &str,
        outcome: Outcome,
        after_sha256: Option<&str>,
    ) -> Result<(), ContextPatchError> {
        append_settle_record(&self.journal, id, outcome, after_sha256, None)
    }
}

fn settle_record<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    id: &str,
    outcome: Outcome,
    after_sha256: Option<&str>,
    after_git_head: Option<&str>,
) -> Result<(), ContextPatchError> {
    let repo_root = repo_root.into();
    let journal = journal_path(repo_root)?;
    with_exclusive_lock(repo_root, || {
        append_settle_record(&journal, id, outcome, after_sha256, after_git_head)
    })
}

pub fn settle_file<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    id: &str,
    outcome: Outcome,
    after_sha256: Option<&str>,
) -> Result<(), ContextPatchError> {
    settle_record(repo_root, id, outcome, after_sha256, None)
}

pub fn settle_git<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
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

/// Every receipt for this repository, including any written before identity keying.
///
/// Receipts are evidence a caller reaches for after losing an answer, so a keying change must not make the
/// previous ones disappear. The identity-keyed journal is read, and a path-keyed journal left over from
/// before the change is folded in ahead of it so the combined sequence stays oldest-first. Legacy records
/// are read only; nothing new is written there.
pub fn all<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
) -> Result<Vec<Receipt>, ContextPatchError> {
    let repo_root = repo_root.into();
    let journal = journal_path(repo_root)?;
    let legacy = crate::fs::scratch::legacy_scratch_root(repo_root).join(RECEIPT_FILENAME);

    let current = with_shared_lock(repo_root, || all_from_path(&journal))?;
    if legacy == journal || !legacy.is_file() {
        return Ok(current);
    }

    // A legacy journal is parsed without the lock: nothing writes to it any more, so there is no writer to
    // exclude, and a failure to read it must not deny access to the current receipts.
    let mut combined = all_from_path(&legacy).unwrap_or_default();
    combined.extend(current);
    Ok(combined)
}

fn validate_limit(limit: usize) -> Result<(), ContextPatchError> {
    if !(1..=MAX_RECENT_LIMIT).contains(&limit) {
        return Err(ContextPatchError::invalid(format!(
            "receipt limit must be between 1 and {MAX_RECENT_LIMIT}"
        )));
    }
    Ok(())
}

pub fn recent<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    limit: usize,
) -> Result<Vec<Receipt>, ContextPatchError> {
    validate_limit(limit)?;
    let mut receipts = all(repo_root)?;
    receipts.reverse();
    receipts.truncate(limit);
    Ok(receipts)
}

pub fn unsettled<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    limit: usize,
) -> Result<Vec<Receipt>, ContextPatchError> {
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
    fn batch_rotation_preserves_applied_prefix_with_later_interruption() {
        let root = temp_repo("batch-rotation");
        let old_id = begin_file(&root, "replace_exact", "old.txt", Some("aaa")).unwrap();
        settle_file(&root, &old_id, Outcome::Applied, Some("bbb")).unwrap();
        let journal = journal_path(&root).unwrap();
        let mut handle = OpenOptions::new().append(true).open(&journal).unwrap();
        writeln!(handle, "{}", "x".repeat(MAX_JOURNAL_BYTES as usize)).unwrap();
        drop(handle);

        let paths = vec!["first.txt".to_string(), "second.txt".to_string()];
        let mut batch = begin_file_batch(&root, "bulk_replace_exact", &paths).unwrap();
        let first = batch.begin_file("first.txt", Some("111")).unwrap();
        batch
            .settle_file(&first, Outcome::Applied, Some("222"))
            .unwrap();
        batch.begin_file("second.txt", Some("333")).unwrap();
        drop(batch);

        let receipts = all(&root).unwrap();
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0].path.as_deref(), Some("first.txt"));
        assert_eq!(receipts[0].outcome.as_deref(), Some("applied"));
        assert_eq!(receipts[1].path.as_deref(), Some("second.txt"));
        assert!(receipts[1].interrupted());
        assert!(fs::metadata(journal).unwrap().len() <= MAX_JOURNAL_BYTES);
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
