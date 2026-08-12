//! Typed Docker Compose stack proofs.
//!
//! Compose is not reachable through `run_guarded_command` and must not become so: safety-contract
//! clauses 247 and 253 require every Docker path to be a narrow typed gate rather than general
//! Docker authority. This module is that gate for the stack proofs. The caller selects a named
//! action; the argv is derived here and never supplied, which is what lets the command run through
//! `run_bounded_command` without passing the guarded-command allowlist.
//!
//! Two properties do the safety work.
//!
//! Each action is pinned to one compose file, so the set of stacks this server can start is fixed by
//! review rather than by whatever compose files exist in the tree. A missing file refuses before
//! anything runs, with the path named, so a wrong pin fails loudly rather than starting the wrong
//! stack.
//!
//! Every action runs under its own Compose project name. That is what makes teardown safe: `down`
//! is scoped to the project this module created, so it cannot stop or delete a stack the developer
//! is running by hand, even one defined by the same compose file. Without the project scope a
//! teardown would be indistinguishable from `docker compose down` in the operator's own session.
//!
//! Unlike `task_image`, this path runs with networking enabled, because a stack proof exists to
//! exercise service-to-service traffic. That makes it genuinely open-world; see
//! `docs/execution-threat-model.md`. Containers started here run repository-defined images with the
//! server user's permissions and network access.

use std::path::PathBuf;
use std::time::Duration;

use crate::error::ContextPatchError;
use crate::process::guarded_command::redact_and_truncate_output;
use crate::process::runner::{run_bounded_command, BoundedProcessOutput};

pub const CONFIRMATION: &str = "run compose stack";

pub const SELECTED_ROOT_REFUSAL: &str =
    "compose stack runs are available only for the configured --repo-root, because a container \
     receives argv paths rather than a directory descriptor and a selected repository's authority \
     cannot be handed to a Docker child";

const DEFAULT_UP_TIMEOUT_SECS: u64 = 1800;
const MAX_UP_TIMEOUT_SECS: u64 = 3600;
const TEARDOWN_TIMEOUT_SECS: u64 = 120;

/// Compose project prefix, so every stack this server starts is identifiable and separable from an
/// operator's own `docker compose` sessions.
const PROJECT_PREFIX: &str = "contextpatch-proof";

/// The stack proofs this tool may run, each pinned to its compose file.
///
/// The compose paths follow the `compose/<action>.yml` convention. If a repository lays its proofs
/// out differently, correct the right-hand column here: it is the single place the mapping lives,
/// and a path that does not exist refuses by name before Docker is invoked.
const STACK_ACTIONS: &[(&str, &str)] = &[
    ("auto-workflow", "compose/auto-workflow.yml"),
    ("dispatch-preflight", "compose/dispatch-preflight.yml"),
    ("front-door", "compose/front-door.yml"),
    ("full-platform", "compose/full-platform.yml"),
    ("human-ai-team-flow", "compose/human-ai-team-flow.yml"),
    ("local-edition", "compose/local-edition.yml"),
];

pub fn action_names() -> Vec<&'static str> {
    STACK_ACTIONS.iter().map(|(action, _)| *action).collect()
}

#[derive(Clone, Debug)]
pub struct ComposeStackPlan {
    repo_root: PathBuf,
    action: String,
    compose_file: String,
    project_name: String,
    up_args: Vec<String>,
    down_args: Vec<String>,
    up_timeout: Duration,
    down_timeout: Duration,
}

impl ComposeStackPlan {
    pub fn action(&self) -> &str {
        &self.action
    }

    pub fn compose_file(&self) -> &str {
        &self.compose_file
    }

    pub fn project_name(&self) -> &str {
        &self.project_name
    }

    pub fn up_args(&self) -> &[String] {
        &self.up_args
    }

    pub fn down_args(&self) -> &[String] {
        &self.down_args
    }

    pub fn up_timeout(&self) -> Duration {
        self.up_timeout
    }

    pub fn down_timeout(&self) -> Duration {
        self.down_timeout
    }
}

#[derive(Clone, Debug)]
pub struct ComposeStackCommandResult {
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout: String,
    pub stdout_truncated: bool,
    pub stderr: String,
    pub stderr_truncated: bool,
}

impl ComposeStackCommandResult {
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit_code == 0
    }
}

#[derive(Clone, Debug)]
pub struct ComposeStackRunResult {
    pub up: ComposeStackCommandResult,
    /// Always attempted, including after a failed or timed-out `up`, so a proof cannot leave a
    /// stack running. `None` only when the teardown could not be spawned at all.
    pub teardown: Option<ComposeStackCommandResult>,
}

impl ComposeStackRunResult {
    /// The proof passed. Teardown failure does not make a passing proof fail, but it is reported
    /// separately so a leaked stack is visible rather than silent.
    pub fn success(&self) -> bool {
        self.up.success()
    }

