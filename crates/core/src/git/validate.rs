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
use std::io::ErrorKind;
use std::path::{Component, Path};

use crate::error::ContextPatchError;
use crate::git::root::RepositoryRoot;

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

/// Reject a path on its spelling alone, before any filesystem access.
///
/// Pathspec metacharacters are rejected because Git would expand them into paths the caller never wrote.
/// Absolute paths and non-normal components are rejected here so `..` never gets a chance to resolve.
fn ensure_git_path_shape(raw: &str) -> Result<(), ContextPatchError> {
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
        if !matches!(component, Component::Normal(_)) {
            return Err(ContextPatchError::new(format!(
                "path `{raw}` must be a normalized relative path"
            )));
        }
    }
    Ok(())
}

/// Confine one caller-named Git path to the repository root.
///
/// This is the guard that bounds every path-taking Git action, which is why those actions do not also
/// require an exact worktree root: the paths they name cannot leave the configured tree.
///
/// Containment is established the same way under either authority. An anchored root walks components
/// relative to the descriptor it retains; a path-backed root opens its own root descriptor once and then
/// walks the same way. Both refuse a symlink at any component, because following one would mean resolving a
/// name and the walk exists precisely to avoid that.
///
/// This supersedes an earlier arrangement in which a path-backed root canonicalized the candidate and
/// merely required the result to sit under the root, which accepted an in-tree symlink. That difference was
/// recorded as deliberate at the time; it is now closed, so a configured root is exactly as safe as a
/// selected one.
///
/// The original spelling is returned, not the resolved path, because Git must receive exactly what the
/// caller named.
pub fn git_path<'a>(
    root: impl Into<RepositoryRoot<'a>>,
    raw: &str,
) -> Result<String, ContextPatchError> {
    let root = root.into();
    ensure_git_path_shape(raw)?;

    #[cfg(unix)]
    {
        confine_relative_to_root(root, raw)?;
        Ok(raw.to_string())
    }

    #[cfg(not(unix))]
    {
        let _ = root;
        Err(ContextPatchError::new(
            "path confinement requires descriptor-relative operations",
        ))
    }
}

/// Containment for either authority, walking from a root descriptor.
///
/// The final component is inspected rather than opened, because a path that does not exist yet is a
/// legitimate create target.
#[cfg(unix)]
fn confine_relative_to_root(
    root: RepositoryRoot<'_>,
    raw: &str,
) -> Result<(), ContextPatchError> {
    let handle = crate::fs::rooted::root_descriptor(root)?;
    confine_relative_to_descriptor(handle.as_file(), raw)
}

/// Containment for a root whose authority is a retained descriptor.
///
/// Each component is opened relative to the descriptor the previous step returned, so the parent chain
/// cannot be substituted part way through and the root's name is never resolved again. The final component
/// is inspected rather than opened, because a path that does not exist yet is a legitimate create target.
#[cfg(unix)]
fn confine_relative_to_descriptor(
    directory: &std::fs::File,
    raw: &str,
) -> Result<(), ContextPatchError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let components = Path::new(raw).components().collect::<Vec<_>>();
    let mut current: Option<std::fs::File> = None;
    let mut walked = std::path::PathBuf::new();

    for (index, component) in components.iter().enumerate() {
        walked.push(component.as_os_str());
        let name = std::ffi::CString::new(component.as_os_str().as_encoded_bytes())
            .map_err(|_| ContextPatchError::new("path must not be empty or contain NUL"))?;
        let parent = current.as_ref().unwrap_or(directory);
        let is_final = index + 1 == components.len();

        if is_final {
            // Absence is an answer here, not a failure: the caller may be naming something to create.
            match inspect_relative(parent, &name) {
                Ok(Some(mode)) if mode == libc::S_IFLNK => {
                    return Err(symlink_component(raw, &walked));
                }
                Ok(_) => return Ok(()),
                Err(error) => return Err(error),
            }
        }

        let opened = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if opened < 0 {
            return Err(classify_confinement_failure(parent, &name, raw, &walked));
        }
        current = Some(unsafe { std::fs::File::from_raw_fd(opened) });
    }

    Ok(())
}

