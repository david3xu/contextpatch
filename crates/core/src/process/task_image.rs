use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

use crate::error::ContextPatchError;
use crate::process::guarded_command::redact_and_truncate_output;
use crate::process::runner::{run_bounded_command, BoundedProcessOutput};

pub const CONFIRMATION: &str = "run task image python";
const TASK_ENVIRONMENT: &str = "task/environment";
const TASK_DOCKERFILE: &str = "task/environment/Dockerfile";
const MAX_ARGS: usize = 32;
const MAX_ARG_BYTES: usize = 4096;
const MAX_RUN_TIMEOUT_SECS: u64 = 600;
const MAX_BUILD_TIMEOUT_SECS: u64 = 1800;
const CLEANUP_TIMEOUT_SECS: u64 = 30;

#[derive(Clone, Debug)]
pub struct TaskImagePlan {
    repo_root: PathBuf,
    script: String,
    cache_image: String,
    image: String,
    container: String,
    build_args: Vec<String>,
    run_args: Vec<String>,
    container_cleanup_args: Vec<String>,
    image_cleanup_args: Vec<String>,
    build_timeout: Duration,
    run_timeout: Duration,
}

impl TaskImagePlan {
    pub fn script(&self) -> &str {
        &self.script
    }

    pub fn cache_image(&self) -> &str {
        &self.cache_image
    }

    pub fn image(&self) -> &str {
        &self.image
    }

    pub fn container(&self) -> &str {
        &self.container
    }

    pub fn build_args(&self) -> &[String] {
        &self.build_args
    }

    pub fn run_args(&self) -> &[String] {
        &self.run_args
    }

    pub fn container_cleanup_args(&self) -> &[String] {
        &self.container_cleanup_args
    }

    pub fn image_cleanup_args(&self) -> &[String] {
        &self.image_cleanup_args
    }

    pub fn build_timeout(&self) -> Duration {
        self.build_timeout
    }

    pub fn run_timeout(&self) -> Duration {
        self.run_timeout
    }
}

#[derive(Clone, Debug)]
pub struct TaskImageCommandResult {
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout: String,
    pub stdout_truncated: bool,
    pub stderr: String,
    pub stderr_truncated: bool,
}

impl TaskImageCommandResult {
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit_code == 0
    }
}

#[derive(Clone, Debug)]
pub struct TaskImageRunResult {
    pub build: TaskImageCommandResult,
    pub run: Option<TaskImageCommandResult>,
    pub container_cleanup: Option<TaskImageCommandResult>,
    pub image_cleanup: Option<TaskImageCommandResult>,
}

impl TaskImageRunResult {
    pub fn success(&self) -> bool {
        self.build.success()
            && self
                .run
                .as_ref()
                .is_some_and(TaskImageCommandResult::success)
            && self
                .container_cleanup
                .as_ref()
                .map(TaskImageCommandResult::success)
                .unwrap_or(true)
            && self
                .image_cleanup
                .as_ref()
                .map(TaskImageCommandResult::success)
                .unwrap_or(true)
    }
}

