use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use crate::error::ContextPatchError;
use crate::process::runner::CommandCwd;

use super::guarded_path::exact_worktree_root_in;

/// A workspace selection, anchored to the directory descriptor it was validated through.
///
/// The descriptor is the point. Validating a selector by path and then reopening it by path leaves a
/// window in which the directory that was checked and the directory that gets used are different objects:
/// a rename or symlink swap between the two is enough. Holding the descriptor open means the validated
/// directory cannot be replaced out from under the call, and [`Self::revalidate`] proves the path still
/// names it before anything downstream uses that path.
#[derive(Debug)]
pub struct SelectedRepository {
    relative: String,
    path: PathBuf,
    #[cfg(unix)]
    directory: fs::File,
}

impl SelectedRepository {
    /// The resolved worktree root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The normalized selector this came from.
    pub fn relative(&self) -> &str {
        &self.relative
    }

    /// This selection as a repository target that policy functions accept.
    ///
    /// The descriptor travels with it, so every operation reached through this target runs against the
    /// directory that was validated rather than re-resolving a name.
    #[cfg(unix)]
    pub fn repository(&self) -> crate::git::GitRepository<'_> {
        crate::git::GitRepository::anchored(&self.path, &self.directory)
    }

    /// Prove the resolved path still names the directory the descriptor is anchored to.
    ///
    /// Called immediately before the path is handed to anything that reopens it. A mismatch means the
    /// directory was replaced after validation, which is refused rather than followed.
    #[cfg(unix)]
    pub fn revalidate(&self) -> Result<(), ContextPatchError> {
        use std::os::unix::fs::MetadataExt;

        let anchored = self.directory.metadata().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to inspect the anchored selected repository `{}`: {error}",
                self.relative
            ))
        })?;
        let current = fs::metadata(&self.path).map_err(|error| {
            ContextPatchError::new(format!(
                "failed to re-inspect selected repository `{}`: {error}",
                self.relative
            ))
        })?;
        if anchored.dev() != current.dev() || anchored.ino() != current.ino() {
            return Err(ContextPatchError::invalid(format!(
                "selected repository `{}` no longer names the directory that was validated",
                self.relative
            )));
        }
        Ok(())
    }

    /// Non-Unix has no descriptor to anchor to, so there is nothing to prove.
    #[cfg(not(unix))]
    pub fn revalidate(&self) -> Result<(), ContextPatchError> {
        Err(ContextPatchError::invalid(
            "selecting a repository within a workspace requires descriptor-relative directory access, which is unavailable on this platform",
        ))
    }
}

/// Select one exact Git worktree root beneath a fixed workspace boundary.
///
/// The returned selection retains the directory descriptor the walk ended on. Callers must hold it for as
/// long as they use its path.
#[cfg(unix)]
pub fn select_workspace_repository(
    workspace_root: &Path,
    repository: &str,
) -> Result<SelectedRepository, ContextPatchError> {
    let workspace = workspace_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve workspace root {}: {error}",
            workspace_root.display()
        ))
    })?;
    if !workspace.is_dir() {
        return Err(ContextPatchError::new(format!(
            "workspace root {} is not a directory",
            workspace.display()
        )));
    }

    let relative = normalize_repository_selector(repository)?;
    let directory = walk_selector_with_descriptors(&workspace, &relative)?;

    let candidate = workspace.join(&relative);
    let resolved = candidate.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve selected repository `{relative}`: {error}"
        ))
    })?;
    if resolved == workspace || !resolved.starts_with(&workspace) {
        return Err(ContextPatchError::new(format!(
            "selected repository `{relative}` resolves outside the workspace boundary"
        )));
    }

    let path = exact_worktree_root_in(
        &resolved,
        CommandCwd::Anchored {
            directory: &directory,
            logical_path: &resolved,
        },
    )?;
    let selected = SelectedRepository {
        relative,
        path,
        directory,
    };
    // The path was produced by canonicalizing, and the descriptor by walking. Requiring them to agree
    // here means a swap during either step is caught before the selection is handed out at all.
    selected.revalidate()?;
    Ok(selected)
}

/// Non-Unix fails closed: without descriptor-relative opens the walk cannot be made safe.
#[cfg(not(unix))]
pub fn select_workspace_repository(
    _workspace_root: &Path,
    _repository: &str,
) -> Result<SelectedRepository, ContextPatchError> {
    Err(ContextPatchError::invalid(
        "selecting a repository within a workspace requires descriptor-relative directory access, which is unavailable on this platform",
    ))
}