#[cfg(unix)]
fn symlink_component(raw: &str, walked: &Path) -> ContextPatchError {
    ContextPatchError::new(format!(
        "path `{raw}` contains symlink component `{}`",
        walked.display()
    ))
}

/// The file type of one entry relative to a descriptor, or `None` when it does not exist.
#[cfg(unix)]
fn inspect_relative(
    parent: &std::fs::File,
    name: &std::ffi::CStr,
) -> Result<Option<libc::mode_t>, ContextPatchError> {
    use std::os::fd::AsRawFd;

    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    let inspected = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            status.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if inspected != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(ContextPatchError::new(format!(
            "failed to inspect path component: {error}"
        )));
    }
    Ok(Some(unsafe { status.assume_init() }.st_mode & libc::S_IFMT))
}

/// Explain why one descriptor-relative open failed, without leaving the parent descriptor.
///
/// `ELOOP` and `ENOTDIR` do not portably distinguish a symlink from a plain file, so the entry is
/// classified against the same parent rather than guessed from the error code.
#[cfg(unix)]
fn classify_confinement_failure(
    parent: &std::fs::File,
    name: &std::ffi::CStr,
    raw: &str,
    walked: &Path,
) -> ContextPatchError {
    match inspect_relative(parent, name) {
        Ok(None) => ContextPatchError::new(format!(
            "failed to resolve parent for `{raw}`: No such file or directory (os error 2)"
        )),
        Ok(Some(mode)) if mode == libc::S_IFLNK => symlink_component(raw, walked),
        Ok(Some(_)) => ContextPatchError::new(format!(
            "path `{raw}` component `{}` is not a directory",
            walked.display()
        )),
        Err(error) => error,
    }
}

/// Confine every caller-named Git path, refusing on the first that escapes.
pub fn git_paths<'a>(
    root: impl Into<RepositoryRoot<'a>>,
    paths: &[String],
) -> Result<Vec<String>, ContextPatchError> {
    let root = root.into();
    paths.iter().map(|path| git_path(root, path)).collect()
}