pub fn plan_task_image_python_run(
    repo_root: &Path,
    script: &str,
    program: &str,
    args: &[String],
    timeout_secs: Option<u64>,
    build_timeout_secs: Option<u64>,
) -> Result<TaskImagePlan, ContextPatchError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    if !root.is_dir() {
        return Err(ContextPatchError::new(format!(
            "repository root {} is not a directory",
            root.display()
        )));
    }
    if root.to_string_lossy().contains(',') {
        return Err(ContextPatchError::new(
            "repository root contains a comma and cannot be represented safely as a Docker mount",
        ));
    }

    let script = normalize_relative(script)?;
    if !script.ends_with(".py") {
        return Err(ContextPatchError::new(
            "script must be a repository-relative .py file",
        ));
    }
    resolve_without_symlinks(&root, &script, false)?;
    resolve_without_symlinks(&root, TASK_ENVIRONMENT, true)?;
    resolve_without_symlinks(&root, TASK_DOCKERFILE, false)?;

    if !matches!(program, "python3" | "python") {
        return Err(ContextPatchError::new(
            "program must be `python3` or `python`",
        ));
    }
    if args.len() > MAX_ARGS {
        return Err(ContextPatchError::new(format!(
            "args may contain at most {MAX_ARGS} entries"
        )));
    }
    if args
        .iter()
        .any(|arg| arg.contains('\0') || arg.len() > MAX_ARG_BYTES)
    {
        return Err(ContextPatchError::new(format!(
            "each arg must omit NUL and contain at most {MAX_ARG_BYTES} bytes"
        )));
    }

    let run_timeout = checked_timeout(timeout_secs, 120, MAX_RUN_TIMEOUT_SECS)?;
    let build_timeout = checked_timeout(build_timeout_secs, 600, MAX_BUILD_TIMEOUT_SECS)?;
    let (cache_image, image, container) = task_image_identity(&root);
    let dockerfile = root.join(TASK_DOCKERFILE);
    let build_context = root.join(TASK_ENVIRONMENT);
    let build_args = vec![
        "build".to_string(),
        "--file".to_string(),
        dockerfile.display().to_string(),
        "--tag".to_string(),
        cache_image.clone(),
        "--tag".to_string(),
        image.clone(),
        build_context.display().to_string(),
    ];
    let container_script = format!("/workspace/{script}");
    let mount = format!(
        "type=bind,source={},target=/workspace,readonly",
        root.display()
    );
    let mut run_args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--name".to_string(),
        container.clone(),
        "--network".to_string(),
        "none".to_string(),
        "--cap-drop".to_string(),
        "ALL".to_string(),
        "--security-opt".to_string(),
        "no-new-privileges".to_string(),
        "--pids-limit".to_string(),
        "256".to_string(),
        "--read-only".to_string(),
        "--tmpfs".to_string(),
        "/tmp:rw,nosuid,nodev,size=512m".to_string(),
        "--mount".to_string(),
        mount,
        "--workdir".to_string(),
        "/workspace".to_string(),
        "--env".to_string(),
        "HOME=/tmp".to_string(),
        "--env".to_string(),
        "PYTHONDONTWRITEBYTECODE=1".to_string(),
        "--entrypoint".to_string(),
        program.to_string(),
        image.clone(),
        container_script,
    ];
    run_args.extend(args.iter().cloned());
    let container_cleanup_args = vec!["rm".to_string(), "--force".to_string(), container.clone()];
    let image_cleanup_args = vec![
        "image".to_string(),
        "rm".to_string(),
        "--force".to_string(),
        image.clone(),
    ];

    Ok(TaskImagePlan {
        repo_root: root,
        script,
        cache_image,
        image,
        container,
        build_args,
        run_args,
        container_cleanup_args,
        image_cleanup_args,
        build_timeout,
        run_timeout,
    })
}

pub fn run_task_image_python(
    plan: &TaskImagePlan,
    confirm: Option<&str>,
) -> Result<TaskImageRunResult, ContextPatchError> {
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
        "task image build",
    ) {
        Ok(output) => command_result(output, 12_000, 24_000),
        Err(error) => {
            let build = command_error_result(error, 24_000);
            let image_cleanup = Some(run_cleanup_command(
                plan,
                &plan.image_cleanup_args,
                "task image execution tag",
            ));
            return Ok(TaskImageRunResult {
                build,
                run: None,
                container_cleanup: None,
                image_cleanup,
            });
        }
    };
    if !build.success() {
        let image_cleanup = Some(run_cleanup_command(
            plan,
            &plan.image_cleanup_args,
            "task image execution tag",
        ));
        return Ok(TaskImageRunResult {
            build,
            run: None,
            container_cleanup: None,
            image_cleanup,
        });
    }

    let (run, container_cleanup) = match run_bounded_command(
        &plan.repo_root,
        "docker",
        &plan.run_args,
        plan.run_timeout,
        "task image Python",
    ) {
        Ok(output) => {
            let run = command_result(output, 120_000, 40_000);
            let container_cleanup = (run.timed_out || run.exit_code == -1 || run.exit_code == 125)
                .then(|| {
                    run_cleanup_command(plan, &plan.container_cleanup_args, "task image container")
                });
            (run, container_cleanup)
        }
        Err(error) => (
            command_error_result(error, 40_000),
            Some(run_cleanup_command(
                plan,
                &plan.container_cleanup_args,
                "task image container",
            )),
        ),
    };
    let image_cleanup = Some(run_cleanup_command(
        plan,
        &plan.image_cleanup_args,
        "task image execution tag",
    ));
    Ok(TaskImageRunResult {
        build,
        run: Some(run),
        container_cleanup,
        image_cleanup,
    })
}

