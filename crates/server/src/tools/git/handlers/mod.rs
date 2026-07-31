pub mod commit;
pub mod restore;
pub mod sync;
pub mod tracked;

pub(crate) use commit::{
    call_git_commit_exact, call_git_commit_prefix, call_git_commit_scoped, call_git_stage_exact,
    call_git_staged_scope_check,
};
pub(crate) use restore::{
    call_delete_generated_prefix, call_delete_untracked_exact, call_git_restore_exact,
};
pub(crate) use sync::{
    call_git_branch_prepare, call_git_merge_readiness, call_git_push_exact, call_git_remote_check,
    call_git_remote_list,
};
pub(crate) use tracked::{call_delete_guarded, call_move_tracked};
