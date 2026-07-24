use std::path::Path;

use crate::error::ContextPatchError;
use crate::setup::plan::CommandPlan;
use crate::setup::profile::{
    require_no_params, validate_non_empty_single_line, validate_relative_path_param,
    CapacitorPlatform, SetupActionParams,
};

pub(crate) const PROFILE: &str = "node-capacitor-shell";

pub(crate) fn plan(
    cwd: &Path,
    action: &str,
    params: SetupActionParams,
) -> Result<CommandPlan, ContextPatchError> {
    let package_manager = detect_package_manager(cwd);
    match action {
        "install_capacitor_dependencies" => {
            require_no_params(action, params)?;
            Ok(CommandPlan::new(
                package_manager.program(),
                package_manager.add_args(&[
                    "@capacitor/core",
                    "@capacitor/cli",
                    "@capacitor/ios",
                    "@capacitor/android",
                ]),
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

    fn add_args(self, packages: &[&str]) -> Vec<String> {
        let command = match self {
            Self::Npm => "install",
            Self::Pnpm => "add",
        };
        std::iter::once(command.to_string())
            .chain(packages.iter().map(|package| package.to_string()))
            .collect()
    }

    fn cap_exec_args(self) -> Vec<String> {
        match self {
            Self::Npm => vec!["exec".to_string(), "--".to_string(), "cap".to_string()],
            Self::Pnpm => vec!["exec".to_string(), "cap".to_string()],
        }
    }
}

fn detect_package_manager(cwd: &Path) -> PackageManager {
    if cwd.join("pnpm-lock.yaml").is_file() {
        PackageManager::Pnpm
    } else {
        PackageManager::Npm
    }
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
