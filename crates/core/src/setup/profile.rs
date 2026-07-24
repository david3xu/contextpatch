use std::path::Path;

use crate::error::ContextPatchError;
use crate::git::status::{dirty_paths, status_short};
use crate::process::runner::{
    checked_timeout, resolve_cwd, run_no_shell_command, validate_common_command_shape,
};
use crate::setup::node_capacitor;
use crate::setup::plan::{SetupExecution, SetupProfileResult};

pub const SETUP_MUTATION_CONFIRMATION: &str = "run setup profile";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupActionParams {
    None,
    CapInit {
        app_id: String,
        app_name: String,
        web_dir: String,
    },
    CapSync {
        platform: Option<CapacitorPlatform>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacitorPlatform {
    Ios,
    Android,
    All,
}

impl CapacitorPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ios => "ios",
            Self::Android => "android",
            Self::All => "all",
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn setup_profile_run(
    repo_root: &Path,
    cwd: Option<&Path>,
    profile: &str,
    action: &str,
    params: SetupActionParams,
    timeout_secs: Option<u64>,
    dry_run: bool,
    confirm: Option<&str>,
) -> Result<SetupProfileResult, ContextPatchError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ContextPatchError::new(format!(
            "failed to resolve repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    let cwd = resolve_cwd(&root, cwd)?;
    let timeout = checked_timeout(timeout_secs)?;

    if !dry_run && confirm != Some(SETUP_MUTATION_CONFIRMATION) {
        return Err(ContextPatchError::new(format!(
            "setup_profile_run refused: dry_run=false requires confirm: {SETUP_MUTATION_CONFIRMATION:?}"
        )));
    }

    let plan = match profile {
        node_capacitor::PROFILE => node_capacitor::plan(action, params)?,
        _ => {
            return Err(ContextPatchError::new(format!(
                "setup_profile_run refused: unknown profile `{profile}`"
            )));
        }
    };
    validate_common_command_shape(&plan.program, &plan.args)?;

    let execution = if dry_run {
        None
    } else {
        let before_paths = dirty_paths(&root)?;
        if !before_paths.is_empty() {
            return Err(ContextPatchError::new(format!(
                "setup_profile_run refused: worktree must be clean before external setup mutation\n{}",
                before_paths.into_iter().collect::<Vec<_>>().join("\n")
            )));
        }
        let status_before = status_short(&root)?;
        let output = run_no_shell_command(
            &cwd,
            &plan.program,
            &plan.args,
            timeout,
            "setup_profile_run",
        )?;
        let status_after = status_short(&root)?;
        let changed_paths = dirty_paths(&root)?.into_iter().collect::<Vec<_>>();
        let unexpected =
            unexpected_changed_paths(&changed_paths, &plan.expected_changed_path_classes);
        if !unexpected.is_empty() {
            return Err(ContextPatchError::new(format!(
                "setup_profile_run refused after external mutation: changed paths outside expected classes\nunexpected_paths:\n{}\nexpected_changed_path_classes:\n{}",
                unexpected.join("\n"),
                plan.expected_changed_path_classes.join("\n")
            )));
        }
        if output.timed_out || output.exit_code != 0 {
            return Err(ContextPatchError::new(format!(
                "setup_profile_run external command failed after execution\nexit_code: {}\ntimed_out: {}\nchanged_paths:\n{}\nstdout:\n{}\nstderr:\n{}",
                output.exit_code,
                output.timed_out,
                empty_lines(&changed_paths),
                empty_label(&output.stdout),
                empty_label(&output.stderr)
            )));
        }
        Some(SetupExecution {
            exit_code: output.exit_code,
            timed_out: output.timed_out,
            duration_ms: output.duration_ms,
            stdout: output.stdout,
            stderr: output.stderr,
            status_before,
            status_after,
            changed_paths,
        })
    };

    Ok(SetupProfileResult {
        profile: profile.to_string(),
        action: action.to_string(),
        dry_run,
        cwd,
        plan,
        execution,
        required_confirm_for_mutation: SETUP_MUTATION_CONFIRMATION,
    })
}

fn unexpected_changed_paths(paths: &[String], expected_classes: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|path| {
            let class = changed_path_class(path);
            !expected_classes.iter().any(|expected| expected == class)
        })
        .cloned()
        .collect()
}

fn changed_path_class(path: &str) -> &'static str {
    match path {
        "package.json" => "package_manifest",
        "package-lock.json" | "npm-shrinkwrap.json" => "package_lock",
        "capacitor.config.ts" | "capacitor.config.js" | "capacitor.config.json" => {
            "capacitor_config"
        }
        _ if path.starts_with("node_modules/") => "node_modules",
        _ if path.starts_with("ios/") || path == "ios" => "ios_project",
        _ if path.starts_with("android/") || path == "android" => "android_project",
        _ => "unexpected",
    }
}

fn empty_lines(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values.join("\n")
    }
}

