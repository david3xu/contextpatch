//! Filesystem primitives that act through a repository root's own authority.
//!
//! Every function here takes a [`RepositoryRoot`] rather than a path, and each one asks the root how to
//! reach the entry instead of deciding for itself. That is the whole point: a caller that has been handed
//! descriptor authority cannot accidentally fall back to a name, because there is no name-shaped argument
//! to pass.
//!
//! The two authorities deliberately differ, in the same way and for the same reason as Git path
//! confinement. A path-backed root reaches its entries by joining and calling `std::fs`, which follows
//! symlinks exactly as it does today; the byte-for-byte behavior of every existing path-backed caller is
//! preserved, which is why the compatibility branch is kept rather than unified away. An anchored root
//! walks components from the retained descriptor with no-follow descriptor-relative calls and refuses a
//! symlink at any component, because following one would mean resolving a name again and the descriptor
//! exists precisely to avoid that. Anchored roots are therefore stricter, and where that shows through it
//! is asserted in both directions rather than left to be discovered.
//!
//! Low-level mutations return `std::io::Error` rather than a worded refusal. Their callers already
//! interpolate the I/O error into refusal text they own, so handing the error back unworded is what keeps
//! those strings identical while the operation underneath them changes.

#[cfg(unix)]
use std::fs::File;
use std::path::Path;

use crate::error::ContextPatchError;
use crate::git::root::RepositoryRoot;

/// What one entry is, inspected without following a final symlink.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootedEntryKind {
    RegularFile,
    Directory,
    Symlink,
    Other,
}

impl RootedEntryKind {
    #[cfg(unix)]
    fn from_mode(mode: libc::mode_t) -> Self {
        match mode & libc::S_IFMT {
            libc::S_IFREG => Self::RegularFile,
            libc::S_IFDIR => Self::Directory,
            libc::S_IFLNK => Self::Symlink,
            _ => Self::Other,
        }
    }

    fn from_file_type(file_type: std::fs::FileType) -> Self {
        if file_type.is_symlink() {
            Self::Symlink
        } else if file_type.is_file() {
            Self::RegularFile
        } else if file_type.is_dir() {
            Self::Directory
        } else {
            Self::Other
        }
    }
}

/// One directory entry, named relative to the repository root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootedDirEntry {
    /// Path relative to the repository root, with `/` separators.
    pub relative: String,
    /// The entry's type, established without following a final symlink.
    pub kind: RootedEntryKind,
}

