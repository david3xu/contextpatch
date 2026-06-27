use std::path::Path;
use std::{env, fs};

use crate::error::ContextPatchError;
use crate::fs::path::resolve_existing_file;

pub fn read_range(
    path: &Path,
    start_line: usize,
    end_line: usize,
) -> Result<String, ContextPatchError> {
    let repo_root = env::current_dir().map_err(|error| {
        ContextPatchError::new(format!("failed to read current directory: {error}"))
    })?;
    read_range_in_root(&repo_root, path, start_line, end_line)
}

pub fn read_range_in_root(
    repo_root: &Path,
    path: &Path,
    start_line: usize,
    end_line: usize,
) -> Result<String, ContextPatchError> {
    if start_line == 0 {
        return Err(ContextPatchError::new("start line must be 1 or greater"));
    }
    if end_line < start_line {
        return Err(ContextPatchError::new(
            "end line must be greater than or equal to start line",
        ));
    }

    let target_path = resolve_existing_file(repo_root, path)?;
    let contents = fs::read_to_string(&target_path).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to read UTF-8 text file {}: {error}",
            target_path.display()
        ))
    })?;

    let mut output = String::new();
    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        if line_number >= start_line && line_number <= end_line {
            output.push_str(&format!("{line_number}. {line}\n"));
        }
        if line_number > end_line {
            break;
        }
    }

    if output.is_empty() {
        return Err(ContextPatchError::new(format!(
            "requested range {start_line}..{end_line} did not include any lines"
        )));
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reads_bounded_lines_with_numbers() {
        let root = test_root("reads_bounded_lines_with_numbers");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();

        let output = read_range_in_root(&root, Path::new("sample.txt"), 2, 3).unwrap();

        assert_eq!(output, "2. beta\n3. gamma\n");
    }

    #[test]
    fn refuses_zero_start_line() {
        let root = test_root("refuses_zero_start_line");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha\n").unwrap();

        let error = read_range_in_root(&root, Path::new("sample.txt"), 0, 1).unwrap_err();

        assert_eq!(error.to_string(), "start line must be 1 or greater");
    }

    #[test]
    fn refuses_reversed_range() {
        let root = test_root("refuses_reversed_range");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha\n").unwrap();

        let error = read_range_in_root(&root, Path::new("sample.txt"), 2, 1).unwrap_err();

        assert_eq!(
            error.to_string(),
            "end line must be greater than or equal to start line"
        );
    }

    #[test]
    fn refuses_range_past_end() {
        let root = test_root("refuses_range_past_end");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha\n").unwrap();

        let error = read_range_in_root(&root, Path::new("sample.txt"), 2, 3).unwrap_err();

        assert_eq!(
            error.to_string(),
            "requested range 2..3 did not include any lines"
        );
    }

    #[test]
    fn refuses_paths_outside_root() {
        let root = test_root("refuses_paths_outside_root");
        let outside_root = test_root("refuses_paths_outside_root_outside");
        let outside_file = outside_root.join("outside.txt");
        fs::write(&outside_file, "alpha\n").unwrap();

        let error = read_range_in_root(&root, &outside_file, 1, 1).unwrap_err();

        assert!(error.to_string().contains("is outside repository root"));
    }

    #[test]
    fn refuses_non_utf8_files() {
        let root = test_root("refuses_non_utf8_files");
        let file = root.join("sample.bin");
        fs::write(&file, [0xff, 0xfe, 0xfd]).unwrap();

        let error = read_range_in_root(&root, Path::new("sample.bin"), 1, 1).unwrap_err();

        assert!(error.to_string().contains("failed to read UTF-8 text file"));
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
