pub mod delete_guarded;
mod guarded_path;
pub mod status;
pub mod tracked_move;
pub mod validate;
mod workspace;

pub use workspace::resolve_workspace_git_root;

/// Exposed so the MCP layer can enforce a per-action worktree-root policy without duplicating the
/// check. The rest of `guarded_path` stays private on purpose; only the strict resolution is public.
pub use guarded_path::exact_worktree_root;
