//! Authority to act on a repository.
//!
//! Every operation on a repository needs two things: somewhere to say in a message, and something that
//! establishes *which* directory is meant. Those are not the same, and conflating them is what the whole
//! anchoring effort is undoing. A path says where to look and can be replaced between the looking and the
//! acting. A retained directory descriptor cannot.
//!
//! This type carries both and keeps the second one private. That opacity is the design, not an accident:
//! an `Option<&File>` in the open would let a caller reach past the authority and use the path whenever
//! the descriptor was inconvenient, which is precisely how anchoring gets quietly lost. Callers ask the
//! root to act on their behalf, or they ask for the logical path and accept that they are using a name.
//!
//! [`GitRepository`] is the Git projection of a root: the same authority, narrowed to what running Git
//! needs. Other projections will follow for the filesystem boundaries that are still path-backed.

use std::path::Path;

use crate::error::ContextPatchError;
use crate::git::repository::GitRepository;

/// What establishes which directory a root refers to.
enum RootAuthority<'a> {
    /// The name is the authority. Resolution happens by path, every time.
    Path,
    /// A retained directory descriptor is the authority, and the path is only a label.
    #[cfg(unix)]
    Descriptor(&'a std::fs::File),
}

/// A repository root together with the authority that identifies it.
#[derive(Clone, Copy)]
pub struct RepositoryRoot<'a> {
    logical_path: &'a Path,
    authority: RootAuthority<'a>,
}

// Derived manually because the descriptor is deliberately not printable state: a debug line should say
// which repository and how it is anchored, not leak a file descriptor number.
impl std::fmt::Debug for RepositoryRoot<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RepositoryRoot")
            .field("logical_path", &self.logical_path)
            .field("anchored", &self.is_anchored())
            .finish()
    }
}

impl<'a> Clone for RootAuthority<'a> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a> Copy for RootAuthority<'a> {}

impl<'a> RepositoryRoot<'a> {
    /// A root identified only by name.
    ///
    /// The compatibility form, used by every caller that has not been given a descriptor. Keeping it means
    /// the migration can proceed one boundary at a time instead of all at once.
    pub fn from_path(logical_path: &'a Path) -> Self {
        Self {
            logical_path,
            authority: RootAuthority::Path,
        }
    }

    /// A root anchored to a directory descriptor that was already validated.
    #[cfg(unix)]
    pub fn anchored(logical_path: &'a Path, directory: &'a std::fs::File) -> Self {
        Self {
            logical_path,
            authority: RootAuthority::Descriptor(directory),
        }
    }

    /// The name to use in messages, in receipts, and at boundaries that still resolve by path.
    ///
    /// Named `logical` rather than `path` on purpose. A caller reading this is choosing to use a name, and
    /// the name should make that choice visible at the call site.
    pub fn logical_path(&self) -> &'a Path {
        self.logical_path
    }

    /// Whether this root carries descriptor authority.
    ///
    /// Exposed so callers can skip work they only needed to do for a name, and so tests can assert that a
    /// descriptor actually arrived rather than silently degrading to the path form.
    pub fn is_anchored(&self) -> bool {
        match self.authority {
            RootAuthority::Path => false,
            #[cfg(unix)]
            RootAuthority::Descriptor(_) => true,
        }
    }

    /// The directory descriptor, for the projections that act relative to it.
    ///
    /// Crate-visible rather than public: descriptor-relative work belongs to projections inside `core`,
    /// not to callers outside it.
    #[cfg(unix)]
    pub(crate) fn directory(&self) -> Option<&'a std::fs::File> {
        match self.authority {
            RootAuthority::Path => None,
            RootAuthority::Descriptor(directory) => Some(directory),
        }
    }

    /// The Git projection of this root.
    pub fn git(&self) -> GitRepository<'a> {
        #[cfg(unix)]
        if let Some(directory) = self.directory() {
            return GitRepository::anchored(self.logical_path, directory);
        }
        GitRepository::from_path(self.logical_path)
    }
}

impl<'a> From<&'a Path> for RepositoryRoot<'a> {
    fn from(path: &'a Path) -> Self {
        Self::from_path(path)
    }
}

/// A repository authority that owns what identifies it.
///
/// [`RepositoryRoot`] borrows its descriptor, which is right for a call that completes before returning. An
/// asynchronous job cannot use it: the closure outlives the call that scheduled it, so a borrow cannot be
/// held and the only thing left to capture would be the name. Capturing the name is exactly the substitution
/// this phase removes, and it is worse in a worker than anywhere else, because the gap between validating a
/// directory and acting on it is now as long as the job takes to run.
///
/// So this owns the descriptor instead. A worker captures one of these, borrows it back into a
/// `RepositoryRoot` when it needs to act, and never resolves a name at all.
///
/// A path-backed root retains nothing, deliberately. A configured root *is* reached by name, its pathname is
/// the authority rather than a stand-in for one, and pretending otherwise would change configured-root
/// behaviour rather than preserve it.
pub struct OwnedRepositoryRoot {
    logical_path: std::path::PathBuf,
    /// Present only for a root that carried descriptor authority.
    #[cfg(unix)]
    directory: Option<std::fs::File>,
}

