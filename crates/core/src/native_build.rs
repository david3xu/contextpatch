use std::path::{Path, PathBuf};

use crate::error::ContextPatchError;
use crate::git::status::status_short;
use crate::process::runner::{
    checked_timeout, display_command, resolve_cwd, run_no_shell_command,
    validate_common_command_shape,
};
use crate::setup::profile::{validate_non_empty_single_line, validate_relative_path_param};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeBuildParams {
    Ios {
        workspace: String,
        scheme: String,
        configuration: Option<String>,
        sdk: Option<String>,
        destination: Option<String>,
        derived_data_path: Option<String>,
    },
    Android {
        gradlew: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBuildPlan {
    pub action: String,
    pub program: String,
    pub display_program: String,
    pub args: Vec<String>,
    pub repo_validation: bool,
    pub mutates_repo_source: bool,
}

impl NativeBuildPlan {
    pub fn display(&self) -> String {
        display_command(&self.display_program, &self.args)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBuildResult {
    pub action: String,
    pub dry_run: bool,
    pub cwd: PathBuf,
    pub plan: NativeBuildPlan,
    pub execution: Option<NativeBuildExecution>,
}

impl NativeBuildResult {
    pub fn summary(&self) -> String {
        let mut summary = format!(
            "action: {}\ndry_run: {}\nrepo_validation: {}\nmutates_repo_source: {}\ncommand: {}\ncwd: {}",
            self.action,
            self.dry_run,
            self.plan.repo_validation,
            self.plan.mutates_repo_source,
            self.plan.display(),
            self.cwd.display()
        );
        if let Some(execution) = &self.execution {
            summary.push_str(&format!(
                "\nexecuted: true\nexit_code: {}\ntimed_out: {}\nduration_ms: {}\nsource_status_unchanged: {}\nstatus_before:\n{}\nstatus_after:\n{}\nstdout:\n{}\nstderr:\n{}",
                execution.exit_code,
                execution.timed_out,
                execution.duration_ms,
                execution.source_status_unchanged,
                empty_label(&execution.status_before),
                empty_label(&execution.status_after),
                empty_label(&execution.stdout),
                empty_label(&execution.stderr)
            ));
        }
        summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeBuildExecution {
    pub exit_code: i32,
    pub timed_out: bool,
    pub duration_ms: u128,
    pub stdout: String,
    pub stderr: String,
    pub status_before: String,
    pub status_after: String,
    pub source_status_unchanged: bool,
}

pub fn native_build_run(
    repo_root: &Path,
    cwd: Option<&Path>,
    action: &str,
    params: NativeBuildParams,
    timeout_secs: Option<u64>,
    dry_run: bool,
) -> Result<NativeBuildResult, ContextPatchError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    let cwd = resolve_cwd(&root, cwd)?;
    let timeout = checked_timeout(timeout_secs)?;
    let plan = plan_native_build(&root, action, params)?;

    let execution = if dry_run {
        None
    } else {
        let status_before = status_short(&root)?;
        let output =
            run_no_shell_command(&cwd, &plan.program, &plan.args, timeout, "native_build_run")?;
        let status_after = status_short(&root)?;
        let source_status_unchanged = status_after == status_before;
        if !source_status_unchanged {
            return Err(ContextPatchError::new(format!(
                "native_build_run refused after execution: build command changed repository source status\nbefore:\n{}\nafter:\n{}",
                empty_label(&status_before),
                empty_label(&status_after)
            )));
        }
        if output.timed_out || output.exit_code != 0 {
            return Err(ContextPatchError::new(format!(
                "native_build_run command failed\nexit_code: {}\ntimed_out: {}\nsource_status_unchanged: {}\nstdout:\n{}\nstderr:\n{}",
                output.exit_code,
                output.timed_out,
                source_status_unchanged,
                empty_label(&output.stdout),
                empty_label(&output.stderr)
            )));
        }
        Some(NativeBuildExecution {
            exit_code: output.exit_code,
            timed_out: output.timed_out,
            duration_ms: output.duration_ms,
            stdout: output.stdout,
            stderr: output.stderr,
            status_before,
            status_after,
            source_status_unchanged,
        })
    };

    Ok(NativeBuildResult {
        action: action.to_string(),
        dry_run,
        cwd,
        plan,
        execution,
    })
}

fn plan_native_build(
    root: &Path,
    action: &str,
    params: NativeBuildParams,
) -> Result<NativeBuildPlan, ContextPatchError> {
    match action {
        "ios_build" | "ios_test" => plan_ios(action, params),
        "android_assemble_debug" | "android_unit_test" => plan_android(root, action, params),
        _ => Err(ContextPatchError::new(format!(
            "native_build_run refused: unknown action `{action}`"
        ))),
    }
}

fn plan_ios(action: &str, params: NativeBuildParams) -> Result<NativeBuildPlan, ContextPatchError> {
    let NativeBuildParams::Ios {
        workspace,
        scheme,
        configuration,
        sdk,
        destination,
        derived_data_path,
    } = params
    else {
        return Err(ContextPatchError::new(format!(
            "native_build_run refused: {action} requires iOS params"
        )));
    };
    validate_relative_path_param("native_build_run", "workspace", &workspace)?;
    if !(workspace.ends_with(".xcworkspace") || workspace.ends_with(".xcodeproj")) {
        return Err(ContextPatchError::new(
            "native_build_run refused: workspace must end with .xcworkspace or .xcodeproj",
        ));
    }
    validate_non_empty_single_line("native_build_run", "scheme", &scheme, 120)?;
    let configuration = configuration.unwrap_or_else(|| "Debug".to_string());
    let sdk = sdk.unwrap_or_else(|| "iphonesimulator".to_string());
    validate_xcode_value("configuration", &configuration)?;
    validate_xcode_value("sdk", &sdk)?;

    let mut args = Vec::new();
    if workspace.ends_with(".xcworkspace") {
        args.push("-workspace".to_string());
    } else {
        args.push("-project".to_string());
    }
    args.push(workspace);
    args.push("-scheme".to_string());
    args.push(scheme);
    args.push("-configuration".to_string());
    args.push(configuration);
    args.push("-sdk".to_string());
    args.push(sdk);
    if let Some(destination) = destination {
        validate_non_empty_single_line("native_build_run", "destination", &destination, 240)?;
        args.push("-destination".to_string());
        args.push(destination);
    }
    if let Some(derived_data_path) = derived_data_path {
        validate_relative_path_param("native_build_run", "derived_data_path", &derived_data_path)?;
        args.push("-derivedDataPath".to_string());
        args.push(derived_data_path);
    }
    args.push(
        match action {
            "ios_build" => "build",
            "ios_test" => "test",
            _ => unreachable!(),
        }
        .to_string(),
    );

    validate_common_command_shape("xcodebuild", &args)?;
    Ok(NativeBuildPlan {
        action: action.to_string(),
        program: "xcodebuild".to_string(),
        display_program: "xcodebuild".to_string(),
        args,
        repo_validation: true,
        mutates_repo_source: false,
    })
}

fn plan_android(
    root: &Path,
    action: &str,
    params: NativeBuildParams,
) -> Result<NativeBuildPlan, ContextPatchError> {
    let NativeBuildParams::Android { gradlew } = params else {
        return Err(ContextPatchError::new(format!(
            "native_build_run refused: {action} requires Android params"
        )));
    };
    let gradlew = gradlew.unwrap_or_else(|| "gradlew".to_string());
    validate_relative_path_param("native_build_run", "gradlew", &gradlew)?;
    if !gradlew.ends_with("gradlew") {
        return Err(ContextPatchError::new(
            "native_build_run refused: gradlew must point to a Gradle wrapper named gradlew",
        ));
    }
    let executable = resolve_repo_relative_executable(root, &gradlew)?;
    let args = vec![match action {
        "android_assemble_debug" => "assembleDebug",
        "android_unit_test" => "testDebugUnitTest",
        _ => unreachable!(),
    }
    .to_string()];
    Ok(NativeBuildPlan {
        action: action.to_string(),
        program: executable.display().to_string(),
        display_program: format!("./{gradlew}"),
        args,
        repo_validation: true,
        mutates_repo_source: false,
    })
}

fn resolve_repo_relative_executable(
    root: &Path,
    relative: &str,
) -> Result<PathBuf, ContextPatchError> {
    let candidate = root.join(relative);
    let resolved = candidate.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "native_build_run refused: failed to resolve executable {relative}: {error}"
        ))
    })?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(ContextPatchError::new(
            "native_build_run refused: executable must be a file inside the repository",
        ));
    }
    Ok(resolved)
}

