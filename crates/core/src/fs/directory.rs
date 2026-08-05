#[cfg(unix)]
use std::collections::{BinaryHeap, VecDeque};
#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::Component;
use std::path::Path;

use crate::error::ContextPatchError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub depth: usize,
    pub kind: &'static str,
    pub size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryListing {
    pub path: String,
    pub recursive: bool,
    pub max_depth: usize,
    pub max_entries: usize,
    pub truncated: bool,
    pub entries: Vec<DirectoryEntry>,
}

pub fn list_directory_in_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    path: &Path,
    include_hidden: bool,
    recursive: bool,
    max_depth: usize,
    max_entries: usize,
) -> Result<DirectoryListing, ContextPatchError> {
    #[cfg(unix)]
    {
        let authority = repo_root.into();
        let relative = normalize_directory_path(path)?;
        let entries = list_directory_unix(
            authority,
            path,
            &relative,
            include_hidden,
            recursive,
            max_depth,
            max_entries,
        )?;

        Ok(DirectoryListing {
            path: relative,
            recursive,
            max_depth,
            max_entries,
            truncated: entries.1,
            entries: entries.0,
        })
    }

    #[cfg(not(unix))]
    {
        let _ = (
            repo_root,
            path,
            include_hidden,
            recursive,
            max_depth,
            max_entries,
        );
        Err(ContextPatchError::new(
            "guarded repository directory listing requires Unix descriptor-relative path support",
        ))
    }
}

#[cfg(unix)]
fn normalize_directory_path(path: &Path) -> Result<String, ContextPatchError> {
    if path == Path::new(".") {
        return Ok(".".to_string());
    }
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ContextPatchError::new(
            "path must be `.` or a normalized repository-relative path",
        ));
    }
    Ok(path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

#[cfg(unix)]
fn list_directory_unix(
    root: crate::git::RepositoryRoot<'_>,
    path: &Path,
    relative: &str,
    include_hidden: bool,
    recursive: bool,
    max_depth: usize,
    max_entries: usize,
) -> Result<(Vec<DirectoryEntry>, bool), ContextPatchError> {
    let target = open_directory_path(root, path, relative)?;
    let mut pending = VecDeque::from([(target, relative.to_string(), 0_usize)]);
    let mut entries = Vec::new();
    let mut truncated = false;

    'walk: while let Some((directory, display_directory, parent_depth)) = pending.pop_front() {
        let remaining = max_entries.saturating_sub(entries.len());
        let (children, directory_truncated) =
            read_directory_entries(&directory, &display_directory, include_hidden, remaining)?;

        for child in children {
            let depth = parent_depth + 1;
            let name = String::from_utf8_lossy(&child.name).to_string();
            let display_path = join_display_path(&display_directory, &name);
            entries.push(DirectoryEntry {
                name,
                path: display_path.clone(),
                depth,
                kind: child.kind,
                size_bytes: child.size_bytes,
            });

            if recursive && child.kind == "directory" && depth < max_depth {
                let child_directory = open_child_directory(&directory, &child.name, &display_path)?;
                pending.push_back((child_directory, display_path, depth));
            }
        }
        if directory_truncated {
            truncated = true;
            break 'walk;
        }
    }

    Ok((entries, truncated))
}

#[cfg(unix)]
struct RawDirectoryEntry {
    name: Vec<u8>,
    kind: &'static str,
    size_bytes: Option<u64>,
}

#[cfg(unix)]
fn open_directory_path(
    root: crate::git::RepositoryRoot<'_>,
    path: &Path,
    relative: &str,
) -> Result<fs::File, ContextPatchError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let handle = crate::fs::rooted::root_descriptor(root)?;
    let mut current = handle.as_file().try_clone().map_err(|error| {
        ContextPatchError::new(format!("failed to retain repository root: {error}"))
    })?;
    if path == Path::new(".") {
        return Ok(current);
    }

    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(ContextPatchError::new(
                "path must be `.` or a normalized repository-relative path",
            ));
        };
        let component = CString::new(component.as_bytes())
            .map_err(|_| ContextPatchError::new("path must not contain an embedded NUL byte"))?;
        let descriptor = unsafe {
            libc::openat(
                current.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            return Err(ContextPatchError::new(format!(
                "failed to open directory `{relative}` without following symlinks: {}",
                std::io::Error::last_os_error()
            )));
        }
        current = unsafe { fs::File::from_raw_fd(descriptor) };
    }
    Ok(current)
}

