use std::path::{Path, PathBuf};
use std::{env, fs};

use crate::error::ContextPatchError;
use crate::fs::atomic_write::write_atomic;
use crate::fs::hash::{sha256_bytes, validate_sha256};
use crate::fs::mutation_lock::try_file_mutation_lock;
use crate::fs::path::resolve_existing_file;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReplaceExactSummary {
    pub path: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
    pub bytes_written: usize,
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
    if old.is_empty() {
        return Err(ContextPatchError::new("old text must not be empty"));
    }
    if let Some(expected_sha256) = expected_sha256 {
        validate_sha256(expected_sha256)?;
    }

    let target_path = resolve_existing_file(repo_root, path)?;
    let _mutation_lock = try_file_mutation_lock(repo_root, &target_path)?;
    let current_bytes = fs::read(&target_path).map_err(|error| {
        ContextPatchError::new(format!("failed to read {}: {error}", target_path.display()))
    })?;
    if let Some(expected_sha256) = expected_sha256 {
        let current_sha256 = sha256_bytes(&current_bytes);
        if current_sha256 != expected_sha256 {
            return Err(ContextPatchError::new(format!(
                "current file SHA-256 mismatch; current_sha256={current_sha256}, \
                 expected_sha256={expected_sha256}"
            )));
        }
    }
    let current = String::from_utf8(current_bytes).map_err(|error| {
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

    write_atomic(&target_path, updated.as_bytes())?;

    Ok(ReplaceExactSummary {
        path: target_path,
        start_byte,
        end_byte,
        bytes_written: updated.len(),
    })
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