/// Walk the selector one component at a time, never following a link and never naming a full path.
///
/// Each step opens relative to the descriptor the previous step returned, so the parent chain cannot be
/// substituted mid-walk. `O_NOFOLLOW` refuses a symlink at the final component of each step and
/// `O_DIRECTORY` refuses anything that is not a directory, which together cover every component.
#[cfg(unix)]
fn walk_selector_with_descriptors(
    workspace: &Path,
    relative: &str,
) -> Result<fs::File, ContextPatchError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let name = std::ffi::CString::new(workspace.as_os_str().as_encoded_bytes()).map_err(|_| {
        ContextPatchError::invalid("workspace root must not contain NUL".to_string())
    })?;
    let descriptor = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(ContextPatchError::new(format!(
            "failed to open workspace root {}: {}",
            workspace.display(),
            std::io::Error::last_os_error()
        )));
    }
    let mut current = unsafe { fs::File::from_raw_fd(descriptor) };

    let components = Path::new(relative).components().collect::<Vec<_>>();
    let mut walked = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        walked.push(component.as_os_str());
        let component_name =
            std::ffi::CString::new(component.as_os_str().as_encoded_bytes()).map_err(|_| {
                ContextPatchError::invalid(format!(
                    "repository `{relative}` must not contain NUL"
                ))
            })?;

        let child = unsafe {
            libc::openat(
                current.as_raw_fd(),
                component_name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if child < 0 {
            // The open failed, but `ELOOP` and `ENOTDIR` do not distinguish a symlink from a plain file
            // portably. Classify against the same parent descriptor so the refusal names the real cause
            // and reads exactly as the path-based walk always did.
            return Err(classify_walk_failure(
                &current,
                &component_name,
                relative,
                &walked,
                index + 1 == components.len(),
                std::io::Error::last_os_error(),
            ));
        }
        current = unsafe { fs::File::from_raw_fd(child) };
    }

    Ok(current)
}

/// Explain why one descriptor-relative open failed, without leaving the parent descriptor.
#[cfg(unix)]
fn classify_walk_failure(
    parent: &fs::File,
    component: &std::ffi::CStr,
    relative: &str,
    walked: &Path,
    is_final: bool,
    open_error: std::io::Error,
) -> ContextPatchError {
    use std::os::fd::AsRawFd;

    let display = walked.display();
    let mut status = std::mem::MaybeUninit::<libc::stat>::uninit();
    let inspected = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            component.as_ptr(),
            status.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if inspected != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == ErrorKind::NotFound {
            return ContextPatchError::invalid(format!(
                "repository `{relative}` does not exist"
            ));
        }
        return ContextPatchError::new(format!(
            "failed to inspect repository path component `{display}`: {error}"
        ));
    }

    let mode = unsafe { status.assume_init() }.st_mode & libc::S_IFMT;
    if mode == libc::S_IFLNK {
        return ContextPatchError::invalid(format!(
            "repository `{relative}` contains symlink component `{display}`"
        ));
    }
    if mode != libc::S_IFDIR {
        let label = if is_final {
            "selected repository"
        } else {
            "repository path component"
        };
        return ContextPatchError::invalid(format!("{label} `{display}` is not a directory"));
    }

    ContextPatchError::new(format!(
        "failed to inspect repository path component `{display}`: {open_error}"
    ))
}

