pub mod commit;
pub mod delete_guarded;
mod guarded_path;
pub mod repository;
pub mod restore;
pub mod root;
pub mod state;
pub mod status;
pub mod sync;
pub mod tracked_move;
pub mod validate;
mod workspace;

pub use workspace::{select_workspace_repository, SelectedRepository};

/// Exposed so the MCP layer can enforce a per-action worktree-root policy without duplicating the
/// check. The rest of `guarded_path` stays private on purpose; only the two resolvers are public, and
/// they differ solely in what they demand beyond resolution.
pub use guarded_path::{exact_worktree_root, exact_worktree_root_in, resolve_repo_root};
pub use repository::GitRepository;
pub use root::{OwnedRepositoryRoot, RepositoryRoot};
