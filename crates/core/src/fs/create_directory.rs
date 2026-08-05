use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::ContextPatchError;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct CreateDirectorySummary {
    pub path: PathBuf,
    pub directories_created: Vec<PathBuf>,
}

pub fn create_directory(
    path: &Path,
    parents: bool,
) -> Result<CreateDirectorySummary, ContextPatchError> {
    let repo_root = std::env::current_dir().map_err(|error| {
        ContextPatchError::new(format!("failed to read current directory: {error}"))
    })?;

    create_directory_in_root(&repo_root, path, parents)
}

/// Create one directory under the repository root.
///
/// The walk is descriptor-relative from the root's own authority. Each component is inspected with
/// `fstatat` and no-follow, created with `mkdirat`, and descended with `openat` and `O_NOFOLLOW`, so a
/// component cannot be substituted between the check and the creation and a symlink is refused rather than
/// followed. Refusals keep naming absolute paths, which are built from the root label for the message only
/// and never used to reach anything.
pub fn create_directory_in_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    path: &Path,
    parents: bool,
) -> Result<CreateDirectorySummary, ContextPatchError> {
    #[cfg(unix)]
    {
        let authority = repo_root.into();
        let root = crate::fs::rooted::canonical_label(authority)?;
        let components = safe_relative_components(&root, path)?;
        create_directory_unix(authority, &root, &components, parents)
    }

    #[cfg(not(unix))]
    {
        let _ = (repo_root, path, parents);
        Err(ContextPatchError::new(
            "guarded directory creation requires descriptor-relative operations",
        ))
    }
}

#[cfg(unix)]
fn create_directory_unix(
    authority: crate::git::RepositoryRoot<'_>,
    root: &Path,
    components: &[OsString],
    parents: bool,
) -> Result<CreateDirectorySummary, ContextPatchError> {
    use std::os::fd::AsRawFd;

    let handle = crate::fs::rooted::root_descriptor(authority)?;
    let mut current = handle.as_file().try_clone().map_err(|error| {
        ContextPatchError::new(format!("failed to retain repository root: {error}"))
    })?;
    let mut walked = PathBuf::new();
    let mut directories_created = Vec::new();

    for (index, component) in components.iter().enumerate() {
        walked.push(component);
        // For messages only. Access goes through `current`, never through this path.
        let absolute = root.join(&walked);
        let is_target = index + 1 == components.len();
        let name = std::ffi::CString::new(component.as_encoded_bytes())
            .map_err(|_| ContextPatchError::new("target path must not contain an embedded NUL byte"))?;

        match component_kind(&current, &name, &absolute)? {
            Some(kind) if kind == libc::S_IFLNK => {
                // A symlink is refused rather than resolved. Following it would mean leaving the descriptor
                // chain, and whether its target happens to land back inside the tree is not the question.
                return Err(ContextPatchError::new(format!(
                    "target path {} is outside repository root {}",
                    absolute.display(),
                    root.display()
                )));
            }
            Some(kind) if kind != libc::S_IFDIR => {
                return Err(ContextPatchError::new(format!(
                    "target path {} is not a directory",
                    absolute.display()
                )));
            }
            Some(_) => {
                if is_target {
                    return Err(ContextPatchError::new(format!(
                        "target directory {} already exists",
                        absolute.display()
                    )));
                }
                current = descend(&current, &name, &absolute)?;
                continue;
            }
            None => {}
        }

        if !parents && !is_target {
            return Err(ContextPatchError::new(format!(
                "failed to resolve parent directory {}: missing directory",
                absolute.display()
            )));
        }

        let status = unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o777) };
        if status != 0 {
            return Err(ContextPatchError::new(format!(
                "failed to create directory {}: {}",
                absolute.display(),
                std::io::Error::last_os_error()
            )));
        }
        directories_created.push(absolute.clone());
        current = descend(&current, &name, &absolute)?;
    }

    Ok(CreateDirectorySummary {
        path: root.join(&walked),
        directories_created,
    })
}

/// The file type of one entry relative to a descriptor, or `None` when it does not exist.
#[cfg(unix)]
fn component_kind(
    parent: &fs::File,
    name: &std::ffi::CStr,
    absolute: &Path,
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
        if error.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(ContextPatchError::new(format!(
            "failed to resolve directory component {}: {error}",
            absolute.display()
        )));
    }
    Ok(Some(unsafe { status.assume_init() }.st_mode & libc::S_IFMT))
}

#[cfg(unix)]
fn descend(
    parent: &fs::File,
    name: &std::ffi::CStr,
    absolute: &Path,
) -> Result<fs::File, ContextPatchError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if descriptor < 0 {
        return Err(ContextPatchError::new(format!(
            "failed to resolve directory component {}: {}",
            absolute.display(),
            std::io::Error::last_os_error()
        )));
    }
    Ok(unsafe { fs::File::from_raw_fd(descriptor) })
}

