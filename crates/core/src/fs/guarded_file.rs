//! Descriptor-relative repository file access that never follows path symlinks.

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::path::Component;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use sha2::{Digest, Sha256};

use crate::error::ContextPatchError;
#[cfg(unix)]
use crate::fs::file_identity::FileIdentity;

/// Permission bits carried across an in-place replacement. Four octal digits so
/// setuid, setgid, and sticky survive alongside the ordinary access bits, which
/// matches the four-digit mode reported by the executable-bit workflow.
#[cfg(unix)]
const PRESERVED_PERMISSION_BITS: u32 = 0o7777;

/// Creation mode requested for a fresh temporary file, before the process umask
/// narrows it. Replacements overwrite this with the target's own bits.
#[cfg(unix)]
const TEMP_FILE_CREATION_MODE: libc::c_uint = 0o666;

/// Distinct temporary names tried before a guarded write reports failure.
#[cfg(unix)]
const TEMP_FILE_ATTEMPT_LIMIT: u32 = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardedPathKind {
    RegularFile,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug)]
pub struct GuardedPathInspection {
    pub kind: GuardedPathKind,
    pub size_bytes: u64,
    pub mode: Option<u32>,
    pub symlink_target: Option<PathBuf>,
    pub symlink_resolves_inside_root: bool,
    pub regular_file: Option<GuardedRegularFile>,
}

#[derive(Debug)]
pub struct GuardedRegularFile {
    root: PathBuf,
    relative: PathBuf,
    file: File,
    #[cfg(unix)]
    parent: File,
    #[cfg(unix)]
    leaf: std::ffi::CString,
    /// The root descriptor this file was reached through, retained so revalidation can re-walk from the
    /// same authority rather than resolving the root's name again.
    #[cfg(unix)]
    root_directory: File,
}

#[derive(Debug)]
pub struct GuardedFileRead {
    pub total_bytes: u64,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub line_count: Option<u64>,
}

impl GuardedRegularFile {
    pub fn file(&self) -> &File {
        &self.file
    }

    pub fn repository_root(&self) -> &Path {
        &self.root
    }

    pub fn relative_path(&self) -> &Path {
        &self.relative
    }

    pub fn target_path(&self) -> PathBuf {
        self.root.join(&self.relative)
    }

    pub fn read_all(&self) -> Result<Vec<u8>, ContextPatchError> {
        self.read_bounded(u64::MAX).map(|(bytes, _)| bytes)
    }

