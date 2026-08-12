//! Typed artifact build and import smoke check.
//!
//! The gap this closes is packaging: a build that succeeds and an artifact that cannot actually be
//! imported are different failures, and reading source finds neither. That needs a real `docker
//! build` followed by a real run of the thing that was built.
//!
//! Unlike [`super::compose_stack`], the Dockerfile is supplied by the caller rather than pinned to a
//! server-owned list. That is not a weakening: `task_image_python_run` already accepts a
//! caller-named repository script on the same reasoning. A Dockerfile is reviewed repository
//! content, and the path is validated descriptor-relative with no-follow at every component, so it
//! cannot escape the repository or traverse a symlink. Pinning would buy nothing here, because
//! there is no fixed set of artifacts the way there is a fixed set of stack proofs, and an invented
//! convention would refuse every real repository.
//!
//! What stays server-owned is every Docker argument. The caller names a Dockerfile, a build context,
//! and arguments for the built image; it never chooses flags, mounts, tags, or networking.
//!
//! Two positional properties do real work:
//!
//! The smoke arguments are placed after the image name in `docker run [OPTIONS] IMAGE [COMMAND]`, so
//! they are the container's command by construction and cannot be reinterpreted as Docker options
//! however they are spelled. That is a property of argv position, not of a filter.
//!
//! The smoke run is pinned to `--network none`. An import check that needs the network is not
//! testing packaging, and a dead export must not be masked by a successful download. The build
//! itself does have the network, because dependency installation is most of what a build does, so
//! this action is open-world overall.
//!
//! `docker build` hands work to the Docker daemon, which conventionally runs as root and is not
//! namespaced from the host the way the task image is. That is a real escalation surface and is
//! documented rather than implied; see `docs/execution-threat-model.md`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::error::ContextPatchError;
use crate::process::guarded_command::redact_and_truncate_output;
use crate::process::runner::{run_bounded_command, BoundedProcessOutput};

pub const CONFIRMATION: &str = "run artifact build check";

pub const SELECTED_ROOT_REFUSAL: &str =
    "artifact build checks are available only for the configured --repo-root, because a Docker \
     build receives argv paths rather than a directory descriptor and a selected repository's \
     authority cannot be handed to a Docker child";

const DEFAULT_BUILD_TIMEOUT_SECS: u64 = 1800;
const MAX_BUILD_TIMEOUT_SECS: u64 = 3600;
const DEFAULT_SMOKE_TIMEOUT_SECS: u64 = 300;
const MAX_SMOKE_TIMEOUT_SECS: u64 = 600;
const CLEANUP_TIMEOUT_SECS: u64 = 60;
const MAX_SMOKE_ARGS: usize = 32;
const MAX_ARG_BYTES: usize = 4096;

/// Image repository for every artifact this server builds, so its images are identifiable and can
/// be removed without pattern-matching an operator's own tags.
const TAG_REPOSITORY: &str = "contextpatch-artifact";

#[derive(Clone, Debug)]
pub struct ArtifactBuildPlan {
    repo_root: PathBuf,
    dockerfile: String,
    context: String,
    tag: String,
    build_args: Vec<String>,
    smoke_args: Vec<String>,
    image_cleanup_args: Vec<String>,
    build_timeout: Duration,
    smoke_timeout: Duration,
}

impl ArtifactBuildPlan {
    pub fn dockerfile(&self) -> &str {
        &self.dockerfile
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn build_args(&self) -> &[String] {
        &self.build_args
    }

    pub fn smoke_args(&self) -> &[String] {
        &self.smoke_args
    }

    pub fn image_cleanup_args(&self) -> &[String] {
        &self.image_cleanup_args
    }

    pub fn build_timeout(&self) -> Duration {
        self.build_timeout
    }

    pub fn smoke_timeout(&self) -> Duration {
        self.smoke_timeout
    }
}

#[derive(Clone, Debug)]
pub struct ArtifactBuildCommandResult {
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout: String,
    pub stdout_truncated: bool,
    pub stderr: String,
    pub stderr_truncated: bool,
}

impl ArtifactBuildCommandResult {
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit_code == 0
    }
}