fn safe_relative_components(root: &Path, path: &Path) -> Result<Vec<OsString>, ContextPatchError> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root).map_err(|_| {
            ContextPatchError::new(format!(
                "target path {} is outside repository root {}",
                path.display(),
                root.display()
            ))
        })?
    } else {
        path
    };

    let mut components = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(name) => components.push(name.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ContextPatchError::new(format!(
                    "target path {} is outside repository root {}",
                    path.display(),
                    root.display()
                )));
            }
        }
    }

    if components.is_empty() {
        return Err(ContextPatchError::new("target path has no directory name"));
    }

    Ok(components)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::create_directory_in_root;

    #[cfg(unix)]
    fn open_directory(path: &Path) -> fs::File {
        use std::os::unix::fs::OpenOptionsExt;

        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
            .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn an_intermediate_symlink_is_refused_rather_than_followed() {
        // The walk is descriptor-relative, so a symlink at any component is refused instead of resolved.
        // An in-tree link is refused too: whether its target lands back inside the tree is not the question.
        let root = temp_root("create-dir-symlink");
        let outside = temp_root("create-dir-symlink-outside");
        fs::create_dir_all(outside.join("captured")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
        fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("alias")).unwrap();

        for link in ["escape", "alias"] {
            let error = create_directory_in_root(&root, &Path::new(link).join("child"), true)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("is outside repository root"),
                "{link}: {error}"
            );
        }

        // Nothing was created through either link.
        assert!(!outside.join("child").exists());
        assert!(!root.join("real").join("child").exists());
    }

    #[cfg(unix)]
    #[test]
    fn an_anchored_root_creates_in_the_anchored_directory_after_the_name_is_replaced() {
        // The case the descriptor exists for. The root is moved aside and a different directory takes its
        // name; creation must land in the directory that was anchored, and the replacement must be untouched.
        let root = temp_root("create-dir-swap");
        let directory = open_directory(&root);
        let anchored = contextpatch_root(&root, &directory);

        let holder = temp_root("create-dir-swap-holder");
        let moved = holder.join("anchored");
        fs::rename(&root, &moved).unwrap();
        let replacement = holder.join("replacement");
        fs::create_dir(&replacement).unwrap();
        std::os::unix::fs::symlink(&replacement, &root).unwrap();

        create_directory_in_root(anchored, Path::new("generated/nested"), true).unwrap();

        assert!(moved.join("generated").join("nested").is_dir());
        assert!(
            !replacement.join("generated").exists(),
            "the replacement repository must be untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn creation_reaches_only_the_named_repository() {
        // Two repositories with identical layouts. Creating through one must not touch the other, and an
        // outside repository is never a candidate.
        let target = temp_root("create-dir-target");
        let sibling = temp_root("create-dir-sibling");
        let outside = temp_root("create-dir-outside");
        for root in [&target, &sibling, &outside] {
            fs::create_dir(root.join("existing")).unwrap();
        }

        create_directory_in_root(&target, Path::new("generated/deep"), true).unwrap();

        assert!(target.join("generated").join("deep").is_dir());
        for root in [&sibling, &outside] {
            assert!(!root.join("generated").exists());
            assert!(root.join("existing").is_dir());
        }

        // And the reverse direction, so neither assertion passes by accident of ordering.
        create_directory_in_root(&sibling, Path::new("generated/deep"), true).unwrap();
        assert!(sibling.join("generated").join("deep").is_dir());
        assert!(!outside.join("generated").exists());
    }

    #[cfg(unix)]
    fn contextpatch_root<'a>(
        path: &'a Path,
        directory: &'a fs::File,
    ) -> crate::git::RepositoryRoot<'a> {
        crate::git::RepositoryRoot::anchored(path, directory)
    }

    #[test]
    fn creates_leaf_directory() {
        let root = temp_root("creates_leaf_directory");

        let summary = create_directory_in_root(&root, Path::new("native"), false).unwrap();

        assert!(summary.path.is_dir());
        assert_eq!(summary.directories_created.len(), 1);
        assert!(root.join("native").is_dir());
    }

    #[test]
    fn creates_nested_directories_when_parents_enabled() {
        let root = temp_root("creates_nested_directories_when_parents_enabled");

        let summary =
            create_directory_in_root(&root, Path::new("native-plugins/background-audio"), true)
                .unwrap();

        assert!(summary.path.is_dir());
        assert_eq!(summary.directories_created.len(), 2);
        assert!(root.join("native-plugins/background-audio").is_dir());
    }

    #[test]
    fn refuses_missing_parent_when_parents_disabled() {
        let root = temp_root("refuses_missing_parent_when_parents_disabled");

        let error = create_directory_in_root(&root, Path::new("missing/child"), false).unwrap_err();

        assert!(error
            .to_string()
            .contains("failed to resolve parent directory"));
        assert!(!root.join("missing").exists());
    }

    #[test]
    fn refuses_existing_directory() {
        let root = temp_root("refuses_existing_directory");
        fs::create_dir(root.join("existing")).unwrap();

        let error = create_directory_in_root(&root, Path::new("existing"), false).unwrap_err();

        assert!(error.to_string().contains("already exists"));
    }

    #[test]
    fn refuses_existing_file_component() {
        let root = temp_root("refuses_existing_file_component");
        fs::write(root.join("file"), "content").unwrap();

        let error = create_directory_in_root(&root, Path::new("file/child"), true).unwrap_err();

        assert!(error.to_string().contains("not a directory"));
    }

    #[test]
    fn refuses_paths_outside_root() {
        let root = temp_root("refuses_paths_outside_root");
        let outside = std::env::temp_dir().join(format!(
            "contextpatch-create-directory-outside-{}",
            std::process::id()
        ));

        let error = create_directory_in_root(&root, &outside, true).unwrap_err();

        assert!(error.to_string().contains("outside repository root"));
        assert!(!outside.exists());
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-create-directory-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }
}