/// Confine every caller-named Git prefix, tolerating one trailing separator.
pub fn git_prefixes<'a>(
    root: impl Into<RepositoryRoot<'a>>,
    prefixes: &[String],
) -> Result<Vec<String>, ContextPatchError> {
    let root = root.into();
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
    #[cfg(unix)]
    fn anchored_root(root: &std::path::Path) -> std::fs::File {
        std::fs::File::open(root).unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn an_anchored_root_refuses_a_symlink_at_the_final_component() {
        let root = test_root("anchored_final_symlink");
        let outside = test_root("anchored_final_symlink_outside");
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link.txt")).unwrap();
        let directory = anchored_root(&root);

        let error = git_path(RepositoryRoot::anchored(&root, &directory), "link.txt")
            .unwrap_err()
            .to_string();

        assert_eq!(error, "path `link.txt` contains symlink component `link.txt`");
        assert_eq!(
            fs::read_to_string(outside.join("secret.txt")).unwrap(),
            "secret"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_anchored_root_refuses_a_symlink_at_an_intermediate_component() {
        let root = test_root("anchored_intermediate_symlink");
        let outside = test_root("anchored_intermediate_symlink_outside");
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
        let directory = anchored_root(&root);

        let error = git_path(RepositoryRoot::anchored(&root, &directory), "linked/secret.txt")
            .unwrap_err()
            .to_string();

        assert_eq!(
            error,
            "path `linked/secret.txt` contains symlink component `linked`"
        );
    }

    #[cfg(unix)]
    #[test]
    fn both_authorities_refuse_an_in_tree_symlink_uniformly() {
        // This replaces a test that asserted the two authorities differ here and recorded that difference as
        // deliberate. The difference is now closed: following a symlink means resolving a name, and neither
        // authority does that. A configured root is exactly as safe as a selected one.
        //
        // The user-visible consequence is intentional and worth stating: a configured root no longer
        // accepts an in-tree symlink that it previously followed and allowed.
        let root = test_root("in_tree_symlink_uniform");
        fs::create_dir_all(root.join("real")).unwrap();
        fs::write(root.join("real").join("file.txt"), "inside").unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("alias")).unwrap();
        let directory = anchored_root(&root);

        for (label, root) in [
            ("path-backed", RepositoryRoot::from_path(&root)),
            ("anchored", RepositoryRoot::anchored(&root, &directory)),
        ] {
            assert_eq!(
                git_path(root, "alias/file.txt").unwrap_err().to_string(),
                "path `alias/file.txt` contains symlink component `alias`",
                "{label}"
            );
            // The same file is still reachable by its real name under either authority.
            assert_eq!(
                git_path(root, "real/file.txt").unwrap(),
                "real/file.txt",
                "{label}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_anchored_root_refuses_traversal_and_absolute_paths_before_touching_the_filesystem() {
        // The shape checks run first for both authorities, so `..` never reaches a descriptor walk.
        let root = test_root("anchored_shape_checks");
        let directory = anchored_root(&root);
        let anchored = RepositoryRoot::anchored(&root, &directory);

        assert!(git_path(anchored, "../escape.txt")
            .unwrap_err()
            .to_string()
            .contains("normalized relative path"));
        assert!(git_path(anchored, "/etc/hosts")
            .unwrap_err()
            .to_string()
            .contains("repository-relative"));
        assert!(git_path(anchored, "src/*.rs")
            .unwrap_err()
            .to_string()
            .contains("Git pathspec metacharacters"));
    }

    #[cfg(unix)]
    #[test]
    fn an_anchored_root_accepts_a_leaf_that_does_not_exist_yet() {
        // A create target is a legitimate answer, so the final component is inspected rather than opened.
        let root = test_root("anchored_absent_leaf");
        fs::create_dir_all(root.join("src")).unwrap();
        let directory = anchored_root(&root);
        let anchored = RepositoryRoot::anchored(&root, &directory);

        assert_eq!(git_path(anchored, "src/new.rs").unwrap(), "src/new.rs");
        assert_eq!(git_path(anchored, "new.rs").unwrap(), "new.rs");
    }

    #[cfg(unix)]
    #[test]
    fn an_anchored_root_reports_a_missing_or_non_directory_parent() {
        let root = test_root("anchored_bad_parent");
        fs::write(root.join("file.txt"), "content").unwrap();
        let directory = anchored_root(&root);
        let anchored = RepositoryRoot::anchored(&root, &directory);

        assert!(git_path(anchored, "missing/new.rs")
            .unwrap_err()
            .to_string()
            .starts_with("failed to resolve parent for `missing/new.rs`"));
        assert_eq!(
            git_path(anchored, "file.txt/inner.rs")
                .unwrap_err()
                .to_string(),
            "path `file.txt/inner.rs` component `file.txt` is not a directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_anchored_root_confines_relative_to_the_descriptor_after_the_name_is_replaced() {
        // The point of the descriptor: the root's name is swapped for a different directory, and
        // confinement still answers about the directory that was anchored.
        let parent = test_root("anchored_after_swap");
        let original = parent.join("original");
        let decoy = parent.join("decoy");
        fs::create_dir_all(original.join("src")).unwrap();
        fs::create_dir_all(&decoy).unwrap();
        let directory = anchored_root(&original);

        fs::rename(&original, parent.join("moved-aside")).unwrap();
        fs::rename(&decoy, &original).unwrap();

        // `src` exists only in the anchored directory, never in the replacement now sitting at that name.
        let anchored = RepositoryRoot::anchored(&original, &directory);
        assert_eq!(git_path(anchored, "src/new.rs").unwrap(), "src/new.rs");
        // A name-based root sees the replacement instead, which is the behaviour being moved away from.
        assert!(git_path(original.as_path(), "src/new.rs").is_err());
    }

    #[test]
    fn a_symlink_leaving_the_root_is_refused() {
        let root = test_root("symlink_escape");
        let outside = test_root("symlink_escape_outside");
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link.txt")).unwrap();

        // Previously this was caught by resolving the candidate and comparing it against the root. The walk
        // now refuses the link at the component itself, before anything outside is reached at all, so the
        // refusal names the component rather than the resolution.
        let error = git_path(&root, "link.txt").unwrap_err().to_string();
        assert_eq!(error, "path `link.txt` contains symlink component `link.txt`");
        assert_eq!(
            fs::read_to_string(outside.join("secret.txt")).unwrap(),
            "secret",
            "nothing outside the root was read"
        );
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