impl std::fmt::Debug for OwnedRepositoryRoot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnedRepositoryRoot")
            .field("logical_path", &self.logical_path)
            .field("anchored", &self.is_anchored())
            .finish()
    }
}

impl OwnedRepositoryRoot {
    /// Retain whatever authority a root carries, so it can outlive the call that had it.
    ///
    /// Fails rather than degrading. An anchored root whose descriptor cannot be duplicated has no authority
    /// to hand a worker, and the only remaining option would be its name, which is the substitution this type
    /// exists to prevent. Callers invoke this before scheduling, so the refusal arrives before any job runs.
    pub fn retain(root: RepositoryRoot<'_>) -> Result<Self, ContextPatchError> {
        #[cfg(unix)]
        {
            let directory = if root.is_anchored() {
                // The root's own authority is asked for a duplicate of itself, so no name is resolved.
                Some(crate::fs::rooted::open_directory(root, "")?)
            } else {
                None
            };
            Ok(Self {
                logical_path: root.logical_path().to_path_buf(),
                directory,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                logical_path: root.logical_path().to_path_buf(),
            })
        }
    }

    /// Borrow this authority back for one operation.
    pub fn borrow(&self) -> RepositoryRoot<'_> {
        #[cfg(unix)]
        if let Some(directory) = &self.directory {
            return RepositoryRoot::anchored(&self.logical_path, directory);
        }
        RepositoryRoot::from_path(&self.logical_path)
    }

    /// Whether this authority is descriptor-backed.
    pub fn is_anchored(&self) -> bool {
        #[cfg(unix)]
        {
            self.directory.is_some()
        }
        #[cfg(not(unix))]
        false
    }

    /// The name to use in messages and receipts, never to reach the repository.
    pub fn logical_path(&self) -> &Path {
        &self.logical_path
    }

    /// Duplicate this authority, or fail.
    ///
    /// Deliberately not `Clone`. `Clone` cannot fail, so implementing it would force a choice between
    /// panicking and quietly returning a path-backed copy of an anchored root. The second is worse than it
    /// looks: the copy would still answer `logical_path`, still be accepted everywhere a root is accepted,
    /// and differ only in that a name now decides which directory is reached. That is exactly the failure
    /// this phase removes, so it is not offered.
    pub fn try_clone(&self) -> Result<Self, ContextPatchError> {
        #[cfg(unix)]
        {
            let directory = match &self.directory {
                Some(directory) => Some(directory.try_clone().map_err(|error| {
                    ContextPatchError::new(format!(
                        "failed to duplicate the retained authority for repository {}: {error}",
                        self.logical_path.display()
                    ))
                })?),
                None => None,
            };
            Ok(Self {
                logical_path: self.logical_path.clone(),
                directory,
            })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                logical_path: self.logical_path.clone(),
            })
        }
    }
}

impl<'a> From<&'a std::path::PathBuf> for RepositoryRoot<'a> {
    fn from(path: &'a std::path::PathBuf) -> Self {
        Self::from_path(path.as_path())
    }
}

impl<'a> From<RepositoryRoot<'a>> for GitRepository<'a> {
    fn from(root: RepositoryRoot<'a>) -> Self {
        root.git()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_backed_root_has_no_descriptor_authority() {
        let root = RepositoryRoot::from_path(Path::new("/repository"));

        assert!(!root.is_anchored());
        assert_eq!(root.logical_path(), Path::new("/repository"));
        assert!(!root.git().is_anchored());
    }

    #[cfg(unix)]
    #[test]
    fn the_git_projection_inherits_descriptor_authority() {
        let directory = std::fs::File::open(std::env::temp_dir()).unwrap();
        let path = Path::new("/repository");
        let root = RepositoryRoot::anchored(path, &directory);

        assert!(root.is_anchored());
        // The projection carries the authority through, and still reports the logical path for messages.
        assert!(root.git().is_anchored());
        assert_eq!(root.git().path(), path);
        assert_eq!(root.logical_path(), path);
    }

    #[cfg(unix)]
    #[test]
    fn the_debug_form_reports_anchoring_without_leaking_the_descriptor() {
        let directory = std::fs::File::open(std::env::temp_dir()).unwrap();
        let root = RepositoryRoot::anchored(Path::new("/repository"), &directory);

        let rendered = format!("{root:?}");

        assert!(rendered.contains("anchored: true"), "{rendered}");
        assert!(rendered.contains("/repository"), "{rendered}");
        // A descriptor number is not state a message should carry.
        assert!(!rendered.contains("File"), "{rendered}");
    }
}