    pub fn size_bytes(&self) -> Result<u64, ContextPatchError> {
        self.file
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|error| {
                ContextPatchError::new(format!(
                    "failed to inspect `{}`: {error}",
                    self.relative.display()
                ))
            })
    }

    pub fn read_range_with_digest(
        &self,
        offset: u64,
        max_bytes: u64,
    ) -> Result<GuardedFileRead, ContextPatchError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;

            let before = FileContentSnapshot::capture(&self.file, &self.relative)?;
            if offset > before.size_bytes {
                return Err(ContextPatchError::new(format!(
                    "offset {offset} is past end of `{}` ({} bytes)",
                    self.relative.display(),
                    before.size_bytes
                )));
            }
            let requested_bytes = (before.size_bytes - offset).min(max_bytes);
            let capacity = usize::try_from(requested_bytes).map_err(|_| {
                ContextPatchError::new(format!(
                    "requested byte range for `{}` is too large to retain in memory",
                    self.relative.display()
                ))
            })?;
            let range_end = offset + requested_bytes;
            let mut bytes = Vec::with_capacity(capacity);
            let mut hasher = Sha256::new();
            let mut lines = Utf8LineCounter::new();
            let mut current_offset = 0_u64;
            let mut buffer = [0_u8; 64 * 1024];

            while current_offset < before.size_bytes {
                let remaining = before.size_bytes - current_offset;
                let requested = usize::try_from(remaining)
                    .unwrap_or(usize::MAX)
                    .min(buffer.len());
                let count = self
                    .file
                    .read_at(&mut buffer[..requested], current_offset)
                    .map_err(|error| {
                        ContextPatchError::new(format!(
                            "failed to inspect `{}`: {error}",
                            self.relative.display()
                        ))
                    })?;
                if count == 0 {
                    return Err(ContextPatchError::new(format!(
                        "`{}` changed while it was being inspected",
                        self.relative.display()
                    )));
                }

                let chunk = &buffer[..count];
                hasher.update(chunk);
                lines.update(chunk);
                let chunk_end = current_offset + count as u64;
                if current_offset < range_end && chunk_end > offset {
                    let retained_start = usize::try_from(offset.saturating_sub(current_offset))
                        .expect("retained range starts within one read buffer");
                    let retained_end = usize::try_from(range_end.min(chunk_end) - current_offset)
                        .expect("retained range ends within one read buffer");
                    bytes.extend_from_slice(&chunk[retained_start..retained_end]);
                }
                current_offset = chunk_end;
            }

            let after = FileContentSnapshot::capture(&self.file, &self.relative)?;
            if before != after {
                return Err(ContextPatchError::new(format!(
                    "`{}` changed while it was being inspected",
                    self.relative.display()
                )));
            }
            self.revalidate_current_path()?;

            Ok(GuardedFileRead {
                total_bytes: before.size_bytes,
                bytes,
                sha256: format!("{:x}", hasher.finalize()),
                line_count: lines.finish(),
            })
        }

        #[cfg(not(unix))]
        {
            let _ = (offset, max_bytes);
            Err(unsupported_guarded_filesystem())
        }
    }

    pub fn read_range(&self, offset: u64, max_bytes: u64) -> Result<Vec<u8>, ContextPatchError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;

            let total_bytes = self.size_bytes()?;
            if offset > total_bytes {
                return Err(ContextPatchError::new(format!(
                    "offset {offset} is past end of `{}` ({total_bytes} bytes)",
                    self.relative.display()
                )));
            }
            let requested_bytes = (total_bytes - offset).min(max_bytes);
            let capacity = usize::try_from(requested_bytes).map_err(|_| {
                ContextPatchError::new(format!(
                    "requested byte range for `{}` is too large to retain in memory",
                    self.relative.display()
                ))
            })?;
            let mut bytes = Vec::with_capacity(capacity);
            let mut current_offset = offset;
            let end_offset = offset + requested_bytes;
            let mut buffer = [0_u8; 64 * 1024];
            while current_offset < end_offset {
                let remaining = end_offset - current_offset;
                let requested = usize::try_from(remaining)
                    .unwrap_or(usize::MAX)
                    .min(buffer.len());
                let count = self
                    .file
                    .read_at(&mut buffer[..requested], current_offset)
                    .map_err(|error| {
                        ContextPatchError::new(format!(
                            "failed to read `{}`: {error}",
                            self.relative.display()
                        ))
                    })?;
                if count == 0 {
                    return Err(ContextPatchError::new(format!(
                        "`{}` changed while its byte range was being read",
                        self.relative.display()
                    )));
                }
                bytes.extend_from_slice(&buffer[..count]);
                current_offset += count as u64;
            }
            Ok(bytes)
        }

        #[cfg(not(unix))]
        {
            let _ = (offset, max_bytes);
            Err(unsupported_guarded_filesystem())
        }
    }

    pub fn sha256(&self) -> Result<String, ContextPatchError> {
        self.read_range_with_digest(0, 0)
            .map(|inspection| inspection.sha256)
    }

    pub fn read_bounded(&self, max_bytes: u64) -> Result<(Vec<u8>, bool), ContextPatchError> {
        let total_bytes = self.size_bytes()?;
        let bytes = self.read_range(0, max_bytes)?;
        Ok((bytes, total_bytes > max_bytes))
    }

    pub fn revalidate_current_path(&self) -> Result<(), ContextPatchError> {
        #[cfg(unix)]
        {
            ensure_parent_is_current(&self.root_directory, &self.relative, &self.parent)?;
            let current = open_regular_at(&self.parent, &self.leaf, &self.relative)?;
            if FileIdentity::from_file(&self.file)? != FileIdentity::from_file(&current)? {
                return Err(ContextPatchError::new(format!(
                    "target `{}` changed after it was opened",
                    self.relative.display()
                )));
            }
            Ok(())
        }

        #[cfg(not(unix))]
        {
            Err(unsupported_guarded_filesystem())
        }
    }

    pub fn replace_atomic(&self, contents: &[u8]) -> Result<(), ContextPatchError> {
        #[cfg(unix)]
        {
            replace_atomic_unix(self, contents)
        }

        #[cfg(not(unix))]
        {
            let _ = contents;
            Err(unsupported_guarded_filesystem())
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileContentSnapshot {
    device: u64,
    inode: u64,
    size_bytes: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[cfg(unix)]
impl FileContentSnapshot {
    fn capture(file: &File, relative: &Path) -> Result<Self, ContextPatchError> {
        use std::os::unix::fs::MetadataExt;

        let metadata = file.metadata().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to inspect `{}`: {error}",
                relative.display()
            ))
        })?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size_bytes: metadata.size(),
            modified_seconds: metadata.mtime(),
            modified_nanoseconds: metadata.mtime_nsec(),
            changed_seconds: metadata.ctime(),
            changed_nanoseconds: metadata.ctime_nsec(),
        })
    }
}