fn run_cleanup_command(
    plan: &TaskImagePlan,
    args: &[String],
    operation_label: &str,
) -> TaskImageCommandResult {
    match run_bounded_command(
        &plan.repo_root,
        "docker",
        args,
        Duration::from_secs(CLEANUP_TIMEOUT_SECS),
        operation_label,
    ) {
        Ok(output) => command_result(output, 12_000, 24_000),
        Err(error) => command_error_result(error, 24_000),
    }
}

fn command_error_result(error: ContextPatchError, max_stderr: usize) -> TaskImageCommandResult {
    let (stderr, stderr_truncated) = redact_and_truncate_output(&error.to_string(), max_stderr);
    TaskImageCommandResult {
        exit_code: -1,
        timed_out: false,
        duration_ms: 0,
        stdout: String::new(),
        stdout_truncated: false,
        stderr,
        stderr_truncated,
    }
}

fn command_result(
    output: BoundedProcessOutput,
    max_stdout: usize,
    max_stderr: usize,
) -> TaskImageCommandResult {
    let (stdout, stdout_truncated) =
        redact_and_truncate_output(&String::from_utf8_lossy(&output.stdout), max_stdout);
    let (stderr, stderr_truncated) =
        redact_and_truncate_output(&String::from_utf8_lossy(&output.stderr), max_stderr);
    TaskImageCommandResult {
        exit_code: output.exit_code,
        timed_out: output.timed_out,
        duration_ms: output.duration_ms,
        stdout,
        stdout_truncated: output.stdout_truncated || stdout_truncated,
        stderr,
        stderr_truncated: output.stderr_truncated || stderr_truncated,
    }
}

fn checked_timeout(
    requested: Option<u64>,
    default_secs: u64,
    max_secs: u64,
) -> Result<Duration, ContextPatchError> {
    let seconds = requested.unwrap_or(default_secs);
    if seconds == 0 || seconds > max_secs {
        return Err(ContextPatchError::new(format!(
            "timeout must be between 1 and {max_secs} seconds"
        )));
    }
    Ok(Duration::from_secs(seconds))
}

fn normalize_relative(raw: &str) -> Result<String, ContextPatchError> {
    if raw.is_empty() || raw.contains('\0') || raw.contains('\\') {
        return Err(ContextPatchError::new(
            "path must be a non-empty normalized repository-relative path",
        ));
    }
    let path = Path::new(raw);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ContextPatchError::new(
            "path must be a normalized repository-relative path",
        ));
    }
    Ok(path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn resolve_without_symlinks(
    root: &Path,
    relative: &str,
    require_directory: bool,
) -> Result<PathBuf, ContextPatchError> {
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            ContextPatchError::new(format!("failed to inspect `{relative}`: {error}"))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ContextPatchError::new(format!(
                "`{relative}` must not contain symlink components"
            )));
        }
    }
    let metadata = fs::metadata(&current).map_err(|error| {
        ContextPatchError::new(format!("failed to inspect `{relative}`: {error}"))
    })?;
    let correct_type = if require_directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    if !correct_type {
        let expected = if require_directory {
            "directory"
        } else {
            "regular file"
        };
        return Err(ContextPatchError::new(format!(
            "`{relative}` is not an existing {expected}"
        )));
    }
    let resolved = current.canonicalize().map_err(|error| {
        ContextPatchError::new(format!("failed to resolve `{relative}`: {error}"))
    })?;
    if !resolved.starts_with(root) {
        return Err(ContextPatchError::new(format!(
            "`{relative}` resolves outside repository root"
        )));
    }
    Ok(resolved)
}

