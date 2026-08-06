//! Centralized advertised-authority classification for MCP tool annotations.
//!
//! `openWorldHint` and `readOnlyHint` are public capability claims, so they are derived here from
//! one documented classification instead of being repeated as bare booleans at each schema site.
//!
//! The classification answers a narrow question: can this action reach a system outside the host,
//! either by contacting one itself or by starting a child that inherits the server's network
//! capability? It is not a containment claim. Executable allowlisting narrows entry points and
//! arguments; it does not sandbox the resulting process. Only the task-image path carries the
//! documented container isolation. See `docs/execution-threat-model.md`.

use crate::tools::git::names as git_names;
use crate::tools::{capability, files, fixtures, github, native, process, project, setup};

/// How far an advertised action can reach beyond the local host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteReach {
    /// Stays on the local host: no remote contact, and no child that inherits network capability.
    Local,
    /// Contacts a remote system itself, such as a Git remote or the GitHub API.
    DirectRemote,
    /// Executes repository-controlled code that inherits the server environment, and with it the
    /// server user's network capability. Dependency resolution alone reaches the network.
    InheritedByExecutedCode,
    /// Executes code under the documented container isolation with networking disabled.
    IsolatedExecution,
    /// The project wrapper dispatches every inner action, so it inherits the widest reach of any.
    WrapperDispatch,
}

impl RemoteReach {
    /// Whether this reach is advertised as open-world in `tools/list` annotations.
    pub const fn is_open_world(self) -> bool {
        match self {
            Self::Local | Self::IsolatedExecution => false,
            Self::DirectRemote | Self::InheritedByExecutedCode | Self::WrapperDispatch => true,
        }
    }
}

/// Classify one advertised action name.
///
/// Unknown names classify as `Local` so a missing entry cannot silently widen an advertised claim;
/// the schema tests pin the classification for every documented action.
pub fn remote_reach(name: &str) -> RemoteReach {
    if name == project::project_execute::NAME {
        return RemoteReach::WrapperDispatch;
    }

    if is_direct_remote(name) {
        return RemoteReach::DirectRemote;
    }

    if is_isolated_execution(name) {
        return RemoteReach::IsolatedExecution;
    }

    if executes_repository_code(name) {
        return RemoteReach::InheritedByExecutedCode;
    }

    RemoteReach::Local
}

/// Actions that contact a remote system directly.
fn is_direct_remote(name: &str) -> bool {
    name == git_names::git_remote_check::NAME
        || name == git_names::git_push_exact::NAME
        || name == git_names::git_branch_prepare::NAME
        || name == git_names::git_merge_readiness::NAME
        || name == github::github_fork_prepare::NAME
        || name == github::github_pr_run::NAME
}

/// Actions whose execution happens under the documented container isolation with networking off.
fn is_isolated_execution(name: &str) -> bool {
    name == process::task_image_python_run::NAME
        || name == process::image_cleanliness_check_run::NAME
}

/// Actions that start repository-controlled code inheriting the server environment.
fn executes_repository_code(name: &str) -> bool {
    name == process::run_guarded_command::NAME
        || name == process::artifact_python_run::NAME
        || name == process::validation_profile_run::NAME
        || name == process::harbor_run_start::NAME
        || name == setup::setup_profile_run::NAME
        || name == native::native_build_run::NAME
        || name == native::native_device_run::NAME
        || name == fixtures::fixture_generator_run::NAME
        || name == fixtures::base_image_check_run::NAME
}

/// Whether an action only observes state and never mutates the repository or host.
pub fn is_read_only(name: &str) -> bool {
    name == capability::capability_manifest::NAME
        || name == capability::preflight_health::NAME
        || name == files::read_range::NAME
        || name == files::read_write_receipts::NAME
        || name == files::diff_preview::NAME
        || name == files::status_guard::NAME
        || name == files::file_info::NAME
        || name == files::list_directory::NAME
        || name == files::read_file_bytes::NAME
        || name == process::read_command_log::NAME
        || name == fixtures::fixture_manifest_verify::NAME
        || name == git_names::git_remote_list::NAME
        || name == git_names::git_merge_readiness::NAME
        || name == git_names::git_staged_scope_check::NAME
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_execution_is_not_open_world() {
        assert!(!RemoteReach::IsolatedExecution.is_open_world());
        assert_eq!(
            remote_reach(process::task_image_python_run::NAME),
            RemoteReach::IsolatedExecution
        );
    }

    #[test]
    fn direct_remote_and_executed_code_are_open_world() {
        assert_eq!(
            remote_reach(git_names::git_push_exact::NAME),
            RemoteReach::DirectRemote
        );
        assert_eq!(
            remote_reach(process::run_guarded_command::NAME),
            RemoteReach::InheritedByExecutedCode
        );
        assert!(remote_reach(git_names::git_push_exact::NAME).is_open_world());
        assert!(remote_reach(process::run_guarded_command::NAME).is_open_world());
    }

    #[test]
    fn wrapper_inherits_widest_reach() {
        assert_eq!(
            remote_reach(project::project_execute::NAME),
            RemoteReach::WrapperDispatch
        );
        assert!(remote_reach(project::project_execute::NAME).is_open_world());
    }

    #[test]
    fn local_writes_stay_closed_world() {
        assert_eq!(remote_reach(files::write_new_file::NAME), RemoteReach::Local);
        assert!(!remote_reach(files::write_new_file::NAME).is_open_world());
    }

    #[test]
    fn unknown_names_do_not_widen_claims() {
        assert_eq!(remote_reach(""), RemoteReach::Local);
    }
}