fn normalize_repository_selector(repository: &str) -> Result<String, ContextPatchError> {
    let path = Path::new(repository);
    let bytes = repository.as_bytes();
    let has_windows_drive_prefix =
        bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if repository.is_empty()
        || repository.starts_with('/')
        || repository.ends_with('/')
        || repository.contains("//")
        || repository.contains('\\')
        || repository.chars().any(char::is_control)
        || has_windows_drive_prefix
    {
        return Err(ContextPatchError::invalid(
            "repository must be a normalized workspace-relative path",
        ));
    }

    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(ContextPatchError::invalid(
                "repository must be a normalized workspace-relative path",
            ));
        };
        let value = part
            .to_str()
            .ok_or_else(|| ContextPatchError::invalid("repository must be valid UTF-8"))?;
        if value.eq_ignore_ascii_case(".git") {
            return Err(ContextPatchError::invalid(
                "repository must not include the Git administrative directory",
            ));
        }
        parts.push(value);
    }

    let normalized = parts.join("/");
    if normalized != repository {
        return Err(ContextPatchError::invalid(
            "repository must be a normalized workspace-relative path",
        ));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    /// Test-only view that keeps the existing path-resolution assertions readable.
    ///
    /// Production callers must hold the selection so the descriptor stays open; discarding it is only safe
    /// here, where the assertion is about which path was resolved.
    fn resolve_workspace_git_root(
        workspace: &Path,
        repository: &str,
    ) -> Result<PathBuf, ContextPatchError> {
        select_workspace_repository(workspace, repository)
            .map(|selected| selected.path().to_path_buf())
    }

    #[cfg(unix)]
    #[test]
    fn anchored_git_queries_and_mutations_reach_the_selected_original_inode() {
        // The end-to-end proof that the descriptor is doing work. Two distinct repositories, one selected;
        // then the selected directory is swapped for the other at the same name. Anchored Git execution
        // must keep operating on the original inode, and the replacement must be left completely alone.
        use crate::git::state;

        let workspace = temp_root("anchored_execution_swap");
        let target = workspace.join("target");
        let decoy = workspace.join("decoy");
        init_git_repo(&target);
        init_git_repo(&decoy);
        commit_marker(&target, "original\n");
        commit_marker(&decoy, "replacement\n");

        let selected = select_workspace_repository(&workspace, "target").unwrap();
        let original_head = state::stdout(selected.repository(), &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        let decoy_head_before = state::stdout(&decoy, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        assert_ne!(
            original_head, decoy_head_before,
            "the two repositories must be distinguishable"
        );

        // Swap `target` for `decoy` behind the selection's back.
        let moved_aside = workspace.join("moved-aside");
        fs::rename(&target, &moved_aside).unwrap();
        fs::rename(&decoy, &target).unwrap();

        // A query still reads the original repository, not the one now sitting at that name.
        let head_after_swap = state::stdout(selected.repository(), &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        assert_eq!(head_after_swap, original_head);
        assert_eq!(
            state::stdout(selected.repository(), &["show", "-s", "--format=%s", "HEAD"])
                .unwrap()
                .trim(),
            "original"
        );

        // And a mutation lands in the original repository too.
        state::success(
            selected.repository(),
            &[
                "commit".to_string(),
                "--allow-empty".to_string(),
                "--quiet".to_string(),
                "-m".to_string(),
                "anchored".to_string(),
            ],
        )
        .unwrap();

        assert_eq!(state::rev_count(&moved_aside, "HEAD").unwrap(), 2);
        assert_eq!(
            state::stdout(&moved_aside, &["show", "-s", "--format=%s", "HEAD"])
                .unwrap()
                .trim(),
            "anchored"
        );

        // The replacement is untouched: same head, same subject, still one commit.
        assert_eq!(state::rev_count(&target, "HEAD").unwrap(), 1);
        assert_eq!(
            state::stdout(&target, &["rev-parse", "HEAD"]).unwrap().trim(),
            decoy_head_before
        );
        assert_eq!(
            fs::read_to_string(target.join("marker.txt")).unwrap(),
            "replacement\n"
        );
    }

    #[cfg(unix)]
    fn commit_marker(root: &Path, contents: &str) {
        fs::write(root.join("marker.txt"), contents).unwrap();
        run_git(root, &["add", "."]);
        run_git(
            root,
            &["commit", "--quiet", "-m", contents.trim()],
        );
    }

    #[cfg(unix)]
    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    #[cfg(unix)]
    #[test]
    fn a_selection_survives_its_directory_being_renamed_away() {
        // The adversarial case the descriptor exists for: validate, then swap the directory for a different
        // one at the same path. The selection must notice rather than operate on the replacement.
        let workspace = temp_root("rename_swap");
        let target = workspace.join("target");
        let decoy = workspace.join("decoy");
        init_git_repo(&target);
        init_git_repo(&decoy);
        fs::write(target.join("marker.txt"), "original\n").unwrap();
        fs::write(decoy.join("marker.txt"), "replacement\n").unwrap();

        let selected = select_workspace_repository(&workspace, "target").unwrap();
        assert!(selected.revalidate().is_ok());

        // Swap `target` for `decoy` behind the selection's back.
        fs::rename(&target, workspace.join("moved-away")).unwrap();
        fs::rename(&decoy, &target).unwrap();

        let error = selected.revalidate().unwrap_err().to_string();
        assert_eq!(
            error,
            "selected repository `target` no longer names the directory that was validated"
        );

        // Neither the replacement nor the directory that was moved aside was touched.
        assert_eq!(
            fs::read_to_string(target.join("marker.txt")).unwrap(),
            "replacement\n"
        );
        assert_eq!(
            fs::read_to_string(workspace.join("moved-away").join("marker.txt")).unwrap(),
            "original\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_selection_survives_its_directory_being_replaced_by_a_symlink() {
        // Same window, different weapon: replace the validated directory with a link pointing outside the
        // workspace. The link target must be left alone.
        let workspace = temp_root("symlink_swap");
        let outside = temp_root("symlink_swap_outside");
        let target = workspace.join("target");
        init_git_repo(&target);
        init_git_repo(&outside);
        fs::write(outside.join("secret.txt"), "outside\n").unwrap();

        let selected = select_workspace_repository(&workspace, "target").unwrap();

        fs::rename(&target, workspace.join("moved-away")).unwrap();
        std::os::unix::fs::symlink(&outside, &target).unwrap();

        let error = selected.revalidate().unwrap_err().to_string();
        assert_eq!(
            error,
            "selected repository `target` no longer names the directory that was validated"
        );
        assert_eq!(
            fs::read_to_string(outside.join("secret.txt")).unwrap(),
            "outside\n"
        );
        assert!(!outside.join("marker.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_selection_keeps_its_descriptor_open_after_the_directory_is_unlinked() {
        // Holding the descriptor is what makes the previous two tests possible: the validated directory
        // stays reachable through the descriptor even once its name is gone, so the mismatch is detectable
        // instead of the path silently resolving to something else.
        let workspace = temp_root("unlinked_selection");
        let target = workspace.join("target");
        init_git_repo(&target);

        let selected = select_workspace_repository(&workspace, "target").unwrap();
        fs::rename(&target, workspace.join("renamed")).unwrap();

        // The path is gone, so revalidation cannot even inspect it, and that is still a refusal.
        let error = selected.revalidate().unwrap_err().to_string();
        assert!(
            error.starts_with("failed to re-inspect selected repository `target`"),
            "{error}"
        );
        assert!(workspace.join("renamed").join(".git").exists());
    }

    #[test]
    fn resolves_only_exact_descendant_git_roots() {
        let workspace = temp_root("workspace_git_root");
        let first = workspace.join("first");
        init_git_repo(&first);
        fs::create_dir_all(first.join("subdir")).unwrap();

        let parent = workspace.join("parent");
        init_git_repo(&parent);
        let nested = parent.join("nested");
        init_git_repo(&nested);

        assert_eq!(
            resolve_workspace_git_root(&workspace, "first").unwrap(),
            first.canonicalize().unwrap()
        );
        assert_eq!(
            resolve_workspace_git_root(&workspace, "parent/nested").unwrap(),
            nested.canonicalize().unwrap()
        );

        let error = resolve_workspace_git_root(&workspace, "first/subdir").unwrap_err();
        assert!(
            error.to_string().contains("not the Git worktree root"),
            "{error}"
        );

        let plain = workspace.join("plain");
        fs::create_dir(&plain).unwrap();
        let error = resolve_workspace_git_root(&workspace, "plain").unwrap_err();
        assert!(
            error.to_string().contains("resolve Git root failed"),
            "{error}"
        );

        fs::write(workspace.join("file"), "not a directory\n").unwrap();
        let error = resolve_workspace_git_root(&workspace, "file").unwrap_err();
        assert!(error.to_string().contains("not a directory"), "{error}");
    }

    #[test]
    fn refuses_non_normalized_repository_selectors() {
        let workspace = temp_root("workspace_git_root_invalid");
        for repository in [
            "",
            "/absolute",
            "../escape",
            "repo/",
            "repo//nested",
            "repo\\nested",
            "./repo",
            "repo/../other",
            "C:/repo",
            "repo\nnested",
        ] {
            let error = resolve_workspace_git_root(&workspace, repository).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("normalized workspace-relative path"),
                "{repository:?}: {error}"
            );
        }

        for repository in [".git", "repo/.GIT/objects"] {
            let error = resolve_workspace_git_root(&workspace, repository).unwrap_err();
            assert!(
                error.to_string().contains("Git administrative directory"),
                "{repository:?}: {error}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_components_even_when_the_target_is_a_git_root() {
        use std::os::unix::fs::symlink;

        let workspace = temp_root("workspace_git_root_symlink");
        let outside = temp_root("workspace_git_root_outside");
        init_git_repo(&outside);
        symlink(&outside, workspace.join("linked")).unwrap();

        let error = resolve_workspace_git_root(&workspace, "linked").unwrap_err();
        assert!(error.to_string().contains("symlink component"), "{error}");
    }

    fn init_git_repo(root: &Path) {
        fs::create_dir_all(root).unwrap();
        let status = Command::new("git")
            .arg("init")
            .arg("--quiet")
            .arg(root)
            .status()
            .unwrap();
        assert!(status.success());
    }

    fn temp_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
