use crate::error::ContextPatchError;
use crate::git::RepositoryRoot;
use crate::setup::plan::{CommandPlan, PlannedCommand};
use crate::setup::profile::{
    require_no_params, validate_non_empty_single_line, validate_relative_path_param,
    CapacitorPlatform, SetupActionParams,
};

pub(crate) const PROFILE: &str = "node-capacitor-shell";

const PNPM_LOCKFILE: &str = "pnpm-lock.yaml";
const PODFILE: &str = "Podfile";

/// Plan one setup action for a project rooted at `cwd_relative` inside `root`.
///
/// The project is inspected through the repository's own authority, named relative to the root rather than
/// by joining the working directory's path. A lockfile or Podfile therefore decides the plan only when it is
/// a regular file inside the repository that was selected: a symlink is not followed, and a sibling
/// repository with the same layout cannot answer for this one.
pub(crate) fn plan(
    root: RepositoryRoot<'_>,
    cwd_relative: &str,
    action: &str,
    params: SetupActionParams,
) -> Result<CommandPlan, ContextPatchError> {
    let package_manager = detect_package_manager(root, cwd_relative)?;
    match action {
        "install_capacitor_dependencies" => {
            require_no_params(action, params)?;
            Ok(CommandPlan::sequence(
                package_manager.install_capacitor_dependency_commands(),
                vec![
                    "package_manifest".to_string(),
                    "package_lock".to_string(),
                    "node_modules".to_string(),
                ],
            ))
        }
        "install_capacitor_filesystem" => {
            require_no_params(action, params)?;
            Ok(CommandPlan::new(
                package_manager.program(),
                package_manager.add_args(&["@capacitor/filesystem"], false),
                vec![
                    "package_manifest".to_string(),
                    "package_lock".to_string(),
                    "node_modules".to_string(),
                ],
            ))
        }
        "cap_init" => {
            let SetupActionParams::CapInit {
                app_id,
                app_name,
                web_dir,
            } = params
            else {
                return Err(ContextPatchError::new(
                    "setup_profile_run refused: cap_init requires app_id, app_name, and web_dir",
                ));
            };
            validate_app_id(&app_id)?;
            validate_non_empty_single_line("setup_profile_run", "app_name", &app_name, 120)?;
            validate_relative_path_param("setup_profile_run", "web_dir", &web_dir)?;
            let mut args = package_manager.cap_exec_args();
            args.extend([
                "init".to_string(),
                app_name,
                app_id,
                "--web-dir".to_string(),
                web_dir,
            ]);
            Ok(CommandPlan::new(
                package_manager.program(),
                args,
                vec!["capacitor_config".to_string()],
            ))
        }
        "cap_add_ios" => {
            require_no_params(action, params)?;
            Ok(cap_platform_plan(
                package_manager,
                "add",
                CapacitorPlatform::Ios,
                vec!["ios_project"],
            ))
        }
        "cap_add_android" => {
            require_no_params(action, params)?;
            Ok(cap_platform_plan(
                package_manager,
                "add",
                CapacitorPlatform::Android,
                vec!["android_project"],
            ))
        }
        "cap_sync" => {
            let SetupActionParams::CapSync { platform } = params else {
                return Err(ContextPatchError::new(
                    "setup_profile_run refused: cap_sync requires optional platform params",
                ));
            };
            let mut args = package_manager.cap_exec_args();
            args.push("sync".to_string());
            let expected = match platform {
                Some(CapacitorPlatform::Ios) => {
                    args.push("ios".to_string());
                    vec!["ios_project".to_string()]
                }
                Some(CapacitorPlatform::Android) => {
                    args.push("android".to_string());
                    vec!["android_project".to_string()]
                }
                Some(CapacitorPlatform::All) | None => {
                    vec!["ios_project".to_string(), "android_project".to_string()]
                }
            };
            Ok(CommandPlan::new(package_manager.program(), args, expected))
        }
        "ios_pod_install" => {
            require_no_params(action, params)?;
            if !crate::fs::rooted::is_regular_file(root, &project_relative(cwd_relative, PODFILE))? {
                return Err(ContextPatchError::new(
                    "setup_profile_run refused: ios_pod_install requires a Podfile in cwd; Swift Package Manager based Capacitor projects do not need CocoaPods",
                ));
            }
            Ok(CommandPlan::new(
                "pod",
                vec!["install".to_string()],
                vec!["ios_project".to_string()],
            ))
        }
        _ => Err(ContextPatchError::new(format!(
            "setup_profile_run refused: unknown action `{action}` for profile `{PROFILE}`"
        ))),
    }
}

fn cap_platform_plan(
    package_manager: PackageManager,
    cap_action: &str,
    platform: CapacitorPlatform,
    expected_changed_path_classes: Vec<&str>,
) -> CommandPlan {
    let mut args = package_manager.cap_exec_args();
    args.extend([cap_action.to_string(), platform.as_str().to_string()]);
    CommandPlan::new(
        package_manager.program(),
        args,
        expected_changed_path_classes
            .into_iter()
            .map(ToString::to_string)
            .collect(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageManager {
    Npm,
    Pnpm,
}

impl PackageManager {
    fn program(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
        }
    }

    fn add_args(self, packages: &[&str], save_dev: bool) -> Vec<String> {
        let command = match self {
            Self::Npm => "install",
            Self::Pnpm => "add",
        };
        let mut args = vec![command.to_string()];
        if save_dev {
            args.push("--save-dev".to_string());
        }
        args.extend(packages.iter().map(|package| package.to_string()));
        args
    }

    fn cap_exec_args(self) -> Vec<String> {
        match self {
            Self::Npm => vec!["exec".to_string(), "--".to_string(), "cap".to_string()],
            Self::Pnpm => vec!["exec".to_string(), "cap".to_string()],
        }
    }

    fn install_capacitor_dependency_commands(self) -> Vec<PlannedCommand> {
        let runtime_packages = ["@capacitor/core", "@capacitor/ios", "@capacitor/android"];
        let dev_packages = ["@capacitor/cli"];
        vec![
            PlannedCommand::new(self.program(), self.add_args(&runtime_packages, false)),
            PlannedCommand::new(self.program(), self.add_args(&dev_packages, true)),
        ]
    }
}

fn detect_package_manager(
    root: RepositoryRoot<'_>,
    cwd_relative: &str,
) -> Result<PackageManager, ContextPatchError> {
    if crate::fs::rooted::is_regular_file(root, &project_relative(cwd_relative, PNPM_LOCKFILE))? {
        Ok(PackageManager::Pnpm)
    } else {
        Ok(PackageManager::Npm)
    }
}

/// Name one project file relative to the repository root.
///
/// The working directory is already root relative, so this is string composition rather than path
/// resolution: nothing here reaches the filesystem.
fn project_relative(cwd_relative: &str, file_name: &str) -> String {
    if cwd_relative.is_empty() {
        return file_name.to_string();
    }
    format!("{cwd_relative}/{file_name}")
}

fn validate_app_id(app_id: &str) -> Result<(), ContextPatchError> {
    validate_non_empty_single_line("setup_profile_run", "app_id", app_id, 200)?;
    if !app_id.contains('.')
        || !app_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(ContextPatchError::new(
            "setup_profile_run refused: app_id must be a reverse-DNS style identifier",
        ));
    }
    Ok(())
}