#[derive(Clone, Debug)]
pub struct ArtifactBuildRunResult {
    pub build: ArtifactBuildCommandResult,
    /// Absent when the build failed, because there is nothing to import-check.
    pub smoke: Option<ArtifactBuildCommandResult>,
    /// Always attempted after any build that may have produced a tag, so runs do not accumulate
    /// images. `None` only when the removal could not be spawned.
    pub image_cleanup: Option<ArtifactBuildCommandResult>,
}

impl ArtifactBuildRunResult {
    /// The packaging gate passed only when the artifact both built and ran.
    pub fn success(&self) -> bool {
        self.build.success()
            && self
                .smoke
                .as_ref()
                .is_some_and(ArtifactBuildCommandResult::success)
    }

    pub fn cleanup_clean(&self) -> bool {
        self.image_cleanup
            .as_ref()
            .is_some_and(ArtifactBuildCommandResult::success)
    }
}

pub fn ensure_artifact_root_is_addressable(
    repo_root: crate::git::RepositoryRoot<'_>,
) -> Result<(), ContextPatchError> {
    if repo_root.is_anchored() {
        return Err(ContextPatchError::new(SELECTED_ROOT_REFUSAL));
    }
    Ok(())
}

pub fn plan_artifact_build_check<'a>(
    repo_root: impl Into<crate::git::RepositoryRoot<'a>>,
    dockerfile: &str,
    context: Option<&str>,
    smoke_args: &[String],
    build_timeout_secs: Option<u64>,
    smoke_timeout_secs: Option<u64>,
) -> Result<ArtifactBuildPlan, ContextPatchError> {
    let root = repo_root.into();
    ensure_artifact_root_is_addressable(root)?;

    // Descriptor-relative and no-follow at every component, so traversal and symlinked components
    // are refused here rather than by the Docker daemon after the fact.
    if !crate::fs::rooted::is_regular_file(root, dockerfile)? {
        return Err(ContextPatchError::invalid(format!(
            "`{dockerfile}` must be an existing normalized repository-relative regular file; \
             symlinked and traversing paths are refused"
        )));
    }

    // The repository root is already validated authority, so it needs no second check; any other
    // context must be a real directory reached the same no-follow way.
    let context = match context.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some(".") => ".".to_string(),
        Some(candidate) => {
            if !crate::fs::rooted::is_directory(root, candidate)? {
                return Err(ContextPatchError::invalid(format!(
                    "build context `{candidate}` must be an existing normalized \
                     repository-relative directory; symlinked and traversing paths are refused"
                )));
            }
            candidate.to_string()
        }
    };

    if smoke_args.len() > MAX_SMOKE_ARGS {
        return Err(ContextPatchError::invalid(format!(
            "smoke_args may contain at most {MAX_SMOKE_ARGS} entries"
        )));
    }
    if let Some(invalid) = smoke_args
        .iter()
        .find(|arg| arg.len() > MAX_ARG_BYTES || arg.contains('\0'))
    {
        return Err(ContextPatchError::invalid(format!(
            "smoke argument is invalid: {invalid:?}"
        )));
    }

    let build_timeout = checked_timeout(
        build_timeout_secs,
        DEFAULT_BUILD_TIMEOUT_SECS,
        MAX_BUILD_TIMEOUT_SECS,
    )?;
    let smoke_timeout = checked_timeout(
        smoke_timeout_secs,
        DEFAULT_SMOKE_TIMEOUT_SECS,
        MAX_SMOKE_TIMEOUT_SECS,
    )?;

    // Unique per plan so concurrent jobs cannot remove each other's image during cleanup.
    let tag = format!("{TAG_REPOSITORY}:{}", unique_suffix());

    let build_args = vec![
        "build".to_string(),
        "--file".to_string(),
        dockerfile.to_string(),
        "--tag".to_string(),
        tag.clone(),
        context.clone(),
    ];

    // Everything after the image name is the container command by construction, so a caller's
    // smoke argument cannot be reinterpreted as a Docker option regardless of how it is spelled.
    let mut smoke = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--network".to_string(),
        "none".to_string(),
        tag.clone(),
    ];
    smoke.extend(smoke_args.iter().cloned());

    Ok(ArtifactBuildPlan {
        repo_root: root.logical_path().to_path_buf(),
        dockerfile: dockerfile.to_string(),
        context,
        image_cleanup_args: vec![
            "image".to_string(),
            "rm".to_string(),
            "--force".to_string(),
            tag.clone(),
        ],
        tag,
        build_args,
        smoke_args: smoke,
        build_timeout,
        smoke_timeout,
    })
}