#[cfg(unix)]
struct Utf8LineCounter {
    valid: bool,
    pending: Vec<u8>,
    newline_count: u64,
    last_byte: Option<u8>,
}

#[cfg(unix)]
impl Utf8LineCounter {
    fn new() -> Self {
        Self {
            valid: true,
            pending: Vec::with_capacity(4),
            newline_count: 0,
            last_byte: None,
        }
    }

    fn update(&mut self, mut bytes: &[u8]) {
        self.newline_count += bytes.iter().filter(|byte| **byte == b'\n').count() as u64;
        if let Some(last) = bytes.last() {
            self.last_byte = Some(*last);
        }
        if !self.valid {
            return;
        }

        while !self.pending.is_empty() && !bytes.is_empty() {
            self.pending.push(bytes[0]);
            bytes = &bytes[1..];
            match std::str::from_utf8(&self.pending) {
                Ok(_) => self.pending.clear(),
                Err(error) if error.error_len().is_none() => continue,
                Err(_) => {
                    self.valid = false;
                    self.pending.clear();
                    return;
                }
            }
        }
        if !self.pending.is_empty() || bytes.is_empty() {
            return;
        }

        if let Err(error) = std::str::from_utf8(bytes) {
            if error.error_len().is_some() {
                self.valid = false;
            } else {
                self.pending
                    .extend_from_slice(&bytes[error.valid_up_to()..]);
            }
        }
    }

    fn finish(&self) -> Option<u64> {
        if !self.valid || !self.pending.is_empty() {
            return None;
        }
        Some(match self.last_byte {
            None => 0,
            Some(b'\n') => self.newline_count,
            Some(_) => self.newline_count + 1,
        })
    }
}

pub fn inspect_path_in_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    path: &Path,
) -> Result<Option<GuardedPathInspection>, ContextPatchError> {
    #[cfg(unix)]
    {
        let authority = repo_root.into();
        let root = reporting_root(authority)?;
        let normalized = normalize_relative_path(path)?;
        inspect_path_unix(authority, root, normalized)
    }

    #[cfg(not(unix))]
    {
        let _ = (repo_root, path);
        Err(unsupported_guarded_filesystem())
    }
}

pub fn open_regular_file_in_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    path: &Path,
) -> Result<GuardedRegularFile, ContextPatchError> {
    #[cfg(unix)]
    {
        let authority = repo_root.into();
        let root = reporting_root(authority)?;
        let normalized = normalize_relative_path(path)?;
        open_regular_file_unix(authority, root, normalized)
    }

    #[cfg(not(unix))]
    {
        let _ = (repo_root, path);
        Err(unsupported_guarded_filesystem())
    }
}

pub fn validate_new_file_path_in_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    path: &Path,
    parents: bool,
) -> Result<(), ContextPatchError> {
    #[cfg(unix)]
    {
        let authority = repo_root.into();
        let root = reporting_root(authority)?;
        let normalized = normalize_relative_path(path)?;
        validate_new_file_path_unix(authority, &root, &normalized, parents)
    }

    #[cfg(not(unix))]
    {
        let _ = (repo_root, path, parents);
        Err(unsupported_guarded_filesystem())
    }
}

