use std::path::{Path, PathBuf};

use crate::error::ContextPatchError;
use crate::git::status::status_short;
use crate::git::RepositoryRoot;
use crate::process::runner::{
    checked_timeout, display_command, resolve_child_cwd, run_no_shell_command,
    validate_common_command_shape,
};
use crate::setup::profile::{validate_non_empty_single_line, validate_relative_path_param};

/// The refusal a selected repository receives from the Gradle actions.
///
/// Executing a program means handing `exec` a pathname. Every other part of a native build can be anchored:
/// the working directory is a retained descriptor, and the arguments are repository relative. The Gradle
/// wrapper cannot, because it is a file *inside* the repository that has to be named to be run, and there is
/// no descriptor form of a program argument. Naming it would mean resolving a path at exec time, so the
/// executable the child runs could differ from the file that was verified.
///
/// The Xcode actions are unaffected: `xcodebuild` is resolved from the tool path rather than from the
/// repository, so only its arguments refer to repository contents and those stay relative.
///
/// A configured root is unaffected too: it is reached by name already, and its pathname is the authority
/// rather than a stand-in for one.
pub const SELECTED_ROOT_GRADLE_REFUSAL: &str =
    "native_build_run refused: Gradle actions are unavailable for a selected repository because the Gradle \
     wrapper is a repository file that can only be run by naming it, which would substitute a pathname for \
     the selected repository's authority; run this action against the configured repository root instead";

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

/// The Gradle wrapper as the manifest advertises it, which is the default the planner assumes.
const ADVERTISED_GRADLE_PROGRAM: &str = "./gradlew";

/// One `native_build_run` action.
///
/// Flat rather than split by platform, because nothing in the call selects a platform. A setup
/// profile is named by the caller and selects a vocabulary; here the platform is a property of the
/// action itself, so the advertised set is one enum and the partition is derived from it rather than
/// supplied alongside it.
///
/// `ALL` is the single source for the advertised schema keyword and for the capability manifest.
/// Safety-contract clause 34 leaves the keyword advisory with a core guard behind it, and `parse` is
/// that guard.
///
/// A variant left out of `ALL` does not compile. `ALL` is the only site that constructs one, so an
/// omitted variant is constructed nowhere and the dead-code lint refuses it under `-D warnings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    IosBuild,
    IosTest,
    AndroidAssembleDebug,
    AndroidUnitTest,
}

/// Which planner owns an action, carrying the verb that planner appends.
///
/// The route and the verb come from one exhaustive match rather than two. Both were previously
/// recovered by matching the action name twice, once here to route and once inside each planner to
/// choose the verb, and the inner matches were exhaustive only because the outer one had already
/// agreed with them. Nothing enforced that agreement, so each inner match ended in `unreachable!()`:
/// a panic guarded by an invariant held in two places and checked in neither. Returning the verb
/// with the route deletes the second match, which leaves nothing to be unreachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Ios(&'static str),
    Android(&'static str),
}

impl Action {
    /// Every action, in the order the surfaces advertise them.
    pub const ALL: &'static [Self] = &[
        Self::IosBuild,
        Self::IosTest,
        Self::AndroidAssembleDebug,
        Self::AndroidUnitTest,
    ];

    /// The wire name, which is the only form a caller supplies or reads.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IosBuild => "ios_build",
            Self::IosTest => "ios_test",
            Self::AndroidAssembleDebug => "android_assemble_debug",
            Self::AndroidUnitTest => "android_unit_test",
        }
    }

    /// The advertised names, for surfaces that must report the set rather than restate it.
    pub fn advertised_names() -> Vec<&'static str> {
        Self::ALL.iter().map(|action| action.as_str()).collect()
    }

    /// The program the manifest advertises for this action.
    pub const fn advertised_program(self) -> &'static str {
        match self.platform() {
            Platform::Ios(_) => "xcodebuild",
            Platform::Android(_) => ADVERTISED_GRADLE_PROGRAM,
        }
    }

    /// Whether this action accepts a repository-relative derived data path.
    pub const fn supports_repo_relative_derived_data_path(self) -> bool {
        matches!(self.platform(), Platform::Ios(_))
    }

    const fn platform(self) -> Platform {
        match self {
            Self::IosBuild => Platform::Ios("build"),
            Self::IosTest => Platform::Ios("test"),
            Self::AndroidAssembleDebug => Platform::Android("assembleDebug"),
            Self::AndroidUnitTest => Platform::Android("testDebugUnitTest"),
        }
    }

    pub(crate) fn parse(action: &str) -> Result<Self, ContextPatchError> {
        Self::ALL
            .iter()
            .copied()
            .find(|candidate| candidate.as_str() == action)
            .ok_or_else(|| {
                ContextPatchError::new(format!(
                    "native_build_run refused: unknown action `{action}`"
                ))
            })
    }
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

pub fn native_build_run<'a>(
    repository_root: impl Into<RepositoryRoot<'a>>,
    cwd: Option<&Path>,
    action: &str,
    params: NativeBuildParams,
    timeout_secs: Option<u64>,
    dry_run: bool,
) -> Result<NativeBuildResult, ContextPatchError> {
    // Planning, the status snapshots that bracket the build, and the child's working directory all derive
    // from one authority, so a build cannot be planned against one repository and run in another.
    let root = repository_root.into();
    let cwd = resolve_child_cwd(root, cwd)?;
    let timeout = checked_timeout(timeout_secs)?;
    let plan = plan_native_build(root, Action::parse(action)?, params)?;

    let execution = if dry_run {
        None
    } else {
        let status_before = status_short(root)?;
        let output = run_no_shell_command(
            cwd.command_cwd(),
            &plan.program,
            &plan.args,
            timeout,
            "native_build_run",
        )?;
        let status_after = status_short(root)?;
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
        cwd: cwd.logical_path().to_path_buf(),
        plan,
        execution,
    })
}