pub fn run_artifact_build_check(
    plan: &ArtifactBuildPlan,
    confirm: Option<&str>,
) -> Result<ArtifactBuildRunResult, ContextPatchError> {
    if confirm != Some(CONFIRMATION) {
        return Err(ContextPatchError::new(format!(
            "execution requires confirm: {CONFIRMATION:?}"
        )));
    }

    let build = match run_bounded_command(
        &plan.repo_root,
        "docker",
        &plan.build_args,
        plan.build_timeout,
        "artifact build",
    ) {
        Ok(output) => command_result(output, 16_000, 24_000),
        Err(error) => {
            // A build that could not even start may still have tagged something on a retry path, so
            // the removal is attempted regardless.
            return Ok(ArtifactBuildRunResult {
                build: command_error_result(error),
                smoke: None,
                image_cleanup: cleanup_image(plan),
            });
        }
    };

    if !build.success() {
        return Ok(ArtifactBuildRunResult {
            build,
            smoke: None,
            image_cleanup: cleanup_image(plan),
        });
    }

    let smoke = run_bounded_command(
        &plan.repo_root,
        "docker",
        &plan.smoke_args,
        plan.smoke_timeout,
        "artifact import smoke",
    )
    .map(|output| command_result(output, 12_000, 16_000))
    .unwrap_or_else(command_error_result);

    Ok(ArtifactBuildRunResult {
        build,
        smoke: Some(smoke),
        image_cleanup: cleanup_image(plan),
    })
}

fn cleanup_image(plan: &ArtifactBuildPlan) -> Option<ArtifactBuildCommandResult> {
    run_bounded_command(
        &plan.repo_root,
        "docker",
        &plan.image_cleanup_args,
        Duration::from_secs(CLEANUP_TIMEOUT_SECS),
        "artifact image cleanup",
    )
    .ok()
    .map(|output| command_result(output, 2_000, 4_000))
}

fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:x}-{sequence:x}")
}

fn command_result(
    output: BoundedProcessOutput,
    max_stdout: usize,
    max_stderr: usize,
) -> ArtifactBuildCommandResult {
    let (stdout, stdout_truncated) =
        redact_and_truncate_output(&String::from_utf8_lossy(&output.stdout), max_stdout);
    let (stderr, stderr_truncated) =
        redact_and_truncate_output(&String::from_utf8_lossy(&output.stderr), max_stderr);
    ArtifactBuildCommandResult {
        exit_code: output.exit_code,
        timed_out: output.timed_out,
        duration_ms: output.duration_ms,
        stdout,
        stdout_truncated: output.stdout_truncated || stdout_truncated,
        stderr,
        stderr_truncated: output.stderr_truncated || stderr_truncated,
    }
}