pub fn create_new_file_in_root<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    path: &Path,
    contents: &[u8],
    parents: bool,
) -> Result<PathBuf, ContextPatchError> {
    #[cfg(unix)]
    {
        let authority = repo_root.into();
        let root = reporting_root(authority)?;
        let normalized = normalize_relative_path(path)?;
        create_new_file_unix(authority, root, normalized, contents, parents)
    }

    #[cfg(not(unix))]
    {
        let _ = (repo_root, path, contents, parents);
        Err(unsupported_guarded_filesystem())
    }
}

/// The canonical spelling used for reporting and for symlink-target comparison.
///
/// Delegates to the shared label helper so there is one rule for how a root's name is resolved, and so an
/// anchored selection is never re-resolved. In neither case is this value used to *reach* a file: access
/// always goes through the root descriptor.
#[cfg(unix)]
fn reporting_root(root: crate::git::RepositoryRoot<'_>) -> Result<PathBuf, ContextPatchError> {
    let root = crate::fs::rooted::canonical_label(root)?;
    if !root.is_dir() {
        return Err(ContextPatchError::new(format!(
            "repository root {} is not a directory",
            root.display()
        )));
    }
    Ok(root)
}

#[cfg(unix)]
fn normalize_relative_path(path: &Path) -> Result<PathBuf, ContextPatchError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(ContextPatchError::new(
            "path must be a normalized repository-relative path",
        ));
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(ContextPatchError::new(
                "path must be a normalized repository-relative path",
            ));
        };
        normalized.push(component);
    }
    if normalized.as_os_str().is_empty() {
        return Err(ContextPatchError::new(
            "path must be a normalized repository-relative path",
        ));
    }
    Ok(normalized)
}

#[cfg(unix)]
fn inspect_path_unix(
    authority: crate::git::RepositoryRoot<'_>,
    root: PathBuf,
    relative: PathBuf,
) -> Result<Option<GuardedPathInspection>, ContextPatchError> {
    use std::os::unix::fs::MetadataExt;

    let handle = crate::fs::rooted::root_descriptor(authority)?;
    let root_directory = handle.as_file();
    let Some(parent) = open_parent_unix(root_directory, &relative, false, true)? else {
        return Ok(None);
    };
    let leaf = leaf_c_string(&relative)?;
    let Some(stat) = stat_at(&parent, &leaf)? else {
        return Ok(None);
    };
    let file_type = stat.st_mode & libc::S_IFMT;

    if file_type == libc::S_IFREG {
        let file = open_regular_at(&parent, &leaf, &relative)?;
        let metadata = file.metadata().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to inspect `{}`: {error}",
                relative.display()
            ))
        })?;
        return Ok(Some(GuardedPathInspection {
            kind: GuardedPathKind::RegularFile,
            size_bytes: metadata.len(),
            mode: Some(metadata.mode() & 0o7777),
            symlink_target: None,
            symlink_resolves_inside_root: true,
            regular_file: Some(GuardedRegularFile {
                root,
                relative,
                file,
                parent,
                leaf,
                root_directory: retain(root_directory)?,
            }),
        }));
    }

    if file_type == libc::S_IFLNK {
        let target = read_link_at(&parent, &leaf, &relative)?;
        let parent_path = root.join(relative.parent().unwrap_or_else(|| Path::new("")));
        let candidate = if target.is_absolute() {
            target.clone()
        } else {
            parent_path.join(&target)
        };
        let resolves_inside = candidate
            .canonicalize()
            .map(|resolved| resolved.starts_with(&root))
            .unwrap_or(false);
        return Ok(Some(GuardedPathInspection {
            kind: GuardedPathKind::Symlink,
            size_bytes: stat.st_size.max(0) as u64,
            mode: None,
            symlink_target: Some(target),
            symlink_resolves_inside_root: resolves_inside,
            regular_file: None,
        }));
    }

    let kind = if file_type == libc::S_IFDIR {
        GuardedPathKind::Directory
    } else {
        GuardedPathKind::Other
    };
    Ok(Some(GuardedPathInspection {
        kind,
        size_bytes: stat.st_size.max(0) as u64,
        mode: Some((stat.st_mode as u32) & 0o7777),
        symlink_target: None,
        symlink_resolves_inside_root: true,
        regular_file: None,
    }))
}

