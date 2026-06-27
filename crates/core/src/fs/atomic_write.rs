use std::path::Path;
use std::{fs, io::Write};

use crate::error::ContextPatchError;

pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), ContextPatchError> {
    let parent = path
        .parent()
        .ok_or_else(|| ContextPatchError::new("target path has no parent directory"))?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ContextPatchError::new("target path has no valid file name"))?;

    let write_result = (|| {
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

            fs::rename(&temp_path, path).map_err(|error| {
                ContextPatchError::new(format!(
                    "failed to rename temporary file {} to {}: {error}",
                    temp_path.display(),
                    path.display()
                ))
            })
        })();

        if write_result.is_err() && temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }

        write_result
    })();

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