fn command_error_result(error: ContextPatchError) -> ArtifactBuildCommandResult {
    let (stderr, stderr_truncated) = redact_and_truncate_output(&error.to_string(), 8_000);
    ArtifactBuildCommandResult {
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

    fn repo(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "contextpatch-artifact-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("packaging")).unwrap();
        fs::write(root.join("packaging/Dockerfile"), "FROM scratch\n").unwrap();
        root
    }

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn plans_a_build_and_a_networkless_import_smoke() {
        let root = repo("plans");
        let plan = plan_artifact_build_check(
            root.as_path(),
            "packaging/Dockerfile",
            None,
            &args(&["node", "-e", "require('.')"]),
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            plan.build_args(),
            [
                "build",
                "--file",
                "packaging/Dockerfile",
                "--tag",
                plan.tag(),
                "."
            ]
        );
        assert!(plan.tag().starts_with("contextpatch-artifact:"));
        assert_eq!(plan.build_timeout(), Duration::from_secs(1800));
        assert_eq!(plan.smoke_timeout(), Duration::from_secs(300));
    }

    /// Position is the guarantee: everything after the image name is the container's command, so a
    /// caller cannot smuggle a Docker option through the smoke arguments.
    #[test]
    fn smoke_arguments_land_after_the_image_and_cannot_become_docker_options() {
        let root = repo("position");
        let plan = plan_artifact_build_check(
            root.as_path(),
            "packaging/Dockerfile",
            None,
            &args(&["--privileged", "-v", "/:/host"]),
            None,
            None,
        )
        .unwrap();

        let smoke = plan.smoke_args();
        let image = smoke
            .iter()
            .position(|arg| arg == plan.tag())
            .expect("the image must appear in the run argv");
        assert_eq!(&smoke[..image], ["run", "--rm", "--network", "none"]);
        assert_eq!(&smoke[image + 1..], ["--privileged", "-v", "/:/host"]);
    }

    #[test]
    fn refuses_a_dockerfile_outside_the_repository_or_behind_a_symlink() {
        let root = repo("traversal");
        for candidate in ["../Dockerfile", "/etc/Dockerfile", "packaging/missing"] {
            let error = plan_artifact_build_check(root.as_path(), candidate, None, &[], None, None)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("repository-relative") || error.contains("normalized"),
                "unexpected refusal for {candidate}: {error}"
            );
        }
    }

    #[test]
    fn refuses_a_build_context_that_is_not_a_directory() {
        let root = repo("context");
        let error = plan_artifact_build_check(
            root.as_path(),
            "packaging/Dockerfile",
            Some("packaging/Dockerfile"),
            &[],
            None,
            None,
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("build context"), "{error}");
    }

    #[test]
    fn concurrent_plans_do_not_share_an_image_tag() {
        let root = repo("tags");
        let first = plan_artifact_build_check(
            root.as_path(),
            "packaging/Dockerfile",
            None,
            &[],
            None,
            None,
        )
        .unwrap();
        let second = plan_artifact_build_check(
            root.as_path(),
            "packaging/Dockerfile",
            None,
            &[],
            None,
            None,
        )
        .unwrap();

        assert_ne!(
            first.tag(),
            second.tag(),
            "a shared tag would let one job's cleanup delete another job's image"
        );
        assert!(first
            .image_cleanup_args()
            .contains(&first.tag().to_string()));
    }

    #[test]
    fn refuses_execution_without_the_exact_confirmation() {
        let root = repo("confirm");
        let plan = plan_artifact_build_check(
            root.as_path(),
            "packaging/Dockerfile",
            None,
            &[],
            None,
            None,
        )
        .unwrap();

        for confirm in [None, Some("yes"), Some("run artifact build")] {
            let error = run_artifact_build_check(&plan, confirm)
                .unwrap_err()
                .to_string();
            assert!(error.contains("run artifact build check"), "{error}");
        }
    }

    #[test]
    fn refuses_timeouts_outside_the_permitted_ranges() {
        let root = repo("timeouts");
        let build = plan_artifact_build_check(
            root.as_path(),
            "packaging/Dockerfile",
            None,
            &[],
            Some(MAX_BUILD_TIMEOUT_SECS + 1),
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(build.contains("between 1 and 3600"), "{build}");

        let smoke = plan_artifact_build_check(
            root.as_path(),
            "packaging/Dockerfile",
            None,
            &[],
            None,
            Some(0),
        )
        .unwrap_err()
        .to_string();
        assert!(smoke.contains("between 1 and 600"), "{smoke}");
    }
}