/// Inspect one entry without following a final symlink, reporting absence as `None`.
pub fn entry_kind(
    root: RepositoryRoot<'_>,
    relative: &str,
) -> Result<Option<RootedEntryKind>, ContextPatchError> {
    #[cfg(unix)]
    if let Some(directory) = root.directory() {
        let Some(parent) = open_parent(directory, relative)? else {
            return Ok(None);
        };
        let leaf = leaf_name(relative)?;
        return Ok(stat_at(&parent, &leaf, relative)?.map(RootedEntryKind::from_mode));
    }

    match std::fs::symlink_metadata(joined(root, relative)) {
        Ok(metadata) => Ok(Some(RootedEntryKind::from_file_type(metadata.file_type()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ContextPatchError::new(format!(
            "failed to inspect `{relative}`: {error}"
        ))),
    }
}

/// Whether one entry is a regular file.
///
/// The path-backed branch follows symlinks, matching `Path::is_file`. The anchored branch does not, so a
/// symlink is reported as not a regular file rather than as whatever it points at.
pub fn is_regular_file(root: RepositoryRoot<'_>, relative: &str) -> Result<bool, ContextPatchError> {
    #[cfg(unix)]
    if root.is_anchored() {
        return Ok(entry_kind(root, relative)? == Some(RootedEntryKind::RegularFile));
    }

    Ok(joined(root, relative).is_file())
}

/// Whether one entry is a directory.
pub fn is_directory(root: RepositoryRoot<'_>, relative: &str) -> Result<bool, ContextPatchError> {
    #[cfg(unix)]
    if root.is_anchored() {
        return Ok(entry_kind(root, relative)? == Some(RootedEntryKind::Directory));
    }

    Ok(joined(root, relative).is_dir())
}

/// Whether one entry exists at all.
pub fn exists(root: RepositoryRoot<'_>, relative: &str) -> Result<bool, ContextPatchError> {
    #[cfg(unix)]
    if root.is_anchored() {
        return Ok(entry_kind(root, relative)?.is_some());
    }

    Ok(joined(root, relative).exists())
}

/// Open one regular file for reading, refusing anything that is not one.
///
/// The anchored branch opens the leaf relative to its parent descriptor with `O_NOFOLLOW`, so neither the
/// parent chain nor the leaf can be redirected between the check and the read.
pub fn open_regular_file(
    root: RepositoryRoot<'_>,
    relative: &str,
) -> Result<std::fs::File, ContextPatchError> {
    #[cfg(unix)]
    if let Some(directory) = root.directory() {
        let parent = open_parent(directory, relative)?.ok_or_else(|| {
            ContextPatchError::new(format!("`{relative}` is not an existing regular file"))
        })?;
        let leaf = leaf_name(relative)?;
        return open_regular_at(&parent, &leaf, relative);
    }

    let target = joined(root, relative);
    std::fs::File::open(&target).map_err(|error| {
        ContextPatchError::new(format!(
            "failed to open {} for hashing: {error}",
            target.display()
        ))
    })
}

/// Hash one regular file's contents through the same authority that will act on it.
///
/// Reading through an already-open descriptor is what makes the hash and the mutation refer to one file:
/// a replacement after this point changes the name's target, not the bytes that were hashed.
pub fn sha256(root: RepositoryRoot<'_>, relative: &str) -> Result<String, ContextPatchError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = open_regular_file(root, relative)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| ContextPatchError::new(format!("failed to hash `{relative}`: {error}")))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Rename one entry to another, both resolved through the root's authority.
///
/// The anchored branch resolves each side to its own parent descriptor and issues one `renameat`, so
/// neither parent chain can be substituted between resolving the two sides and performing the move.
pub fn rename(
    root: RepositoryRoot<'_>,
    from: &str,
    to: &str,
) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    if let Some(directory) = root.directory() {
        use std::os::fd::AsRawFd;

        let from_parent = open_parent(directory, from)
            .map_err(io_error)?
            .ok_or_else(missing_entry)?;
        let to_parent = open_parent(directory, to)
            .map_err(io_error)?
            .ok_or_else(missing_entry)?;
        let from_leaf = leaf_name(from).map_err(io_error)?;
        let to_leaf = leaf_name(to).map_err(io_error)?;

        let status = unsafe {
            libc::renameat(
                from_parent.as_raw_fd(),
                from_leaf.as_ptr(),
                to_parent.as_raw_fd(),
                to_leaf.as_ptr(),
            )
        };
        if status != 0 {
            return Err(std::io::Error::last_os_error());
        }
        return Ok(());
    }

    std::fs::rename(joined(root, from), joined(root, to))
}

/// Remove one file, resolved through the root's authority.
pub fn remove_file(root: RepositoryRoot<'_>, relative: &str) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    if let Some(directory) = root.directory() {
        return unlink_at(directory, relative, 0);
    }

    std::fs::remove_file(joined(root, relative))
}

/// Remove one empty directory, resolved through the root's authority.
pub fn remove_dir(root: RepositoryRoot<'_>, relative: &str) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    if let Some(directory) = root.directory() {
        return unlink_at(directory, relative, libc::AT_REMOVEDIR);
    }

    std::fs::remove_dir(joined(root, relative))
}

/// Remove one directory and everything beneath it, resolved through the root's authority.
///
/// The anchored branch descends with descriptor-relative reads and refuses a symlink anywhere in the tree,
/// so a recursive removal cannot be redirected out of the repository part way down.
pub fn remove_dir_all(root: RepositoryRoot<'_>, relative: &str) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    if root.is_anchored() {
        let entries = read_dir(root, relative).map_err(io_error)?;
        for entry in entries {
            match entry.kind {
                RootedEntryKind::Directory => remove_dir_all(root, &entry.relative)?,
                _ => remove_file(root, &entry.relative)?,
            }
        }
        return remove_dir(root, relative);
    }

    std::fs::remove_dir_all(joined(root, relative))
}

