//! The repository a Git operation targets.
//!
//! Policy functions take this rather than a bare path, and rather than the runner's working-directory
//! type. A path says *where to look*; this says *which repository*, and it can carry the validated
//! directory descriptor that proves the answer has not changed since it was checked. Keeping the runner's
//! type out of policy signatures matters because policy should not be expressing how a child process
//! establishes its working directory.
//!
//! Copyable on purpose. A plan is often threaded through several steps that each need the repository, and
//! a context that had to be borrowed or cloned would push lifetime noise into every one of them.

use std::path::Path;

use crate::process::runner::CommandCwd;

/// A repository target, optionally anchored to an open directory descriptor.
#[derive(Clone, Copy, Debug)]
pub struct GitRepository<'a> {
    path: &'a Path,
    #[cfg(unix)]
    directory: Option<&'a std::fs::File>,
}

impl<'a> GitRepository<'a> {
    /// A repository identified only by path.
    ///
    /// The compatibility form. Every caller that has not yet been given a descriptor uses this, which is
    /// what lets the migration proceed one boundary at a time instead of all at once.
    pub fn from_path(path: &'a Path) -> Self {
        Self {
            path,
            #[cfg(unix)]
            directory: None,
        }
    }

    /// A repository anchored to a directory descriptor that was already validated.
    #[cfg(unix)]
    pub fn anchored(path: &'a Path, directory: &'a std::fs::File) -> Self {
        Self {
            path,
            directory: Some(directory),
        }
    }

    /// The repository root path, for messages, receipts, and path-relative work.
    pub fn path(&self) -> &'a Path {
        self.path
    }

    /// Whether this target carries a descriptor.
    ///
    /// Exposed so tests can assert that a descriptor actually reached the operation under test, rather
    /// than silently falling back to the path form.
    pub fn is_anchored(&self) -> bool {
        #[cfg(unix)]
        {
            self.directory.is_some()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// The working directory a guarded command should run in.
    pub fn command_cwd(&self) -> CommandCwd<'a> {
        #[cfg(unix)]
        if let Some(directory) = self.directory {
            return CommandCwd::Anchored {
                directory,
                logical_path: self.path,
            };
        }
        CommandCwd::Path(self.path)
    }
}

impl<'a> From<&'a Path> for GitRepository<'a> {
    fn from(path: &'a Path) -> Self {
        Self::from_path(path)
    }
}

impl<'a> From<&'a std::path::PathBuf> for GitRepository<'a> {
    fn from(path: &'a std::path::PathBuf) -> Self {
        Self::from_path(path.as_path())
    }
}

impl<'a> From<GitRepository<'a>> for CommandCwd<'a> {
    fn from(repository: GitRepository<'a>) -> Self {
        repository.command_cwd()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_backed_target_carries_no_descriptor() {
        let repository = GitRepository::from_path(Path::new("/repository"));

        assert!(!repository.is_anchored());
        assert_eq!(repository.path(), Path::new("/repository"));
        assert!(matches!(repository.command_cwd(), CommandCwd::Path(_)));
    }

    #[cfg(unix)]
    #[test]
    fn an_anchored_target_produces_an_anchored_working_directory() {
        let directory = std::fs::File::open(std::env::temp_dir()).unwrap();
        let path = Path::new("/repository");
        let repository = GitRepository::anchored(path, &directory);

        assert!(repository.is_anchored());
        // The logical path is still reported, because messages and receipts need it.
        assert_eq!(repository.path(), path);
        match repository.command_cwd() {
            CommandCwd::Anchored { logical_path, .. } => assert_eq!(logical_path, path),
            other => panic!("expected an anchored working directory, got {other:?}"),
        }
    }
}
