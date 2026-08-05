//! Typed validation for Git requests.
//!
//! This is policy, not adaptation. Which paths a Git action may name, and what a commit message may
//! contain, are safety rules rather than transport concerns, so they belong beside the other Git guards
//! in `core`. Keeping them here means the CLI and any future adapter enforce the same rules as the MCP
//! server, and means the rules can be exercised without a JSON layer in the way.
//!
//! Refusals carry the detail only, never a tool-name prefix. The adapter owns how a refusal is addressed
//! to its caller, which is what lets this policy move without changing a single refusal string.

use std::collections::BTreeSet;
use std::path::{Component, Path};

use crate::error::ContextPatchError;

/// Longest accepted commit subject, in bytes.
pub const MAX_COMMIT_SUBJECT_BYTES: usize = 200;

/// Longest accepted commit body, in bytes.
pub const MAX_COMMIT_BODY_BYTES: usize = 20_000;

/// Validate a commit subject and return it trimmed.
///
/// A subject reaches Git as one argument, so an embedded newline would silently become a body and a NUL
/// would truncate it. Both are refused rather than reinterpreted.
pub fn commit_subject(subject: &str) -> Result<String, ContextPatchError> {
    let trimmed = subject.trim();
    if trimmed.is_empty() {
        return Err(ContextPatchError::new("subject must not be empty"));
    }
    if trimmed.len() > MAX_COMMIT_SUBJECT_BYTES {
        return Err(ContextPatchError::new(format!(
            "subject must be at most {MAX_COMMIT_SUBJECT_BYTES} bytes"
        )));
    }
    if subject.contains('\n') || subject.contains('\r') || subject.contains('\0') {
        return Err(ContextPatchError::new(
            "subject must be a single line without NUL bytes",
        ));
    }
    Ok(trimmed.to_string())
}

/// Validate a commit body and return it trimmed.
pub fn commit_body(body: &str) -> Result<String, ContextPatchError> {
    if body.contains('\0') {
        return Err(ContextPatchError::new("body must not contain NUL bytes"));
    }
    if body.len() > MAX_COMMIT_BODY_BYTES {
        return Err(ContextPatchError::new(format!(
            "body must be at most {MAX_COMMIT_BODY_BYTES} bytes"
        )));
    }
    Ok(body.trim().to_string())
}

/// Confine one caller-named Git path to the repository root.
///
/// This is the guard that bounds every path-taking Git action, which is why those actions do not also
/// require an exact worktree root: the paths they name cannot leave the configured tree. Four distinct
/// escapes are refused. Pathspec metacharacters are rejected because Git would expand them into paths
/// the caller never wrote. Absolute paths and non-normal components are rejected before any filesystem
/// access, so `..` never gets a chance to resolve. Finally the candidate is resolved and required to sit
/// under the root, which catches a symlink whose target leaves the tree.
///
/// The original spelling is returned, not the resolved path, because Git must receive exactly what the
/// caller named.
pub fn git_path(root: &Path, raw: &str) -> Result<String, ContextPatchError> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(ContextPatchError::new(
            "path must not be empty or contain NUL",
        ));
    }
    if raw.starts_with(':') || raw.contains('*') || raw.contains('?') || raw.contains('[') {
        return Err(ContextPatchError::new(format!(
            "path `{raw}` contains Git pathspec metacharacters"
        )));
    }

    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(ContextPatchError::new(format!(
            "path `{raw}` must be repository-relative"
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => {
                return Err(ContextPatchError::new(format!(
                    "path `{raw}` must be a normalized relative path"
                )))
            }
        }
    }

    let candidate = root.join(path);
    let resolved = if candidate.exists() {
        candidate
            .canonicalize()
            .map_err(|error| ContextPatchError::new(format!("failed to resolve path `{raw}`: {error}")))?
    } else {
        // A path that does not exist yet still has to be confined, so the parent is resolved and the
        // leaf appended. Canonicalizing the whole candidate would fail instead of answering.
        let parent = candidate
            .parent()
            .ok_or_else(|| ContextPatchError::new(format!("path `{raw}` has no parent")))?;
        let parent = parent.canonicalize().map_err(|error| {
            ContextPatchError::new(format!("failed to resolve parent for `{raw}`: {error}"))
        })?;
        let file_name = candidate
            .file_name()
            .ok_or_else(|| ContextPatchError::new(format!("path `{raw}` has no file name")))?;
        parent.join(file_name)
    };

    if !resolved.starts_with(root) {
        return Err(ContextPatchError::new(format!(
            "path `{raw}` resolves outside repository root"
        )));
    }

    Ok(raw.to_string())
}

/// Confine every caller-named Git path, refusing on the first that escapes.
pub fn git_paths(root: &Path, paths: &[String]) -> Result<Vec<String>, ContextPatchError> {
    paths.iter().map(|path| git_path(root, path)).collect()
}

/// Confine every caller-named Git prefix, tolerating one trailing separator.
pub fn git_prefixes(root: &Path, prefixes: &[String]) -> Result<Vec<String>, ContextPatchError> {
    prefixes
        .iter()
        .map(|prefix| {
            let trimmed = prefix.trim_end_matches('/');
            if trimmed.is_empty() {
                return Err(ContextPatchError::new("prefixes must not be empty"));
            }
            git_path(root, trimmed)
        })
        .collect()
}