/// List one directory's entries, named relative to the repository root.
pub fn read_dir(
    root: RepositoryRoot<'_>,
    relative: &str,
) -> Result<Vec<RootedDirEntry>, ContextPatchError> {
    #[cfg(unix)]
    if let Some(directory) = root.directory() {
        return read_dir_anchored(directory, relative);
    }

    let target = joined(root, relative);
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&target)
        .map_err(|error| ContextPatchError::new(format!("failed to read `{relative}`: {error}")))?
    {
        let entry = entry.map_err(|error| {
            ContextPatchError::new(format!("failed to read entry under `{relative}`: {error}"))
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        let child = format!("{relative}/{name}");
        let file_type = entry.file_type().map_err(|error| {
            ContextPatchError::new(format!("failed to inspect `{child}`: {error}"))
        })?;
        entries.push(RootedDirEntry {
            relative: child,
            kind: RootedEntryKind::from_file_type(file_type),
        });
    }
    Ok(entries)
}

/// Whether one directory holds no entries.
pub fn directory_is_empty(
    root: RepositoryRoot<'_>,
    relative: &str,
) -> Result<bool, ContextPatchError> {
    Ok(read_dir(root, relative)?.is_empty())
}

fn joined(root: RepositoryRoot<'_>, relative: &str) -> std::path::PathBuf {
    root.logical_path().join(Path::new(relative))
}

#[cfg(unix)]
fn io_error(error: ContextPatchError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

#[cfg(unix)]
fn missing_entry() -> std::io::Error {
    std::io::Error::from(std::io::ErrorKind::NotFound)
}

/// Open the directory holding one entry, walking from the retained descriptor.
///
/// Each component is opened relative to the descriptor the previous step returned, so the parent chain
/// cannot be substituted part way through and the root's name is never resolved again. A missing parent is
/// reported as `None`, because several callers treat absence as an answer rather than a failure.
#[cfg(unix)]
fn open_parent(directory: &File, relative: &str) -> Result<Option<File>, ContextPatchError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let components = Path::new(relative).components().collect::<Vec<_>>();
    if components.is_empty() {
        return Err(ContextPatchError::new(
            "path must be a normalized repository-relative path",
        ));
    }

    let mut current = directory.try_clone().map_err(|error| {
        ContextPatchError::new(format!("failed to retain repository root: {error}"))
    })?;
    let mut walked = std::path::PathBuf::new();
    for component in &components[..components.len() - 1] {
        let std::path::Component::Normal(component) = component else {
            return Err(ContextPatchError::new(
                "path must be a normalized repository-relative path",
            ));
        };
        walked.push(component);
        let name = c_string(component.as_encoded_bytes())?;
        let opened = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )
        };
        if opened < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(component_failure(&current, &name, relative, &walked, error));
        }
        current = unsafe { File::from_raw_fd(opened) };
    }
    Ok(Some(current))
}

/// Explain why one descriptor-relative open failed, without leaving the parent descriptor.
///
/// `ELOOP` and `ENOTDIR` do not portably distinguish a symlink from a plain file, so the entry is
/// classified against the same parent rather than guessed from the error code.
#[cfg(unix)]
fn component_failure(
    parent: &File,
    name: &std::ffi::CStr,
    relative: &str,
    walked: &Path,
    error: std::io::Error,
) -> ContextPatchError {
    match stat_at(parent, name, relative) {
        Ok(Some(mode)) if mode & libc::S_IFMT == libc::S_IFLNK => ContextPatchError::new(format!(
            "path `{relative}` contains symlink component `{}`",
            walked.display()
        )),
        Ok(Some(_)) => ContextPatchError::new(format!(
            "path `{relative}` component `{}` is not a directory",
            walked.display()
        )),
        Ok(None) => ContextPatchError::new(format!(
            "failed to inspect path component `{}`: {error}",
            walked.display()
        )),
        Err(classified) => classified,
    }
}

#[cfg(unix)]
fn stat_at(
    parent: &File,
    name: &std::ffi::CStr,
    relative: &str,
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
            "failed to inspect `{relative}`: {error}"
        )));
    }
    Ok(Some(unsafe { status.assume_init() }.st_mode))
}

