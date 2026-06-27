use std::path::Path;
use std::{env, fs, path::PathBuf};

use crate::error::ContextPatchError;
use crate::fs::atomic_write::write_atomic;

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
    if old.is_empty() {
        return Err(ContextPatchError::new("old text must not be empty"));
    }

    let target_path = resolve_existing_file(repo_root, path)?;
    let current = fs::read_to_string(&target_path).map_err(|error| {
        ContextPatchError::new(format!("failed to read {}: {error}", target_path.display()))
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

fn resolve_existing_file(repo_root: &Path, path: &Path) -> Result<PathBuf, ContextPatchError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };

    let resolved = candidate.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve target file {}: {error}",
            candidate.display()
        ))
    })?;

    if !resolved.starts_with(&root) {
        return Err(ContextPatchError::new(format!(
            "target path {} is outside repository root {}",
            resolved.display(),
            root.display()
        )));
    }

    if !resolved.is_file() {
        return Err(ContextPatchError::new(format!(
            "target path {} is not a file",
            resolved.display()
        )));
    }

    Ok(resolved)
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