fn plan_native_build(
    root: RepositoryRoot<'_>,
    action: Action,
    params: NativeBuildParams,
) -> Result<NativeBuildPlan, ContextPatchError> {
    match action.platform() {
        Platform::Ios(verb) => plan_ios(action, verb, params),
        Platform::Android(verb) => plan_android(root, action, verb, params),
    }
}

fn plan_ios(
    action: Action,
    verb: &'static str,
    params: NativeBuildParams,
) -> Result<NativeBuildPlan, ContextPatchError> {
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
            "native_build_run refused: {} requires iOS params",
            action.as_str()
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
    args.push(verb.to_string());

    validate_common_command_shape("xcodebuild", &args)?;
    Ok(NativeBuildPlan {
        action: action.as_str().to_string(),
        program: "xcodebuild".to_string(),
        display_program: "xcodebuild".to_string(),
        args,
        repo_validation: true,
        mutates_repo_source: false,
    })
}

fn plan_android(
    root: RepositoryRoot<'_>,
    action: Action,
    verb: &'static str,
    params: NativeBuildParams,
) -> Result<NativeBuildPlan, ContextPatchError> {
    let NativeBuildParams::Android { gradlew } = params else {
        return Err(ContextPatchError::new(format!(
            "native_build_run refused: {} requires Android params",
            action.as_str()
        )));
    };
    // Gated here, during planning, so a selection is refused before any command is built or run and a caller
    // cannot plan against a selection and then execute the plan.
    ensure_gradle_root_is_addressable(root)?;
    let gradlew = gradlew.unwrap_or_else(|| "gradlew".to_string());
    validate_relative_path_param("native_build_run", "gradlew", &gradlew)?;
    if !gradlew.ends_with("gradlew") {
        return Err(ContextPatchError::new(
            "native_build_run refused: gradlew must point to a Gradle wrapper named gradlew",
        ));
    }
    let executable = resolve_repo_relative_executable(root, &gradlew)?;
    let args = vec![verb.to_string()];
    Ok(NativeBuildPlan {
        action: action.as_str().to_string(),
        program: executable,
        display_program: format!("./{gradlew}"),
        args,
        repo_validation: true,
        mutates_repo_source: false,
    })
}

/// Refuse a Gradle action that cannot name its wrapper without substituting a path for authority.
///
/// Applied during planning, which is also what a dry run performs, so the refusal is identical whether a
/// caller plans or executes.
pub fn ensure_gradle_root_is_addressable(
    repo_root: RepositoryRoot<'_>,
) -> Result<(), ContextPatchError> {
    if repo_root.is_anchored() {
        return Err(ContextPatchError::new(SELECTED_ROOT_GRADLE_REFUSAL));
    }
    Ok(())
}

/// Confirm one repository-relative executable and render the pathname `exec` will be given.
///
/// The file is confirmed through the root's own authority, so a symlink at any component is refused rather
/// than followed and the check cannot leave the repository. Only after that is a pathname composed, and only
/// because a program argument has no descriptor form. A selected root never reaches this point.
fn resolve_repo_relative_executable(
    root: RepositoryRoot<'_>,
    relative: &str,
) -> Result<String, ContextPatchError> {
    let inspected = crate::fs::guarded_file::inspect_path_in_root(root, Path::new(relative))
        .map_err(|error| {
            ContextPatchError::new(format!(
                "native_build_run refused: failed to resolve executable {relative}: {error}"
            ))
        })?;
    let executable = inspected
        .filter(|inspection| {
            inspection.kind == crate::fs::guarded_file::GuardedPathKind::RegularFile
        })
        .ok_or_else(|| {
            ContextPatchError::new(
                "native_build_run refused: executable must be a file inside the repository",
            )
        })?;
    let _ = executable;
    Ok(root.logical_path().join(relative).display().to_string())
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

    use super::{native_build_run, Action, NativeBuildParams};

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

    /// Every declared action reaches a planner, and each is routed to the one matching its platform.
    ///
    /// The population comes from `ALL` rather than a list written here, so this cannot pass by
    /// agreeing with a copy of itself. Each action is offered params for the *other* platform, which
    /// every planner refuses by name, so the assertion reads the routing rather than the build: an
    /// action sent to the wrong planner would report the wrong platform's params, and one that failed
    /// to dispatch at all would report an unknown action. Nothing here runs xcodebuild or Gradle.
    #[test]
    fn every_declared_action_routes_to_the_planner_for_its_platform() {
        let root = git_root("every_declared_action_routes_to_the_planner_for_its_platform");

        for action in Action::ALL.iter().copied() {
            let wants_ios = action.supports_repo_relative_derived_data_path();
            let mismatched = if wants_ios {
                NativeBuildParams::Android { gradlew: None }
            } else {
                NativeBuildParams::Ios {
                    workspace: "App.xcworkspace".to_string(),
                    scheme: "App".to_string(),
                    configuration: None,
                    sdk: None,
                    destination: None,
                    derived_data_path: None,
                }
            };

            let error = native_build_run(&root, None, action.as_str(), mismatched, Some(30), true)
                .expect_err("params for the other platform must be refused");
            let message = error.to_string();

            assert!(
                !message.contains("unknown action"),
                "{} must reach a planner: {message}",
                action.as_str()
            );
            let expected = if wants_ios {
                "requires iOS params"
            } else {
                "requires Android params"
            };
            assert!(
                message.contains(expected),
                "{} must route to the planner for its platform, expected {expected:?}: {message}",
                action.as_str()
            );
        }
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