#[cfg(unix)]
fn open_regular_at(
    parent: &File,
    leaf: &std::ffi::CStr,
    relative: &str,
) -> Result<File, ContextPatchError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let descriptor = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if descriptor < 0 {
        let error = std::io::Error::last_os_error();
        let message = if error.raw_os_error() == Some(libc::ELOOP) {
            format!("`{relative}` contains a symlink component")
        } else {
            format!("failed to open `{relative}` without following symlinks: {error}")
        };
        return Err(ContextPatchError::new(message));
    }
    let file = unsafe { File::from_raw_fd(descriptor) };
    let metadata = file
        .metadata()
        .map_err(|error| ContextPatchError::new(format!("failed to inspect `{relative}`: {error}")))?;
    if !metadata.is_file() {
        return Err(ContextPatchError::new(format!(
            "`{relative}` is not an existing regular file"
        )));
    }
    Ok(file)
}

#[cfg(unix)]
fn unlink_at(
    directory: &File,
    relative: &str,
    flags: libc::c_int,
) -> Result<(), std::io::Error> {
    use std::os::fd::AsRawFd;

    let parent = open_parent(directory, relative)
        .map_err(io_error)?
        .ok_or_else(missing_entry)?;
    let leaf = leaf_name(relative).map_err(io_error)?;
    let status = unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), flags) };
    if status != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn read_dir_anchored(
    directory: &File,
    relative: &str,
) -> Result<Vec<RootedDirEntry>, ContextPatchError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    let parent = open_parent(directory, relative)?.ok_or_else(|| {
        ContextPatchError::new(format!("failed to read `{relative}`: missing directory"))
    })?;
    let leaf = leaf_name(relative)?;
    let opened = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if opened < 0 {
        let error = std::io::Error::last_os_error();
        return Err(ContextPatchError::new(format!(
            "failed to read `{relative}`: {error}"
        )));
    }
    let handle = unsafe { File::from_raw_fd(opened) };

    let mut entries = Vec::new();
    for name in list_names(&handle, relative)? {
        let child = format!("{relative}/{name}");
        let name = c_string(name.as_bytes())?;
        let Some(mode) = stat_at(&handle, &name, &child)? else {
            // The entry left between listing and inspection, so there is nothing to report or remove.
            continue;
        };
        entries.push(RootedDirEntry {
            relative: child,
            kind: RootedEntryKind::from_mode(mode),
        });
    }
    Ok(entries)
}