#[cfg(unix)]
fn read_directory_entries(
    directory: &fs::File,
    display_directory: &str,
    include_hidden: bool,
    max_entries: usize,
) -> Result<(Vec<RawDirectoryEntry>, bool), ContextPatchError> {
    use std::ffi::CStr;
    use std::os::fd::AsRawFd;

    let duplicated = unsafe { libc::dup(directory.as_raw_fd()) };
    if duplicated < 0 {
        return Err(ContextPatchError::new(format!(
            "failed to duplicate directory `{display_directory}`: {}",
            std::io::Error::last_os_error()
        )));
    }
    let stream = unsafe { libc::fdopendir(duplicated) };
    if stream.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe {
            libc::close(duplicated);
        }
        return Err(ContextPatchError::new(format!(
            "failed to read directory `{display_directory}`: {error}"
        )));
    }

    let mut names = {
        let mut names = BinaryHeap::new();
        let retained_names = max_entries.saturating_add(1);
        loop {
            errno::set_errno(errno::Errno(0));
            let entry = unsafe { libc::readdir(stream) };
            if entry.is_null() {
                if let Err(error) = check_readdir_errno(display_directory) {
                    unsafe {
                        libc::closedir(stream);
                    }
                    return Err(error);
                }
                break;
            }
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_bytes()
                .to_vec();
            if matches!(name.as_slice(), b"." | b"..") {
                continue;
            }
            if !include_hidden && name.first() == Some(&b'.') {
                continue;
            }
            retain_smallest(&mut names, name, retained_names);
        }
        names.into_vec()
    };

    let close_status = unsafe { libc::closedir(stream) };
    if close_status != 0 {
        return Err(ContextPatchError::new(format!(
            "failed to close directory `{display_directory}`: {}",
            std::io::Error::last_os_error()
        )));
    }
    names.sort();
    let truncated = names.len() > max_entries;
    names.truncate(max_entries);

    let mut entries = Vec::with_capacity(names.len());
    for name in names {
        let c_name =
            std::ffi::CString::new(name.clone()).expect("directory entry names cannot contain NUL");
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let status = unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                c_name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if status != 0 {
            return Err(ContextPatchError::new(format!(
                "directory entry changed while listing `{display_directory}`: {}",
                std::io::Error::last_os_error()
            )));
        }
        let stat = unsafe { stat.assume_init() };
        let file_type = stat.st_mode & libc::S_IFMT;
        let kind = if file_type == libc::S_IFLNK {
            "symlink"
        } else if file_type == libc::S_IFDIR {
            "directory"
        } else if file_type == libc::S_IFREG {
            "file"
        } else {
            "other"
        };
        entries.push(RawDirectoryEntry {
            name,
            kind,
            size_bytes: (kind == "file").then_some(stat.st_size.max(0) as u64),
        });
    }
    Ok((entries, truncated))
}

#[cfg(unix)]
fn check_readdir_errno(display_directory: &str) -> Result<(), ContextPatchError> {
    let error = errno::errno();
    if error.0 == 0 {
        Ok(())
    } else {
        Err(ContextPatchError::new(format!(
            "failed while enumerating directory `{display_directory}`: {}",
            std::io::Error::from_raw_os_error(error.0)
        )))
    }
}

#[cfg(unix)]
fn open_child_directory(
    parent: &fs::File,
    name: &[u8],
    display_path: &str,
) -> Result<fs::File, ContextPatchError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let name =
        std::ffi::CString::new(name).expect("directory entry names cannot contain embedded NUL");
    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if descriptor < 0 {
        return Err(ContextPatchError::new(format!(
            "directory `{display_path}` changed while preparing recursive traversal: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(unsafe { fs::File::from_raw_fd(descriptor) })
}

fn join_display_path(parent: &str, name: &str) -> String {
    if parent == "." {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

fn retain_smallest<T: Ord>(items: &mut BinaryHeap<T>, value: T, limit: usize) {
    if limit == 0 {
        return;
    }
    if items.len() < limit {
        items.push(value);
    } else if items.peek().is_some_and(|largest| &value < largest) {
        items.pop();
        items.push(value);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn recursively_lists_without_following_symlinks() {
        let root = temp_root("directory-list");
        fs::create_dir_all(root.join("task/nested")).unwrap();
        fs::write(root.join("task/a.txt"), "a").unwrap();
        fs::write(root.join("task/nested/b.txt"), "bb").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("../nested", root.join("task/link")).unwrap();

        let listing = list_directory_in_root(&root, Path::new("task"), true, true, 3, 100).unwrap();

        assert!(listing
            .entries
            .iter()
            .any(|entry| entry.path == "task/a.txt"));
        assert!(listing
            .entries
            .iter()
            .any(|entry| entry.path == "task/nested/b.txt"));
        #[cfg(unix)]
        assert_eq!(
            listing
                .entries
                .iter()
                .find(|entry| entry.path == "task/link")
                .unwrap()
                .kind,
            "symlink"
        );
        assert!(!listing
            .entries
            .iter()
            .any(|entry| entry.path.starts_with("task/link/")));
    }

    #[cfg(unix)]
    #[test]
    fn directory_scan_retains_only_the_smallest_bounded_candidates() {
        let root = temp_root("bounded-directory-list");
        fs::create_dir(root.join("task")).unwrap();
        for name in ["z.txt", "b.txt", "a.txt", "c.txt", ".hidden"] {
            fs::write(root.join("task").join(name), name).unwrap();
        }
        let directory = open_directory_path((&root).into(), Path::new("task"), "task").unwrap();

        let (entries, truncated) = read_directory_entries(&directory, "task", false, 2).unwrap();
        let names = entries
            .iter()
            .map(|entry| String::from_utf8_lossy(&entry.name).to_string())
            .collect::<Vec<_>>();

        assert_eq!(names, ["a.txt", "b.txt"]);
        assert!(truncated);
    }

    #[cfg(unix)]
    #[test]
    fn readdir_errors_are_not_treated_as_end_of_directory() {
        errno::set_errno(errno::Errno(libc::EIO));
        let error = check_readdir_errno("task").unwrap_err();
        assert!(error.to_string().contains("enumerating directory `task`"));
        assert!(error.to_string().contains("Input/output error"));

        errno::set_errno(errno::Errno(0));
        check_readdir_errno("task").unwrap();
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("contextpatch-{name}-{unique}"));
        fs::create_dir_all(&root).unwrap();
        root
    }
}