#[cfg(unix)]
fn open_regular_file_unix(
    authority: crate::git::RepositoryRoot<'_>,
    root: PathBuf,
    relative: PathBuf,
) -> Result<GuardedRegularFile, ContextPatchError> {
    let handle = crate::fs::rooted::root_descriptor(authority)?;
    let root_directory = handle.as_file();
    let parent = open_parent_unix(root_directory, &relative, false, false)?.ok_or_else(|| {
        ContextPatchError::new(format!(
            "`{}` is not an existing regular file",
            relative.display()
        ))
    })?;
    let leaf = leaf_c_string(&relative)?;
    let file = open_regular_at(&parent, &leaf, &relative)?;
    Ok(GuardedRegularFile {
        root,
        relative,
        file,
        parent,
        leaf,
        root_directory: retain(root_directory)?,
    })
}

/// Duplicate a root descriptor so a returned value can outlive the borrowed authority.
#[cfg(unix)]
fn retain(directory: &File) -> Result<File, ContextPatchError> {
    directory.try_clone().map_err(|error| {
        ContextPatchError::new(format!("failed to retain repository root: {error}"))
    })
}

#[cfg(unix)]
fn validate_new_file_path_unix(
    authority: crate::git::RepositoryRoot<'_>,
    root: &Path,
    relative: &Path,
    parents: bool,
) -> Result<(), ContextPatchError> {
    let handle = crate::fs::rooted::root_descriptor(authority)?;
    let Some(parent) = open_parent_unix(handle.as_file(), relative, false, parents)? else {
        return Ok(());
    };
    let leaf = leaf_c_string(relative)?;
    if stat_at(&parent, &leaf)?.is_some() {
        return Err(ContextPatchError::new(format!(
            "target file {} already exists",
            root.join(relative).display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn create_new_file_unix(
    authority: crate::git::RepositoryRoot<'_>,
    root: PathBuf,
    relative: PathBuf,
    contents: &[u8],
    parents: bool,
) -> Result<PathBuf, ContextPatchError> {
    use std::os::fd::AsRawFd;

    let handle = crate::fs::rooted::root_descriptor(authority)?;
    let root_directory = handle.as_file();
    let parent = open_parent_unix(root_directory, &relative, parents, false)?.ok_or_else(|| {
        ContextPatchError::new(format!(
            "failed to resolve parent directory for {}",
            relative.display()
        ))
    })?;
    let leaf = leaf_c_string(&relative)?;
    if stat_at(&parent, &leaf)?.is_some() {
        return Err(ContextPatchError::new(format!(
            "target file {} already exists",
            root.join(&relative).display()
        )));
    }

    let (temp_name, mut temp_file) = create_temp_file_at(&parent)?;
    let result = (|| {
        temp_file.write_all(contents).map_err(|error| {
            ContextPatchError::new(format!(
                "failed to write temporary file for {}: {error}",
                relative.display()
            ))
        })?;
        temp_file.sync_all().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to flush temporary file for {}: {error}",
                relative.display()
            ))
        })?;

        ensure_parent_is_current(root_directory, &relative, &parent)?;
        let status = unsafe {
            libc::linkat(
                parent.as_raw_fd(),
                temp_name.as_ptr(),
                parent.as_raw_fd(),
                leaf.as_ptr(),
                0,
            )
        };
        if status != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                return Err(ContextPatchError::new(format!(
                    "target file {} already exists",
                    root.join(&relative).display()
                )));
            }
            return Err(ContextPatchError::new(format!(
                "failed to publish new file {}: {error}",
                root.join(&relative).display()
            )));
        }

        unlink_at(&parent, &temp_name).map_err(|error| {
            ContextPatchError::new(format!(
                "failed to remove temporary file for {}: {error}",
                relative.display()
            ))
        })
    })();

    if result.is_err() {
        let _ = unlink_at(&parent, &temp_name);
    }
    result?;
    Ok(root.join(relative))
}