/// Read one open directory's entry names, skipping `.` and `..`.
///
/// `fdopendir` takes ownership of the descriptor it is given, so the handle is duplicated first and the
/// stream closes the copy. A null entry ends the stream or reports a failure, and the two are told apart
/// through `errno` exactly as the guarded directory listing already does, so a truncated enumeration is
/// refused rather than mistaken for an empty tail.
#[cfg(unix)]
fn list_names(handle: &File, relative: &str) -> Result<Vec<String>, ContextPatchError> {
    use std::os::fd::AsRawFd;

    let duplicated = unsafe { libc::dup(handle.as_raw_fd()) };
    if duplicated < 0 {
        return Err(ContextPatchError::new(format!(
            "failed to read `{relative}`: {}",
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
            "failed to read `{relative}`: {error}"
        )));
    }

    let mut names = Vec::new();
    loop {
        errno::set_errno(errno::Errno(0));
        let entry = unsafe { libc::readdir(stream) };
        if entry.is_null() {
            let failure = errno::errno();
            if failure.0 != 0 {
                unsafe {
                    libc::closedir(stream);
                }
                return Err(ContextPatchError::new(format!(
                    "failed to read entry under `{relative}`: {}",
                    std::io::Error::from_raw_os_error(failure.0)
                )));
            }
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
        let bytes = name.to_bytes();
        if matches!(bytes, b"." | b"..") {
            continue;
        }
        names.push(String::from_utf8_lossy(bytes).to_string());
    }
    let closed = unsafe { libc::closedir(stream) };
    if closed != 0 {
        return Err(ContextPatchError::new(format!(
            "failed to read `{relative}`: {}",
            std::io::Error::last_os_error()
        )));
    }

    names.sort();
    Ok(names)
}

#[cfg(unix)]
fn leaf_name(relative: &str) -> Result<std::ffi::CString, ContextPatchError> {
    let leaf = Path::new(relative)
        .file_name()
        .ok_or_else(|| ContextPatchError::new("path has no file name"))?;
    c_string(leaf.as_encoded_bytes())
}

#[cfg(unix)]
fn c_string(bytes: &[u8]) -> Result<std::ffi::CString, ContextPatchError> {
    std::ffi::CString::new(bytes)
        .map_err(|_| ContextPatchError::new("path must not contain an embedded NUL byte"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::fs::hash::sha256_bytes;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-rooted-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn open_directory(path: &Path) -> File {
        use std::os::unix::fs::OpenOptionsExt;

        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)
            .unwrap()
    }

    /// Move the anchored directory aside and leave a different directory under its name.
    ///
    /// This is the substitution a retained descriptor exists to defeat, and it is done by renaming rather
    /// than by deleting so the anchored directory keeps its contents and stays reachable through the
    /// descriptor alone.
    fn substitute_root(root: &Path, name: &str) -> (PathBuf, PathBuf) {
        let holder = test_root(name);
        let moved = holder.join("anchored");
        fs::rename(root, &moved).unwrap();
        let replacement = holder.join("replacement");
        fs::create_dir(&replacement).unwrap();
        symlink(&replacement, root).unwrap();
        (moved, replacement)
    }

    #[test]
    fn a_symlink_leaf_is_a_regular_file_by_name_and_not_by_descriptor() {
        let root = test_root("symlink-leaf");
        fs::write(root.join("real.txt"), "content\n").unwrap();
        symlink("real.txt", root.join("link.txt")).unwrap();
        let directory = open_directory(&root);

        // The name follows the link and answers about its target.
        assert!(is_regular_file(RepositoryRoot::from_path(&root), "link.txt").unwrap());
        // The descriptor does not, so a link cannot stand in for the file it points at.
        let anchored = RepositoryRoot::anchored(&root, &directory);
        assert!(!is_regular_file(anchored, "link.txt").unwrap());
        assert_eq!(
            entry_kind(anchored, "link.txt").unwrap(),
            Some(RootedEntryKind::Symlink)
        );
    }

    #[test]
    fn an_intermediate_symlink_cannot_redirect_an_anchored_read() {
        let root = test_root("symlink-parent");
        let outside = test_root("symlink-parent-outside");
        fs::write(outside.join("secret.txt"), "outside\n").unwrap();
        symlink(&outside, root.join("escape")).unwrap();
        let directory = open_directory(&root);

        let error = sha256(
            RepositoryRoot::anchored(&root, &directory),
            "escape/secret.txt",
        )
        .unwrap_err()
        .to_string();

        assert!(
            error.contains("contains symlink component `escape`"),
            "{error}"
        );
        assert!(outside.join("secret.txt").exists());
    }

    #[test]
    fn replacing_the_root_does_not_redirect_an_anchored_read() {
        let root = test_root("swap-read");
        fs::write(root.join("tracked.txt"), "anchored\n").unwrap();
        let directory = open_directory(&root);
        let (_moved, replacement) = substitute_root(&root, "swap-read-holder");
        fs::write(replacement.join("tracked.txt"), "replacement\n").unwrap();

        // The descriptor still reaches the directory that was anchored.
        assert_eq!(
            sha256(RepositoryRoot::anchored(&root, &directory), "tracked.txt").unwrap(),
            sha256_bytes(b"anchored\n")
        );
        // The name now reaches the replacement, which is the substitution the descriptor prevents.
        assert_eq!(
            sha256(RepositoryRoot::from_path(&root), "tracked.txt").unwrap(),
            sha256_bytes(b"replacement\n")
        );
    }

    #[test]
    fn replacing_the_root_does_not_redirect_an_anchored_deletion() {
        let root = test_root("swap-delete");
        fs::write(root.join("doomed.txt"), "anchored\n").unwrap();
        let directory = open_directory(&root);
        let (moved, replacement) = substitute_root(&root, "swap-delete-holder");
        fs::write(replacement.join("doomed.txt"), "replacement\n").unwrap();

        remove_file(RepositoryRoot::anchored(&root, &directory), "doomed.txt").unwrap();

        assert!(!moved.join("doomed.txt").exists());
        assert!(
            replacement.join("doomed.txt").exists(),
            "the replacement repository must be untouched"
        );
    }

    #[test]
    fn replacing_the_root_does_not_redirect_an_anchored_rename() {
        let root = test_root("swap-rename");
        fs::write(root.join("source.txt"), "anchored\n").unwrap();
        let directory = open_directory(&root);
        let (moved, replacement) = substitute_root(&root, "swap-rename-holder");
        fs::write(replacement.join("source.txt"), "replacement\n").unwrap();

        rename(
            RepositoryRoot::anchored(&root, &directory),
            "source.txt",
            "moved.txt",
        )
        .unwrap();

        assert!(!moved.join("source.txt").exists());
        assert_eq!(
            fs::read_to_string(moved.join("moved.txt")).unwrap(),
            "anchored\n"
        );
        assert!(
            replacement.join("source.txt").exists(),
            "the replacement repository must be untouched"
        );
        assert!(!replacement.join("moved.txt").exists());
    }

    #[test]
    fn replacing_the_root_does_not_redirect_an_anchored_listing() {
        let root = test_root("swap-list");
        fs::create_dir(root.join("generated")).unwrap();
        fs::write(root.join("generated").join("anchored.txt"), "a\n").unwrap();
        let directory = open_directory(&root);
        let (_moved, replacement) = substitute_root(&root, "swap-list-holder");
        fs::create_dir(replacement.join("generated")).unwrap();
        fs::write(replacement.join("generated").join("other.txt"), "b\n").unwrap();

        let entries = read_dir(RepositoryRoot::anchored(&root, &directory), "generated").unwrap();

        assert_eq!(
            entries,
            vec![RootedDirEntry {
                relative: "generated/anchored.txt".to_string(),
                kind: RootedEntryKind::RegularFile,
            }]
        );
    }

    #[test]
    fn a_recursive_removal_does_not_follow_a_symlink_out_of_the_tree() {
        let root = test_root("recursive-symlink");
        let outside = test_root("recursive-symlink-outside");
        fs::write(outside.join("keep.txt"), "keep\n").unwrap();
        fs::create_dir(root.join("generated")).unwrap();
        fs::write(root.join("generated").join("inner.txt"), "inner\n").unwrap();
        symlink(&outside, root.join("generated").join("escape")).unwrap();
        let directory = open_directory(&root);

        remove_dir_all(RepositoryRoot::anchored(&root, &directory), "generated").unwrap();

        // The link is removed; what it pointed at is not.
        assert!(!root.join("generated").exists());
        assert!(outside.join("keep.txt").exists());
    }

    #[test]
    fn an_absent_entry_and_an_absent_parent_are_both_reported_as_absence() {
        let root = test_root("absent");
        let directory = open_directory(&root);
        let anchored = RepositoryRoot::anchored(&root, &directory);

        assert_eq!(entry_kind(anchored, "missing.txt").unwrap(), None);
        assert_eq!(entry_kind(anchored, "missing/deep.txt").unwrap(), None);
        assert!(!exists(anchored, "missing.txt").unwrap());
    }

    #[test]
    fn a_non_directory_parent_is_refused_rather_than_treated_as_absent() {
        let root = test_root("file-parent");
        fs::write(root.join("regular.txt"), "content\n").unwrap();
        let directory = open_directory(&root);

        let error = entry_kind(
            RepositoryRoot::anchored(&root, &directory),
            "regular.txt/child",
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("is not a directory"), "{error}");
    }

    #[test]
    fn emptiness_is_reported_through_the_same_authority() {
        let root = test_root("empty");
        fs::create_dir(root.join("hollow")).unwrap();
        fs::create_dir(root.join("occupied")).unwrap();
        fs::write(root.join("occupied").join("inner.txt"), "x\n").unwrap();
        let directory = open_directory(&root);
        let anchored = RepositoryRoot::anchored(&root, &directory);

        assert!(directory_is_empty(anchored, "hollow").unwrap());
        assert!(!directory_is_empty(anchored, "occupied").unwrap());
        assert!(is_directory(anchored, "hollow").unwrap());
        assert!(!is_regular_file(anchored, "hollow").unwrap());
    }

    #[test]
    fn a_path_backed_root_keeps_its_existing_behavior() {
        let root = test_root("path-backed");
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested").join("file.txt"), "content\n").unwrap();
        let named = RepositoryRoot::from_path(&root);

        assert!(is_regular_file(named, "nested/file.txt").unwrap());
        assert!(is_directory(named, "nested").unwrap());
        assert_eq!(
            sha256(named, "nested/file.txt").unwrap(),
            sha256_bytes(b"content\n")
        );

        remove_file(named, "nested/file.txt").unwrap();
        assert!(!root.join("nested").join("file.txt").exists());
        remove_dir(named, "nested").unwrap();
        assert!(!root.join("nested").exists());
    }
}
