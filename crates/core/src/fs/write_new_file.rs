use std::path::{Path, PathBuf};

use crate::error::ContextPatchError;
use crate::fs::guarded_file::create_new_file_in_root;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WriteNewFileSummary {
    pub path: PathBuf,
    pub bytes_written: usize,
}

pub fn write_new_file(
    path: &Path,
    content: &str,
) -> Result<WriteNewFileSummary, ContextPatchError> {
    let repo_root = std::env::current_dir().map_err(|error| {
        ContextPatchError::new(format!("failed to read current directory: {error}"))
    })?;

    write_new_file_in_root(&repo_root, path, content)
}

pub fn write_new_file_in_root(
    repo_root: &Path,
    path: &Path,
    content: &str,
) -> Result<WriteNewFileSummary, ContextPatchError> {
    write_new_file_bytes_in_root(repo_root, path, content.as_bytes())
}

pub fn write_new_file_bytes_in_root(
    repo_root: &Path,
    path: &Path,
    content: &[u8],
) -> Result<WriteNewFileSummary, ContextPatchError> {
    write_new_file_bytes_with_parents_in_root(repo_root, path, content, false)
}

pub fn write_new_file_bytes_with_parents_in_root(
    repo_root: &Path,
    path: &Path,
    content: &[u8],
    parents: bool,
) -> Result<WriteNewFileSummary, ContextPatchError> {
    let target_path = create_new_file_in_root(repo_root, path, content, parents)?;
    Ok(WriteNewFileSummary {
        path: target_path,
        bytes_written: content.len(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::write_new_file_in_root;

    #[test]
    fn creates_new_file() {
        let root = temp_root("creates_new_file");

        let summary =
            write_new_file_in_root(&root, Path::new("sample.txt"), "hello\nworld\n").unwrap();

        assert_eq!(summary.bytes_written, "hello\nworld\n".len());
        assert_eq!(fs::read_to_string(summary.path).unwrap(), "hello\nworld\n");
    }

    #[test]
    fn refuses_existing_file() {
        let root = temp_root("refuses_existing_file");
        let target = root.join("sample.txt");
        fs::write(&target, "original").unwrap();

        let error =
            write_new_file_in_root(&root, Path::new("sample.txt"), "replacement").unwrap_err();

        assert!(error.to_string().contains("already exists"));
        assert_eq!(fs::read_to_string(target).unwrap(), "original");
    }

    #[test]
    fn refuses_paths_outside_root() {
        let root = temp_root("refuses_paths_outside_root");
        let outside = std::env::temp_dir().join(format!(
            "contextpatch-write-new-file-outside-{}",
            std::process::id()
        ));
        let _ = fs::remove_file(&outside);

        let error = write_new_file_in_root(&root, &outside, "outside").unwrap_err();

        assert!(error
            .to_string()
            .contains("normalized repository-relative path"));
        assert!(!outside.exists());
    }

    #[test]
    fn refuses_missing_parent_directory() {
        let root = temp_root("refuses_missing_parent_directory");

        let error =
            write_new_file_in_root(&root, Path::new("missing/sample.txt"), "content").unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to resolve parent directory"));
    }

    #[test]
    fn creates_binary_file() {
        let root = temp_root("creates_binary_file");

        let summary =
            super::write_new_file_bytes_in_root(&root, Path::new("sample.bin"), &[0, 159, 255])
                .unwrap();

        assert_eq!(summary.bytes_written, 3);
        assert_eq!(fs::read(summary.path).unwrap(), vec![0, 159, 255]);
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-write-new-file-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