/// Select the paths that sit at or beneath one of the prefixes.
///
/// Matching requires either equality or a separator boundary, so the prefix `src` never captures
/// `src-generated`.
pub fn paths_under_prefixes(
    paths: &BTreeSet<String>,
    prefixes: &[String],
) -> BTreeSet<String> {
    paths
        .iter()
        .filter(|path| {
            prefixes
                .iter()
                .any(|prefix| *path == prefix || path.starts_with(&format!("{prefix}/")))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-git-validate-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn paths(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn a_commit_subject_is_trimmed_and_bounded() {
        assert_eq!(commit_subject("  Fix the guard  ").unwrap(), "Fix the guard");
        assert_eq!(
            commit_subject("   ").unwrap_err().to_string(),
            "subject must not be empty"
        );
        assert_eq!(
            commit_subject(&"a".repeat(MAX_COMMIT_SUBJECT_BYTES + 1))
                .unwrap_err()
                .to_string(),
            "subject must be at most 200 bytes"
        );
    }

    #[test]
    fn a_commit_subject_stays_one_line() {
        // A newline would silently become a body and a NUL would truncate the argument.
        for hostile in ["first\nsecond", "first\rsecond", "first\0second"] {
            assert_eq!(
                commit_subject(hostile).unwrap_err().to_string(),
                "subject must be a single line without NUL bytes"
            );
        }
    }

    #[test]
    fn a_commit_body_is_trimmed_and_bounded() {
        assert_eq!(commit_body("  why  ").unwrap(), "why");
        assert_eq!(
            commit_body("has\0nul").unwrap_err().to_string(),
            "body must not contain NUL bytes"
        );
        assert_eq!(
            commit_body(&"a".repeat(MAX_COMMIT_BODY_BYTES + 1))
                .unwrap_err()
                .to_string(),
            "body must be at most 20000 bytes"
        );
    }

    #[test]
    fn an_ordinary_relative_path_is_returned_as_written() {
        let root = test_root("ordinary_path");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("lib.rs"), "alpha").unwrap();

        // The original spelling is returned, because Git must receive exactly what the caller named.
        assert_eq!(git_path(&root, "src/lib.rs").unwrap(), "src/lib.rs");
    }

    #[test]
    fn a_path_that_does_not_exist_yet_is_still_confined() {
        let root = test_root("absent_path");

        assert_eq!(git_path(&root, "new.txt").unwrap(), "new.txt");
        assert!(git_path(&root, "missing/new.txt").is_err());
    }

    #[test]
    fn pathspec_metacharacters_are_refused() {
        let root = test_root("pathspec");

        // Git would expand these into paths the caller never wrote.
        for hostile in [":!src", "src/*.rs", "src/?.rs", "src/[ab].rs"] {
            let error = git_path(&root, hostile).unwrap_err().to_string();
            assert!(error.contains("Git pathspec metacharacters"), "{error}");
        }
    }

    #[test]
    fn parent_components_and_absolute_paths_are_refused_before_any_resolution() {
        let root = test_root("traversal");

        assert!(git_path(&root, "../escape.txt")
            .unwrap_err()
            .to_string()
            .contains("normalized relative path"));
        assert!(git_path(&root, "src/../../escape.txt")
            .unwrap_err()
            .to_string()
            .contains("normalized relative path"));
        assert!(git_path(&root, "/etc/hosts")
            .unwrap_err()
            .to_string()
            .contains("repository-relative"));
    }

    #[test]
    fn an_empty_path_or_embedded_nul_is_refused() {
        let root = test_root("empty_path");

        for hostile in ["", "src/\0lib.rs"] {
            assert_eq!(
                git_path(&root, hostile).unwrap_err().to_string(),
                "path must not be empty or contain NUL"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_leaving_the_root_is_refused() {
        let root = test_root("symlink_escape");
        let outside = test_root("symlink_escape_outside");
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link.txt")).unwrap();

        // Component checks alone cannot catch this; resolution is what closes it.
        let error = git_path(&root, "link.txt").unwrap_err().to_string();
        assert!(error.contains("resolves outside repository root"), "{error}");
    }

    #[test]
    fn a_batch_refuses_on_the_first_escaping_path() {
        let root = test_root("batch_paths");
        fs::write(root.join("ok.txt"), "alpha").unwrap();

        assert_eq!(
            git_paths(&root, &["ok.txt".to_string()]).unwrap(),
            vec!["ok.txt".to_string()]
        );
        assert!(git_paths(&root, &["ok.txt".to_string(), "../out.txt".to_string()]).is_err());
    }

    #[test]
    fn a_prefix_tolerates_a_trailing_separator_but_not_emptiness() {
        let root = test_root("prefixes");
        fs::create_dir_all(root.join("src")).unwrap();

        assert_eq!(
            git_prefixes(&root, &["src/".to_string()]).unwrap(),
            vec!["src".to_string()]
        );
        assert_eq!(
            git_prefixes(&root, &["/".to_string()])
                .unwrap_err()
                .to_string(),
            "prefixes must not be empty"
        );
    }

    #[test]
    fn a_prefix_matches_only_on_a_separator_boundary() {
        let candidates = paths(&["src", "src/lib.rs", "src-generated/lib.rs", "other/lib.rs"]);

        let selected = paths_under_prefixes(&candidates, &["src".to_string()]);

        // `src` must not capture `src-generated`, which is the whole reason for the boundary check.
        assert_eq!(selected, paths(&["src", "src/lib.rs"]));
    }
}
