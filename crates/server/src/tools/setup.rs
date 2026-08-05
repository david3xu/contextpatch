pub mod setup_profile_run {
    pub const NAME: &str = "setup_profile_run";
}

use std::path::Path;

use contextpatch_core::setup::profile::{setup_profile_run, CapacitorPlatform, SetupActionParams};
use serde_json::Value;

use crate::tools;
use crate::tools::common::{optional_bool, optional_string, optional_u64, required_string};

pub(crate) fn call_setup_profile_run<'a>(
    repository_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let profile = required_string(arguments, "profile")?;
    let action = required_string(arguments, "action")?;
    let params = setup_action_params(action, arguments.get("params"))?;
    let cwd = optional_string(arguments, "cwd")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    let result = setup_profile_run(
        repository_root,
        cwd.map(Path::new),
        profile,
        action,
        params,
        timeout_secs,
        dry_run,
        confirm,
    )
    .map_err(|error| format!("setup_profile_run refused: {error}"))?;
    let summary = result.summary();
    let log_id = tools::process::write_command_log(&summary)
        .map_err(|error| format!("setup_profile_run log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{summary}"))
}

fn setup_action_params(action: &str, value: Option<&Value>) -> Result<SetupActionParams, String> {
    let params = match value {
        Some(value) => value
            .as_object()
            .ok_or_else(|| "invalid object argument: params".to_string())?,
        None => {
            return match action {
                "cap_sync" => Ok(SetupActionParams::CapSync { platform: None }),
                _ => Ok(SetupActionParams::None),
            };
        }
    };

    match action {
        "cap_init" => Ok(SetupActionParams::CapInit {
            app_id: required_string(params, "app_id")?.to_string(),
            app_name: required_string(params, "app_name")?.to_string(),
            web_dir: required_string(params, "web_dir")?.to_string(),
        }),
        "cap_sync" => {
            let platform = optional_string(params, "platform")?
                .map(parse_capacitor_platform)
                .transpose()?;
            Ok(SetupActionParams::CapSync { platform })
        }
        "install_capacitor_dependencies"
        | "install_capacitor_filesystem"
        | "cap_add_ios"
        | "cap_add_android"
        | "ios_pod_install"
            if params.is_empty() =>
        {
            Ok(SetupActionParams::None)
        }
        "install_capacitor_dependencies"
        | "install_capacitor_filesystem"
        | "cap_add_ios"
        | "cap_add_android"
        | "ios_pod_install" => Err(format!(
            "setup_profile_run refused: action `{action}` does not accept params"
        )),
        _ => Ok(SetupActionParams::None),
    }
}

fn parse_capacitor_platform(value: &str) -> Result<CapacitorPlatform, String> {
    match value {
        "ios" => Ok(CapacitorPlatform::Ios),
        "android" => Ok(CapacitorPlatform::Android),
        "all" => Ok(CapacitorPlatform::All),
        _ => Err(format!(
            "setup_profile_run refused: unsupported cap_sync platform `{value}`"
        )),
    }
}