#[cfg(unix)]
fn replace_atomic_unix(
    target: &GuardedRegularFile,
    contents: &[u8],
) -> Result<(), ContextPatchError> {
    use std::os::fd::AsRawFd;

    let (temp_name, mut temp_file) = create_temp_file_at(&target.parent)?;
    let result = (|| {
        temp_file.write_all(contents).map_err(|error| {
            ContextPatchError::new(format!(
                "failed to write temporary file for `{}`: {error}",
                target.relative.display()
            ))
        })?;
        temp_file.sync_all().map_err(|error| {
            ContextPatchError::new(format!(
                "failed to flush temporary file for `{}`: {error}",
                target.relative.display()
            ))
        })?;

        // A replacement must not change the target's mode. The temporary file was
        // created with the default creation mode, so carry the target's own
        // permission bits across before the rename makes it the live file.
        apply_permission_bits(&temp_file, preserved_permission_bits(&target.file)?)?;

        target.revalidate_current_path()?;

        let status = unsafe {
            libc::renameat(
                target.parent.as_raw_fd(),
                temp_name.as_ptr(),
                target.parent.as_raw_fd(),
                target.leaf.as_ptr(),
            )
        };
        if status != 0 {
            return Err(ContextPatchError::new(format!(
                "failed to replace `{}`: {}",
                target.relative.display(),
                std::io::Error::last_os_error()
            )));
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = unlink_at(&target.parent, &temp_name);
    }
    result
}

#[cfg(unix)]
fn ensure_parent_is_current(
    root_directory: &File,
    relative: &Path,
    expected_parent: &File,
) -> Result<(), ContextPatchError> {
    let current_parent = open_parent_unix(root_directory, relative, false, false)?.ok_or_else(|| {
        ContextPatchError::new(format!(
            "parent directory for `{}` no longer exists",
            relative.display()
        ))
    })?;
    if FileIdentity::from_file(expected_parent)? != FileIdentity::from_file(&current_parent)? {
        return Err(ContextPatchError::new(format!(
            "parent directory for `{}` changed before mutation",
            relative.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn open_parent_unix(
    root_directory: &File,
    relative: &Path,
    create_missing: bool,
    allow_missing: bool,
) -> Result<Option<File>, ContextPatchError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let mut current = root_directory.try_clone().map_err(|error| {
        ContextPatchError::new(format!("failed to retain repository root: {error}"))
    })?;
    let components = relative.components().collect::<Vec<_>>();
    let mut prefix = PathBuf::new();
    for component in &components[..components.len() - 1] {
        let Component::Normal(component) = component else {
            return Err(ContextPatchError::new(
                "path must be a normalized repository-relative path",
            ));
        };
        prefix.push(component);
        let name = std::ffi::CString::new(component.as_bytes())
            .map_err(|_| ContextPatchError::new("path must not contain an embedded NUL byte"))?;

        let mut descriptor = unsafe {
            libc::openat(
                current.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
            )
        };
        if descriptor < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound && create_missing {
                let status = unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o777) };
                if status != 0 {
                    let mkdir_error = std::io::Error::last_os_error();
                    if mkdir_error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(ContextPatchError::new(format!(
                            "failed to create directory component `{}`: {mkdir_error}",
                            prefix.display()
                        )));
                    }
                }
                descriptor = unsafe {
                    libc::openat(
                        current.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW,
                    )
                };
            } else if error.kind() == std::io::ErrorKind::NotFound && allow_missing {
                return Ok(None);
            } else if error.kind() == std::io::ErrorKind::NotFound {
                return Err(ContextPatchError::new(format!(
                    "failed to resolve parent directory `{}`: missing directory",
                    prefix.display()
                )));
            } else {
                return Err(path_component_error(&prefix, error));
            }
        }
        if descriptor < 0 {
            return Err(path_component_error(
                &prefix,
                std::io::Error::last_os_error(),
            ));
        }
        current = unsafe { File::from_raw_fd(descriptor) };
    }
    Ok(Some(current))
}

#[cfg(unix)]
fn path_component_error(path: &Path, error: std::io::Error) -> ContextPatchError {
    if matches!(
        error.raw_os_error(),
        Some(libc::ELOOP) | Some(libc::ENOTDIR)
    ) {
        ContextPatchError::new(format!(
            "path contains a symlink or non-directory component at `{}`",
            path.display()
        ))
    } else {
        ContextPatchError::new(format!(
            "failed to open directory component `{}` without following symlinks: {error}",
            path.display()
        ))
    }
}

#[cfg(unix)]
fn leaf_c_string(relative: &Path) -> Result<std::ffi::CString, ContextPatchError> {
    use std::os::unix::ffi::OsStrExt;

    let leaf = relative
        .file_name()
        .ok_or_else(|| ContextPatchError::new("path has no file name"))?;
    std::ffi::CString::new(leaf.as_bytes())
        .map_err(|_| ContextPatchError::new("path must not contain an embedded NUL byte"))
}

#[cfg(unix)]
fn stat_at(parent: &File, leaf: &std::ffi::CStr) -> Result<Option<libc::stat>, ContextPatchError> {
    use std::os::fd::AsRawFd;

    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let status = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if status == 0 {
        return Ok(Some(unsafe { stat.assume_init() }));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(None)
    } else {
        Err(ContextPatchError::new(format!(
            "failed to inspect path entry without following symlinks: {error}"
        )))
    }
}

#[cfg(unix)]
fn open_regular_at(
    parent: &File,
    leaf: &std::ffi::CStr,
    relative: &Path,
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
            format!("`{}` contains a symlink component", relative.display())
        } else {
            format!(
                "failed to open `{}` without following symlinks: {error}",
                relative.display()
            )
        };
        return Err(ContextPatchError::new(message));
    }
    let file = unsafe { File::from_raw_fd(descriptor) };
    let metadata = file.metadata().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to inspect `{}`: {error}",
            relative.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(ContextPatchError::new(format!(
            "`{}` is not an existing regular file",
            relative.display()
        )));
    }
    Ok(file)
}