fn task_image_identity(root: &Path) -> (String, String, String) {
    static NEXT_JOB: AtomicU64 = AtomicU64::new(0);

    let mut root_hasher = Sha256::new();
    root_hasher.update(root.as_os_str().to_string_lossy().as_bytes());
    let root_digest = format!("{:x}", root_hasher.finalize());

    let sequence = NEXT_JOB.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut job_hasher = Sha256::new();
    job_hasher.update(root_digest.as_bytes());
    job_hasher.update(std::process::id().to_le_bytes());
    job_hasher.update(now.to_le_bytes());
    job_hasher.update(sequence.to_le_bytes());
    let job_digest = format!("{:x}", job_hasher.finalize());

    let cache_image = format!("contextpatch-task-image:{}", &root_digest[..16]);
    let image = format!(
        "contextpatch-task-image:{}-{}",
        &root_digest[..12],
        &job_digest[..16]
    );
    let container = format!(
        "contextpatch-task-{}-{}",
        &root_digest[..12],
        &job_digest[..16]
    );
    (cache_image, image, container)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn plans_a_hardened_task_image_python_invocation() {
        let root = temp_root("plans-task-image");
        fs::create_dir_all(root.join(TASK_ENVIRONMENT)).unwrap();
        fs::write(root.join(TASK_DOCKERFILE), "FROM python:3.13-slim\n").unwrap();
        fs::create_dir(root.join("tools")).unwrap();
        fs::write(root.join("tools/calibrate.py"), "print('ok')\n").unwrap();

        let plan = plan_task_image_python_run(
            &root,
            "tools/calibrate.py",
            "python3",
            &["--cell".to_string(), "3".to_string()],
            Some(30),
            Some(60),
        )
        .unwrap();

        assert_eq!(plan.script(), "tools/calibrate.py");
        assert_eq!(plan.build_args()[0], "build");
        assert_ne!(plan.cache_image(), plan.image());
        assert!(plan
            .build_args()
            .windows(2)
            .any(|pair| pair == ["--tag", plan.cache_image()]));
        assert!(plan
            .build_args()
            .windows(2)
            .any(|pair| pair == ["--tag", plan.image()]));
        assert!(plan
            .run_args()
            .windows(2)
            .any(|pair| pair == ["--name", plan.container()]));
        assert!(plan
            .run_args()
            .windows(2)
            .any(|pair| pair == ["--network", "none"]));
        assert!(plan
            .run_args()
            .windows(2)
            .any(|pair| pair == ["--cap-drop", "ALL"]));
        assert!(plan.run_args().iter().any(|arg| arg == "--read-only"));
        assert!(plan
            .run_args()
            .iter()
            .any(|arg| arg == "/workspace/tools/calibrate.py"));
        assert_eq!(plan.run_args().last().map(String::as_str), Some("3"));
    }

    #[test]
    fn plans_unique_execution_images_and_containers_per_job() {
        let root = temp_root("unique-task-images");
        fs::create_dir_all(root.join(TASK_ENVIRONMENT)).unwrap();
        fs::write(root.join(TASK_DOCKERFILE), "FROM python:3.13-slim\n").unwrap();
        fs::write(root.join("script.py"), "print('ok')\n").unwrap();

        let first =
            plan_task_image_python_run(&root, "script.py", "python3", &[], None, None).unwrap();
        let second =
            plan_task_image_python_run(&root, "script.py", "python3", &[], None, None).unwrap();

        assert_eq!(first.cache_image(), second.cache_image());
        assert_ne!(first.image(), second.image());
        assert_ne!(first.container(), second.container());
        assert!(first.run_args().iter().any(|arg| arg == first.image()));
        assert!(!first.run_args().iter().any(|arg| arg == second.image()));
    }

    #[test]
    fn refuses_non_python_and_symlinked_scripts() {
        let root = temp_root("refuses-task-image-script");
        fs::create_dir_all(root.join(TASK_ENVIRONMENT)).unwrap();
        fs::write(root.join(TASK_DOCKERFILE), "FROM python:3.13-slim\n").unwrap();
        fs::write(root.join("script.txt"), "no\n").unwrap();
        let error = plan_task_image_python_run(&root, "script.txt", "python3", &[], None, None)
            .unwrap_err();
        assert!(error.to_string().contains(".py"));

        #[cfg(unix)]
        {
            fs::write(root.join("real.py"), "print('ok')\n").unwrap();
            std::os::unix::fs::symlink(root.join("real.py"), root.join("linked.py")).unwrap();
            let error = plan_task_image_python_run(&root, "linked.py", "python3", &[], None, None)
                .unwrap_err();
            assert!(error.to_string().contains("symlink"));
        }
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
