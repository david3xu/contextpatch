use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::ContextPatchError;
use crate::fs::path::resolve_new_file;

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
    let target_path = resolve_new_file(repo_root, path)?;
    write_new_file_atomic(&target_path, content.as_bytes())?;

    Ok(WriteNewFileSummary {
        path: target_path,
        bytes_written: content.len(),
    })
}

fn write_new_file_atomic(path: &Path, contents: &[u8]) -> Result<(), ContextPatchError> {
    let parent = path
        .parent()
        .ok_or_else(|| ContextPatchError::new("target path has no parent directory"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ContextPatchError::new("target path has no valid file name"))?;

    let (temp_path, mut temp_file) = create_temp_file(parent, file_name)?;
    let write_result = (|| {
        temp_file.write_all(contents).map_err(|error| {
            ContextPatchError::new(format!(
                "failed to write temporary file {}: {error}",
                temp_path.display()
            ))
        })?;
        temp_file.sync_all().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to flush temporary file {}: {error}",
                temp_path.display()
            ))
        })?;
        drop(temp_file);

        fs::hard_link(&temp_path, path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                ContextPatchError::new(format!("target file {} already exists", path.display()))
            } else {
                ContextPatchError::new(format!(
                    "failed to publish temporary file {} to {}: {error}",
                    temp_path.display(),
                    path.display()
                ))
            }
        })?;

        fs::remove_file(&temp_path).map_err(|error| {
            ContextPatchError::new(format!(
                "failed to remove temporary file {}: {error}",
                temp_path.display()
            ))
        })
    })();

    if write_result.is_err() && temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }

    write_result
}

fn create_temp_file(
    parent: &Path,
    file_name: &str,
) -> Result<(std::path::PathBuf, fs::File), ContextPatchError> {
    for attempt in 0..100 {
        let temp_path = parent.join(format!(
            ".{file_name}.contextpatch.{}.{}.tmp",
            std::process::id(),
            attempt
        ));

        let temp_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map(|file| (temp_path.clone(), file));

        match temp_file {
            Ok(created) => return Ok(created),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(ContextPatchError::new(format!(
                    "failed to create temporary file {}: {error}",
                    temp_path.display()
                )))
            }
        }
    }

    Err(ContextPatchError::new(
        "failed to create a unique temporary file after 100 attempts",
    ))
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

        assert!(error.to_string().contains("outside repository root"));
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