#[cfg(unix)]
fn read_link_at(
    parent: &File,
    leaf: &std::ffi::CStr,
    relative: &Path,
) -> Result<PathBuf, ContextPatchError> {
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStringExt;

    let mut buffer = vec![0_u8; 256];
    loop {
        let count = unsafe {
            libc::readlinkat(
                parent.as_raw_fd(),
                leaf.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if count < 0 {
            return Err(ContextPatchError::new(format!(
                "failed to read symlink `{}`: {}",
                relative.display(),
                std::io::Error::last_os_error()
            )));
        }
        let count = count as usize;
        if count < buffer.len() {
            buffer.truncate(count);
            return Ok(PathBuf::from(OsString::from_vec(buffer)));
        }
        if buffer.len() >= 64 * 1024 {
            return Err(ContextPatchError::new(format!(
                "symlink target for `{}` exceeds 65536 bytes",
                relative.display()
            )));
        }
        buffer.resize(buffer.len() * 2, 0);
    }
}

#[cfg(unix)]
fn create_temp_file_at(parent: &File) -> Result<(std::ffi::CString, File), ContextPatchError> {
    use std::os::fd::{AsRawFd, FromRawFd};

    for attempt in 0..TEMP_FILE_ATTEMPT_LIMIT {
        let name = std::ffi::CString::new(format!(
            ".contextpatch.{}.{}.tmp",
            std::process::id(),
            attempt
        ))
        .expect("generated temporary file name has no NUL");
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                TEMP_FILE_CREATION_MODE,
            )
        };
        if descriptor >= 0 {
            return Ok((name, unsafe { File::from_raw_fd(descriptor) }));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(ContextPatchError::new(format!(
                "failed to create guarded temporary file: {error}"
            )));
        }
    }
    Err(ContextPatchError::new(format!(
        "failed to create a unique guarded temporary file after {TEMP_FILE_ATTEMPT_LIMIT} attempts"
    )))
}

/// Read the permission bits currently set on an open target descriptor.
#[cfg(unix)]
fn preserved_permission_bits(file: &File) -> Result<u32, ContextPatchError> {
    use std::os::unix::fs::MetadataExt;

    file.metadata()
        .map(|metadata| metadata.mode() & PRESERVED_PERMISSION_BITS)
        .map_err(|error| {
            ContextPatchError::new(format!("failed to read the current file mode: {error}"))
        })
}