    pub fn teardown_clean(&self) -> bool {
        self.teardown
            .as_ref()
            .is_some_and(ComposeStackCommandResult::success)
    }
}

/// Refuse a selected repository before any argument is read.
///
/// Mirrors `task_image`: a Docker child is handed argv paths, so a selection's descriptor authority
/// cannot follow it, and planning against a selection must fail exactly where executing would.
pub fn ensure_compose_root_is_addressable(
    repo_root: crate::git::RepositoryRoot<'_>,
) -> Result<(), ContextPatchError> {
    if repo_root.is_anchored() {
        return Err(ContextPatchError::new(SELECTED_ROOT_REFUSAL));
    }
    Ok(())
}

pub fn plan_compose_stack_run<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    action: &str,
    timeout_secs: Option<u64>,
) -> Result<ComposeStackPlan, ContextPatchError> {
    let root = repo_root.into();
    ensure_compose_root_is_addressable(root)?;

    let Some((action, compose_file)) = STACK_ACTIONS
        .iter()
        .find(|(name, _)| *name == action)
        .copied()
    else {
        return Err(ContextPatchError::invalid(format!(
            "unknown compose stack action `{action}`; expected one of {}",
            action_names().join(", ")
        )));
    };

    // A pinned path that does not exist is a mis-pin, not a Docker failure. Refusing here names the
    // file, so the mapping above is the obvious thing to correct.
    if !crate::fs::rooted::is_regular_file(root, compose_file)? {
        return Err(ContextPatchError::invalid(format!(
            "compose file `{compose_file}` for action `{action}` is not a regular file in the \
             repository; correct the action-to-file mapping if this repository lays its compose \
             files out differently"
        )));
    }

    let up_timeout = checked_timeout(timeout_secs, DEFAULT_UP_TIMEOUT_SECS, MAX_UP_TIMEOUT_SECS)?;
    let project_name = format!("{PROJECT_PREFIX}-{action}");
    let logical_root = root.logical_path().to_path_buf();

    // `--abort-on-container-exit` keeps a proof from hanging once the workload container finishes,
    // and pairs with the teardown below so the stack does not outlive the call.
    let up_args = vec![
        "compose".to_string(),
        "--file".to_string(),
        compose_file.to_string(),
        "--project-name".to_string(),
        project_name.clone(),
        "up".to_string(),
        "--build".to_string(),
        "--abort-on-container-exit".to_string(),
    ];
    // `--volumes` and `--remove-orphans` are scoped to this project name, so they cannot reach an
    // operator's own stack even when it is defined by the same compose file.
    let down_args = vec![
        "compose".to_string(),
        "--file".to_string(),
        compose_file.to_string(),
        "--project-name".to_string(),
        project_name.clone(),
        "down".to_string(),
        "--volumes".to_string(),
        "--remove-orphans".to_string(),
    ];

    Ok(ComposeStackPlan {
        repo_root: logical_root,
        action: action.to_string(),
        compose_file: compose_file.to_string(),
        project_name,
        up_args,
        down_args,
        up_timeout,
        down_timeout: Duration::from_secs(TEARDOWN_TIMEOUT_SECS),
    })
}

pub fn run_compose_stack(
    plan: &ComposeStackPlan,
    confirm: Option<&str>,
) -> Result<ComposeStackRunResult, ContextPatchError> {
    if confirm != Some(CONFIRMATION) {
        return Err(ContextPatchError::new(format!(
            "execution requires confirm: {CONFIRMATION:?}"
        )));
    }

    let up = match run_bounded_command(
        &plan.repo_root,
        "docker",
        &plan.up_args,
        plan.up_timeout,
        "compose stack up",
    ) {
        Ok(output) => command_result(output, 16_000, 24_000),
        Err(error) => {
            // The stack may still be partly up, so tear down before reporting the failure.
            let teardown = teardown(plan);
            return Ok(ComposeStackRunResult {
                up: command_error_result(error),
                teardown,
            });
        }
    };

    Ok(ComposeStackRunResult {
        teardown: teardown(plan),
        up,
    })
}

/// Always attempted, so a failed or timed-out proof does not leave containers and volumes behind.
fn teardown(plan: &ComposeStackPlan) -> Option<ComposeStackCommandResult> {
    run_bounded_command(
        &plan.repo_root,
        "docker",
        &plan.down_args,
        plan.down_timeout,
        "compose stack down",
    )
    .ok()
    .map(|output| command_result(output, 4_000, 8_000))
}

