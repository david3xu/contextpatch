use crate::error::ContextPatchError;
use crate::setup::plan::CommandPlan;
use crate::setup::profile::{
    require_no_params, validate_non_empty_single_line, validate_relative_path_param,
    CapacitorPlatform, SetupActionParams,
};

pub(crate) const PROFILE: &str = "node-capacitor-shell";

pub(crate) fn plan(
    action: &str,
    params: SetupActionParams,
) -> Result<CommandPlan, ContextPatchError> {
    match action {
        "install_capacitor_dependencies" => {
            require_no_params(action, params)?;
            Ok(CommandPlan::new(
                "npm",
                vec![
                    "install".to_string(),
                    "@capacitor/core".to_string(),
                    "@capacitor/cli".to_string(),
                    "@capacitor/ios".to_string(),
                    "@capacitor/android".to_string(),
                ],
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
            Ok(CommandPlan::new(
                "npm",
                vec![
                    "exec".to_string(),
                    "--".to_string(),
                    "cap".to_string(),
                    "init".to_string(),
                    app_name,
                    app_id,
                    "--web-dir".to_string(),
                    web_dir,
                ],
                vec!["capacitor_config".to_string()],
            ))
        }
        "cap_add_ios" => {
            require_no_params(action, params)?;
            Ok(cap_platform_plan(
                "add",
                CapacitorPlatform::Ios,
                vec!["ios_project"],
            ))
        }
        "cap_add_android" => {
            require_no_params(action, params)?;
            Ok(cap_platform_plan(
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
            let mut args = vec![
                "exec".to_string(),
                "--".to_string(),
                "cap".to_string(),
                "sync".to_string(),
            ];
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
            Ok(CommandPlan::new("npm", args, expected))
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
    cap_action: &str,
    platform: CapacitorPlatform,
    expected_changed_path_classes: Vec<&str>,
) -> CommandPlan {
    CommandPlan::new(
        "npm",
        vec![
            "exec".to_string(),
            "--".to_string(),
            "cap".to_string(),
            cap_action.to_string(),
            platform.as_str().to_string(),
        ],
        expected_changed_path_classes
            .into_iter()
            .map(ToString::to_string)
            .collect(),
    )
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