fn validate_xcode_value(field: &str, value: &str) -> Result<(), ContextPatchError> {
    validate_non_empty_single_line("native_build_run", field, value, 120)?;
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ContextPatchError::new(format!(
            "native_build_run refused: {field} contains unsupported characters"
        )));
    }
    Ok(())
}

fn empty_label(value: &str) -> &str {
    if value.trim().is_empty() {
        "(empty)"
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{native_build_run, NativeBuildParams};

    #[test]
    fn plans_ios_build_without_raw_command() {
        let root = git_root("plans_ios_build_without_raw_command");

        let result = native_build_run(
            &root,
            None,
            "ios_build",
            NativeBuildParams::Ios {
                workspace: "ios/App/App.xcworkspace".to_string(),
                scheme: "App".to_string(),
                configuration: None,
                sdk: None,
                destination: None,
                derived_data_path: Some(".contextpatch-derived-data".to_string()),
            },
            Some(30),
            true,
        )
        .unwrap();

        assert_eq!(result.plan.program, "xcodebuild");
        assert_eq!(
            result.plan.args,
            [
                "-workspace",
                "ios/App/App.xcworkspace",
                "-scheme",
                "App",
                "-configuration",
                "Debug",
                "-sdk",
                "iphonesimulator",
                "-derivedDataPath",
                ".contextpatch-derived-data",
                "build"
            ]
        );
    }

    #[test]
    fn plans_android_wrapper_as_repo_relative_executable() {
        let root = git_root("plans_android_wrapper_as_repo_relative_executable");
        fs::write(root.join("gradlew"), "#!/bin/sh\nexit 0\n").unwrap();

        let result = native_build_run(
            &root,
            None,
            "android_assemble_debug",
            NativeBuildParams::Android { gradlew: None },
            Some(30),
            true,
        )
        .unwrap();

        assert!(result.plan.program.ends_with("/gradlew"));
        assert_eq!(result.plan.display(), "./gradlew assembleDebug");
    }

    #[test]
    fn refuses_invalid_ios_workspace_and_unknown_action() {
        let root = git_root("refuses_invalid_ios_workspace_and_unknown_action");

        let invalid = native_build_run(
            &root,
            None,
            "ios_build",
            NativeBuildParams::Ios {
                workspace: "../App.xcworkspace".to_string(),
                scheme: "App".to_string(),
                configuration: None,
                sdk: None,
                destination: None,
                derived_data_path: None,
            },
            Some(30),
            true,
        )
        .unwrap_err();
        assert!(invalid.to_string().contains("repository-relative path"));

        let unknown = native_build_run(
            &root,
            None,
            "unknown",
            NativeBuildParams::Android { gradlew: None },
            Some(30),
            true,
        )
        .unwrap_err();
        assert!(unknown.to_string().contains("unknown action"));

        let invalid_derived_data = native_build_run(
            &root,
            None,
            "ios_build",
            NativeBuildParams::Ios {
                workspace: "ios/App/App.xcodeproj".to_string(),
                scheme: "App".to_string(),
                configuration: None,
                sdk: None,
                destination: None,
                derived_data_path: Some("../DerivedData".to_string()),
            },
            Some(30),
            true,
        )
        .unwrap_err();
        assert!(invalid_derived_data
            .to_string()
            .contains("repository-relative path"));
    }

    fn git_root(name: &str) -> PathBuf {
        let root = temp_root(name);
        run_git(&root, &["init", "--quiet"]);
        root
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

    fn run_git(root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
}