/// Apply exact permission bits to an open descriptor without naming a path.
#[cfg(unix)]
fn apply_permission_bits(file: &File, bits: u32) -> Result<(), ContextPatchError> {
    use std::os::fd::AsRawFd;

    let status = unsafe { libc::fchmod(file.as_raw_fd(), bits as libc::mode_t) };
    if status != 0 {
        return Err(ContextPatchError::new(format!(
            "failed to preserve file mode {bits:04o}: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn unlink_at(parent: &File, name: &std::ffi::CStr) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let status = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn unsupported_guarded_filesystem() -> ContextPatchError {
    ContextPatchError::new(
        "guarded repository filesystem access requires Unix descriptor-relative path support",
    )
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn refuses_intermediate_symlinks_for_reads_and_creates() {
        let root = temp_root("intermediate-symlink");
        let outside = temp_root("intermediate-symlink-outside");
        fs::write(outside.join("secret.txt"), "outside").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();

        assert!(inspect_path_in_root(&root, Path::new("linked/secret.txt")).is_err());
        assert!(
            create_new_file_in_root(&root, Path::new("linked/new/file.txt"), b"new", true).is_err()
        );
        assert!(!outside.join("new").exists());
    }

    #[cfg(unix)]
    #[test]
    fn creates_and_replaces_through_open_directory_handles() {
        let root = temp_root("create-replace");
        let created =
            create_new_file_in_root(&root, Path::new("nested/sample.txt"), b"before", true)
                .unwrap();
        assert_eq!(fs::read(&created).unwrap(), b"before");

        let opened = open_regular_file_in_root(&root, Path::new("nested/sample.txt")).unwrap();
        assert_eq!(opened.read_all().unwrap(), b"before");
        assert_eq!(
            opened.sha256().unwrap(),
            crate::fs::hash::sha256_bytes(b"before")
        );
        opened.replace_atomic(b"after").unwrap();
        assert_eq!(fs::read(&created).unwrap(), b"after");
    }

    #[test]
    fn reads_only_the_requested_range_from_a_large_sparse_file() {
        use std::os::unix::fs::FileExt;

        let root = temp_root("large-sparse-range");
        let target = root.join("large.bin");
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&target)
            .unwrap();
        let size = 8_u64 * 1024 * 1024 * 1024;
        file.set_len(size).unwrap();
        file.write_at(b"tail", size - 4).unwrap();

        let opened = open_regular_file_in_root(&root, Path::new("large.bin")).unwrap();
        assert_eq!(opened.size_bytes().unwrap(), size);
        assert_eq!(opened.read_range(size - 4, 4).unwrap(), b"tail");

        drop(opened);
        drop(file);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn streams_digest_range_and_utf8_line_count_from_one_file_snapshot() {
        let root = temp_root("streamed-file-snapshot");
        let mut contents = vec![b'a'; 64 * 1024 - 1];
        contents.extend_from_slice("🧪\nlast\n".as_bytes());
        fs::write(root.join("sample.txt"), &contents).unwrap();

        let opened = open_regular_file_in_root(&root, Path::new("sample.txt")).unwrap();
        let read = opened.read_range_with_digest(64 * 1024 - 1, 4).unwrap();

        assert_eq!(read.total_bytes, contents.len() as u64);
        assert_eq!(read.bytes, "🧪".as_bytes());
        assert_eq!(read.sha256, crate::fs::hash::sha256_bytes(&contents));
        assert_eq!(read.line_count, Some(2));
    }

    #[test]
    fn streamed_file_snapshot_reports_no_line_count_for_non_utf8_bytes() {
        let root = temp_root("streamed-binary-snapshot");
        fs::write(root.join("sample.bin"), [b'a', b'\n', 0xff, b'\n']).unwrap();

        let opened = open_regular_file_in_root(&root, Path::new("sample.bin")).unwrap();
        let read = opened.read_range_with_digest(0, 2).unwrap();

        assert_eq!(read.bytes, [b'a', b'\n']);
        assert_eq!(read.line_count, None);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_replacement_after_the_open_parent_is_moved() {
        let root = temp_root("moved-parent");
        let outside = temp_root("moved-parent-outside");
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/sample.txt"), "before").unwrap();

        let opened = open_regular_file_in_root(&root, Path::new("nested/sample.txt")).unwrap();
        fs::rename(root.join("nested"), outside.join("moved")).unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/sample.txt"), "replacement").unwrap();

        let error = opened.replace_atomic(b"after").unwrap_err();
        assert!(error.to_string().contains("parent directory"));
        assert_eq!(
            fs::read(outside.join("moved/sample.txt")).unwrap(),
            b"before"
        );
        assert_eq!(
            fs::read(root.join("nested/sample.txt")).unwrap(),
            b"replacement"
        );
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