fn empty_label(value: &str) -> &str {
    if value.trim().is_empty() {
        "(empty)"
    } else {
        value
    }
}

pub(crate) fn validate_non_empty_single_line(
    tool: &str,
    field: &str,
    value: &str,
    max_len: usize,
) -> Result<(), ContextPatchError> {
    if value.trim().is_empty() {
        return Err(ContextPatchError::new(format!(
            "{tool} refused: {field} must not be empty"
        )));
    }
    if value.len() > max_len || value.contains('\0') || value.contains('\n') || value.contains('\r')
    {
        return Err(ContextPatchError::new(format!(
            "{tool} refused: {field} is invalid"
        )));
    }
    Ok(())
}

pub(crate) fn validate_relative_path_param(
    tool: &str,
    field: &str,
    value: &str,
) -> Result<(), ContextPatchError> {
    validate_non_empty_single_line(tool, field, value, 512)?;
    if value == ".."
        || value.starts_with("../")
        || value.contains("/../")
        || value.starts_with('/')
        || value.contains('\\')
    {
        return Err(ContextPatchError::new(format!(
            "{tool} refused: {field} must be a repository-relative path"
        )));
    }
    Ok(())
}

pub(crate) fn require_no_params(
    action: &str,
    params: SetupActionParams,
) -> Result<(), ContextPatchError> {
    if params == SetupActionParams::None {
        Ok(())
    } else {
        Err(ContextPatchError::new(format!(
            "setup_profile_run refused: action `{action}` does not accept params"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        setup_profile_run, unexpected_changed_paths, CapacitorPlatform, SetupActionParams,
    };

    #[test]
    fn plans_capacitor_dependency_install_without_caller_package_list() {
        let root = git_root("plans_capacitor_dependency_install_without_caller_package_list");

        let result = setup_profile_run(
            &root,
            None,
            "node-capacitor-shell",
            "install_capacitor_dependencies",
            SetupActionParams::None,
            Some(30),
            true,
            None,
        )
        .unwrap();

        assert_eq!(result.plan.program, "npm");
        assert_eq!(
            result.plan.args,
            [
                "install",
                "@capacitor/core",
                "@capacitor/cli",
                "@capacitor/ios",
                "@capacitor/android"
            ]
        );
        assert!(result.summary().contains("external_mutator: true"));
    }

    #[test]
    fn plans_cap_init_from_typed_params() {
        let root = git_root("plans_cap_init_from_typed_params");

        let result = setup_profile_run(
            &root,
            None,
            "node-capacitor-shell",
            "cap_init",
            SetupActionParams::CapInit {
                app_id: "com.example.app".to_string(),
                app_name: "Example".to_string(),
                web_dir: "dist".to_string(),
            },
            Some(30),
            true,
            None,
        )
        .unwrap();

        assert_eq!(
            result.plan.args,
            [
                "exec",
                "--",
                "cap",
                "init",
                "Example",
                "com.example.app",
                "--web-dir",
                "dist"
            ]
        );
    }

    #[test]
    fn plans_cap_sync_platform_enum() {
        let root = git_root("plans_cap_sync_platform_enum");

        let result = setup_profile_run(
            &root,
            None,
            "node-capacitor-shell",
            "cap_sync",
            SetupActionParams::CapSync {
                platform: Some(CapacitorPlatform::Ios),
            },
            Some(30),
            true,
            None,
        )
        .unwrap();

        assert_eq!(result.plan.args, ["exec", "--", "cap", "sync", "ios"]);
    }

    #[test]
    fn plans_ios_pod_install_as_setup_mutator() {
        let root = git_root("plans_ios_pod_install_as_setup_mutator");

        let result = setup_profile_run(
            &root,
            None,
            "node-capacitor-shell",
            "ios_pod_install",
            SetupActionParams::None,
            Some(30),
            true,
            None,
        )
        .unwrap();

        assert_eq!(result.plan.program, "pod");
        assert_eq!(result.plan.args, ["install"]);
        assert_eq!(result.plan.expected_changed_path_classes, ["ios_project"]);
    }

    #[test]
    fn refuses_unknown_profile_and_action() {
        let root = git_root("refuses_unknown_profile_and_action");

        let profile_error = setup_profile_run(
            &root,
            None,
            "unknown",
            "cap_sync",
            SetupActionParams::CapSync { platform: None },
            Some(30),
            true,
            None,
        )
        .unwrap_err();
        assert!(profile_error.to_string().contains("unknown profile"));

        let action_error = setup_profile_run(
            &root,
            None,
            "node-capacitor-shell",
            "unknown",
            SetupActionParams::None,
            Some(30),
            true,
            None,
        )
        .unwrap_err();
        assert!(action_error.to_string().contains("unknown action"));
    }

    #[test]
    fn refuses_invalid_params_and_missing_mutation_confirm() {
        let root = git_root("refuses_invalid_params_and_missing_mutation_confirm");

        let invalid = setup_profile_run(
            &root,
            None,
            "node-capacitor-shell",
            "cap_init",
            SetupActionParams::CapInit {
                app_id: "com.example.app".to_string(),
                app_name: "Example".to_string(),
                web_dir: "../dist".to_string(),
            },
            Some(30),
            true,
            None,
        )
        .unwrap_err();
        assert!(invalid.to_string().contains("repository-relative path"));

        let missing_confirm = setup_profile_run(
            &root,
            None,
            "node-capacitor-shell",
            "cap_sync",
            SetupActionParams::CapSync { platform: None },
            Some(30),
            false,
            None,
        )
        .unwrap_err();
        assert!(missing_confirm.to_string().contains("requires confirm"));
    }

    #[test]
    fn refuses_mutation_when_worktree_is_dirty_before_external_command() {
        let root = git_root("refuses_mutation_when_worktree_is_dirty_before_external_command");
        fs::write(root.join("package.json"), "{}\n").unwrap();

        let error = setup_profile_run(
            &root,
            None,
            "node-capacitor-shell",
            "cap_sync",
            SetupActionParams::CapSync { platform: None },
            Some(30),
            false,
            Some("run setup profile"),
        )
        .unwrap_err();

        assert!(error.to_string().contains("worktree must be clean"));
        assert!(error.to_string().contains("package.json"));
    }

    #[test]
    fn classifies_unexpected_setup_changed_paths() {
        let changed_paths = vec![
            "ios/App/App.xcodeproj/project.pbxproj".to_string(),
            "package-lock.json".to_string(),
            "src/main.ts".to_string(),
        ];
        let expected = vec!["ios_project".to_string(), "package_lock".to_string()];

        assert_eq!(
            unexpected_changed_paths(&changed_paths, &expected),
            ["src/main.ts".to_string()]
        );
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
