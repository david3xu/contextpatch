use std::fs;
use std::path::Path;

use crate::error::ContextPatchError;
use crate::fs::path::resolve_existing_file;

pub fn preview_exact_replacement(
    path: &Path,
    old: &str,
    new: &str,
) -> Result<String, ContextPatchError> {
    let repo_root = std::env::current_dir().map_err(|error| {
        ContextPatchError::new(format!("failed to read current directory: {error}"))
    })?;

    preview_exact_replacement_in_root(&repo_root, path, old, new)
}

pub fn preview_exact_replacement_in_root(
    repo_root: &Path,
    path: &Path,
    old: &str,
    new: &str,
) -> Result<String, ContextPatchError> {
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

    Ok(unified_full_file_diff(
        &target_path.display().to_string(),
        &current,
        &updated,
    ))
}

fn unified_full_file_diff(path: &str, old: &str, new: &str) -> String {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let script = diff_script(&old_lines, &new_lines);
    let old_count = old_lines.len();
    let new_count = new_lines.len();

    let mut output = String::new();
    output.push_str(&format!("--- {path}\n"));
    output.push_str(&format!("+++ {path}\n"));
    output.push_str(&format!("@@ -1,{old_count} +1,{new_count} @@\n"));

    for operation in script {
        match operation {
            DiffOperation::Unchanged(line) => push_diff_line(&mut output, ' ', line),
            DiffOperation::Removed(line) => push_diff_line(&mut output, '-', line),
            DiffOperation::Added(line) => push_diff_line(&mut output, '+', line),
        }
    }

    output
}

fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split_inclusive('\n').collect()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DiffOperation<'a> {
    Unchanged(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

fn diff_script<'a>(old: &'a [&'a str], new: &'a [&'a str]) -> Vec<DiffOperation<'a>> {
    let mut lengths = vec![vec![0usize; new.len() + 1]; old.len() + 1];
    for old_index in (0..old.len()).rev() {
        for new_index in (0..new.len()).rev() {
            lengths[old_index][new_index] = if old[old_index] == new[new_index] {
                lengths[old_index + 1][new_index + 1] + 1
            } else {
                lengths[old_index + 1][new_index].max(lengths[old_index][new_index + 1])
            };
        }
    }

    let mut script = Vec::new();
    let mut old_index = 0;
    let mut new_index = 0;
    while old_index < old.len() && new_index < new.len() {
        if old[old_index] == new[new_index] {
            script.push(DiffOperation::Unchanged(old[old_index]));
            old_index += 1;
            new_index += 1;
        } else if lengths[old_index + 1][new_index] >= lengths[old_index][new_index + 1] {
            script.push(DiffOperation::Removed(old[old_index]));
            old_index += 1;
        } else {
            script.push(DiffOperation::Added(new[new_index]));
            new_index += 1;
        }
    }

    while old_index < old.len() {
        script.push(DiffOperation::Removed(old[old_index]));
        old_index += 1;
    }
    while new_index < new.len() {
        script.push(DiffOperation::Added(new[new_index]));
        new_index += 1;
    }

    script
}

fn push_diff_line(output: &mut String, prefix: char, line: &str) {
    output.push(prefix);
    output.push_str(line);
    if !line.ends_with('\n') {
        output.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::{env, fs};

    use super::preview_exact_replacement_in_root;

    #[test]
    fn previews_exact_replacement_without_writing() {
        let root = test_root("previews_exact_replacement_without_writing");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();

        let diff =
            preview_exact_replacement_in_root(&root, Path::new("sample.txt"), "beta", "delta")
                .unwrap();

        assert!(diff.contains("--- "));
        assert!(diff.contains("@@ -1,3 +1,3 @@"));
        assert!(diff.contains("-beta\n"));
        assert!(diff.contains("+delta\n"));
        assert_eq!(fs::read_to_string(file).unwrap(), "alpha\nbeta\ngamma\n");
    }

    #[test]
    fn refuses_empty_old_text() {
        let root = test_root("refuses_empty_old_text");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha").unwrap();

        let error = preview_exact_replacement_in_root(&root, Path::new("sample.txt"), "", "delta")
            .unwrap_err();

        assert_eq!(error.to_string(), "old text must not be empty");
        assert_eq!(fs::read_to_string(file).unwrap(), "alpha");
    }

    #[test]
    fn refuses_zero_matches() {
        let root = test_root("refuses_zero_matches");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha").unwrap();

        let error =
            preview_exact_replacement_in_root(&root, Path::new("sample.txt"), "missing", "delta")
                .unwrap_err();

        assert_eq!(error.to_string(), "old text was not found in target file");
        assert_eq!(fs::read_to_string(file).unwrap(), "alpha");
    }

    #[test]
    fn refuses_multiple_matches() {
        let root = test_root("refuses_multiple_matches");
        let file = root.join("sample.txt");
        fs::write(&file, "alpha beta beta").unwrap();

        let error =
            preview_exact_replacement_in_root(&root, Path::new("sample.txt"), "beta", "delta")
                .unwrap_err();

        assert_eq!(
            error.to_string(),
            "old text matched 2 times; expected exactly one match"
        );
        assert_eq!(fs::read_to_string(file).unwrap(), "alpha beta beta");
    }

    #[test]
    fn refuses_paths_outside_root() {
        let root = test_root("refuses_paths_outside_root");
        let outside_root = test_root("refuses_paths_outside_root_outside");
        let outside_file = outside_root.join("outside.txt");
        fs::write(&outside_file, "alpha").unwrap();

        let error =
            preview_exact_replacement_in_root(&root, &outside_file, "alpha", "delta").unwrap_err();

        assert!(error.to_string().contains("is outside repository root"));
        assert_eq!(fs::read_to_string(outside_file).unwrap(), "alpha");
    }

    fn test_root(name: &str) -> std::path::PathBuf {
        let root = env::temp_dir().join(format!(
            "contextpatch-diff-preview-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