fn command_result(
    output: BoundedProcessOutput,
    max_stdout: usize,
    max_stderr: usize,
) -> ComposeStackCommandResult {
    let (stdout, stdout_truncated) =
        redact_and_truncate_output(&String::from_utf8_lossy(&output.stdout), max_stdout);
    let (stderr, stderr_truncated) =
        redact_and_truncate_output(&String::from_utf8_lossy(&output.stderr), max_stderr);
    ComposeStackCommandResult {
        exit_code: output.exit_code,
        timed_out: output.timed_out,
        duration_ms: output.duration_ms,
        stdout,
        stdout_truncated: output.stdout_truncated || stdout_truncated,
        stderr,
        stderr_truncated: output.stderr_truncated || stderr_truncated,
    }
}

/// A command that could not be spawned at all still has to be reported as a result rather than an
/// error, so the teardown outcome beside it survives into the log.
fn command_error_result(error: ContextPatchError) -> ComposeStackCommandResult {
    let (stderr, stderr_truncated) = redact_and_truncate_output(&error.to_string(), 8_000);
    ComposeStackCommandResult {
        exit_code: -1,
        timed_out: false,
        duration_ms: 0,
        stdout: String::new(),
        stdout_truncated: false,
        stderr,
        stderr_truncated,
    }
}

fn checked_timeout(
    requested: Option<u64>,
    default_secs: u64,
    max_secs: u64,
) -> Result<Duration, ContextPatchError> {
    let seconds = requested.unwrap_or(default_secs);
    if seconds == 0 || seconds > max_secs {
        return Err(ContextPatchError::invalid(format!(
            "timeout_secs must be between 1 and {max_secs}"
        )));
    }
    Ok(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo_with_compose(name: &str, files: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-compose-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("compose")).unwrap();
        for file in files {
            fs::write(root.join(file), "services: {}\n").unwrap();
        }
        root
    }

    #[test]
    fn plans_a_named_stack_without_running_anything() {
        let root = repo_with_compose("plans", &["compose/full-platform.yml"]);
        let plan = plan_compose_stack_run(root.as_path(), "full-platform", None).unwrap();

        assert_eq!(plan.compose_file(), "compose/full-platform.yml");
        assert_eq!(plan.project_name(), "contextpatch-proof-full-platform");
        assert_eq!(plan.up_timeout(), Duration::from_secs(1800));
        assert_eq!(
            plan.up_args(),
            [
                "compose",
                "--file",
                "compose/full-platform.yml",
                "--project-name",
                "contextpatch-proof-full-platform",
                "up",
                "--build",
                "--abort-on-container-exit"
            ]
        );
    }

    /// The teardown must name the project, or it would be indistinguishable from an operator's own
    /// `docker compose down` against the same file.
    #[test]
    fn teardown_is_scoped_to_the_project_it_created() {
        let root = repo_with_compose("teardown", &["compose/front-door.yml"]);
        let plan = plan_compose_stack_run(root.as_path(), "front-door", None).unwrap();

        let down = plan.down_args();
        let project = down
            .iter()
            .position(|arg| arg == "--project-name")
            .expect("teardown must scope to a project");
        assert_eq!(down[project + 1], "contextpatch-proof-front-door");
        assert!(down.contains(&"down".to_string()));
    }

    #[test]
    fn refuses_an_unknown_action_and_names_the_available_ones() {
        let root = repo_with_compose("unknown", &["compose/full-platform.yml"]);
        let error = plan_compose_stack_run(root.as_path(), "whole-world", None)
            .unwrap_err()
            .to_string();

        assert!(error.contains("unknown compose stack action"), "{error}");
        assert!(error.contains("full-platform"), "{error}");
    }

    /// A wrong pin must fail by name rather than starting some other stack.
    #[test]
    fn refuses_a_pinned_compose_file_that_is_absent() {
        let root = repo_with_compose("absent", &[]);
        let error = plan_compose_stack_run(root.as_path(), "local-edition", None)
            .unwrap_err()
            .to_string();

        assert!(
            error.contains("compose/local-edition.yml"),
            "refusal must name the pinned file: {error}"
        );
    }

    #[test]
    fn refuses_timeouts_outside_the_permitted_range() {
        let root = repo_with_compose("timeout", &["compose/auto-workflow.yml"]);
        for requested in [0, MAX_UP_TIMEOUT_SECS + 1] {
            let error = plan_compose_stack_run(root.as_path(), "auto-workflow", Some(requested))
                .unwrap_err()
                .to_string();
            assert!(error.contains("between 1 and 3600"), "{error}");
        }
    }

    #[test]
    fn refuses_execution_without_the_exact_confirmation() {
        let root = repo_with_compose("confirm", &["compose/auto-workflow.yml"]);
        let plan = plan_compose_stack_run(root.as_path(), "auto-workflow", None).unwrap();

        for confirm in [None, Some("yes"), Some("run compose")] {
            let error = run_compose_stack(&plan, confirm).unwrap_err().to_string();
            assert!(error.contains("run compose stack"), "{error}");
        }
    }
}
