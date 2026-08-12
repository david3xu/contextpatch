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

use crate::tools::project;

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
    if let Some(entry) = crate::tools::registry::descriptor(name) {
        return entry.reach;
    }

    if name == project::project_execute::NAME {
        return RemoteReach::WrapperDispatch;
    }

    // Nothing else is registered, so an unknown name cannot widen an advertised claim.
    RemoteReach::Local
}

/// Whether an action only observes state and never mutates the repository or host.
pub fn is_read_only(name: &str) -> bool {
    crate::tools::registry::descriptor(name).is_some_and(|entry| entry.read_only)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_execution_is_not_open_world() {
        assert!(!RemoteReach::IsolatedExecution.is_open_world());
        assert_eq!(
            remote_reach(crate::tools::process::task_image_python_run::NAME),
            RemoteReach::IsolatedExecution
        );
    }

    #[test]
    fn direct_remote_and_executed_code_are_open_world() {
        assert_eq!(
            remote_reach(crate::tools::git::names::git_push_exact::NAME),
            RemoteReach::DirectRemote
        );
        assert_eq!(
            remote_reach(crate::tools::process::run_guarded_command::NAME),
            RemoteReach::InheritedByExecutedCode
        );
        assert!(remote_reach(crate::tools::git::names::git_push_exact::NAME).is_open_world());
        assert!(remote_reach(crate::tools::process::run_guarded_command::NAME).is_open_world());
    }

    #[test]
    fn wrapper_inherits_widest_reach() {
        assert_eq!(
            remote_reach(project::project_execute::NAME),
            RemoteReach::WrapperDispatch
        );
        assert!(remote_reach(project::project_execute::NAME).is_open_world());
    }

    /// The two Docker paths are deliberately classified differently: the task image pins
    /// `--network none`, the compose stack does not.
    #[test]
    fn the_networked_docker_path_is_open_world_unlike_the_isolated_one() {
        assert_eq!(
            remote_reach(crate::tools::process::compose_stack_run::NAME),
            RemoteReach::InheritedByExecutedCode
        );
        assert!(remote_reach(crate::tools::process::compose_stack_run::NAME).is_open_world());
        assert!(!remote_reach(crate::tools::process::task_image_python_run::NAME).is_open_world());
    }

    #[test]
    fn local_writes_stay_closed_world() {
        assert_eq!(
            remote_reach(crate::tools::files::write_new_file::NAME),
            RemoteReach::Local
        );
        assert!(!remote_reach(crate::tools::files::write_new_file::NAME).is_open_world());
    }

    #[test]
    fn unknown_names_do_not_widen_claims() {
        assert_eq!(remote_reach(""), RemoteReach::Local);
    }
}
