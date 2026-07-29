pub mod native_build_run {
    pub const NAME: &str = "native_build_run";
}

pub mod native_device_run {
    pub const NAME: &str = "native_device_run";
}

use std::path::Path;

use contextpatch_core::native_build::{native_build_run, NativeBuildParams};
use contextpatch_core::native_device::{native_device_run, NativeDeviceParams};
use serde_json::Value;

use crate::tools;
use crate::tools::common::{optional_bool, optional_string, optional_u64, required_string};

pub(crate) fn call_native_build_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let action = required_string(arguments, "action")?;
    let params = native_build_params(action, arguments.get("params"))?;
    let cwd = optional_string(arguments, "cwd")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);

    let result = native_build_run(
        repo_root,
        cwd.map(Path::new),
        action,
        params,
        timeout_secs,
        dry_run,
    )
    .map_err(|error| format!("native_build_run refused: {error}"))?;
    let summary = result.summary();
    let log_id = tools::process::write_command_log(&summary)
        .map_err(|error| format!("native_build_run log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{summary}"))
}

fn native_build_params(action: &str, value: Option<&Value>) -> Result<NativeBuildParams, String> {
    let params = value
        .and_then(Value::as_object)
        .ok_or_else(|| "missing or invalid object argument: params".to_string())?;
    match action {
        "ios_build" | "ios_test" => Ok(NativeBuildParams::Ios {
            workspace: required_string(params, "workspace")?.to_string(),
            scheme: required_string(params, "scheme")?.to_string(),
            configuration: optional_string(params, "configuration")?.map(ToString::to_string),
            sdk: optional_string(params, "sdk")?.map(ToString::to_string),
            destination: optional_string(params, "destination")?.map(ToString::to_string),
            derived_data_path: optional_string(params, "derived_data_path")?
                .map(ToString::to_string),
        }),
        "android_assemble_debug" | "android_unit_test" => Ok(NativeBuildParams::Android {
            gradlew: optional_string(params, "gradlew")?.map(ToString::to_string),
        }),
        _ => Ok(NativeBuildParams::Android { gradlew: None }),
    }
}

pub(crate) fn call_native_device_run(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let action = required_string(arguments, "action")?;
    let params = native_device_params(action, arguments.get("params"))?;
    let cwd = optional_string(arguments, "cwd")?;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?;
    let dry_run = optional_bool(arguments, "dry_run")?.unwrap_or(true);
    let confirm = optional_string(arguments, "confirm")?;

    let result = native_device_run(
        repo_root,
        cwd.map(Path::new),
        action,
        params,
        timeout_secs,
        dry_run,
        confirm,
    )
    .map_err(|error| format!("native_device_run refused: {error}"))?;
    let summary = result.summary();
    let log_id = tools::process::write_command_log(&summary)
        .map_err(|error| format!("native_device_run log write failed: {error}"))?;
    Ok(format!("log_id: {log_id}\n{summary}"))
}

fn native_device_params(action: &str, value: Option<&Value>) -> Result<NativeDeviceParams, String> {
    let params = match value {
        Some(value) => value
            .as_object()
            .ok_or_else(|| "invalid object argument: params".to_string())?,
        None => {
            return match action {
                "ios_list_simulators" | "android_list_devices" => Ok(NativeDeviceParams::None),
                _ => Err("missing object argument: params".to_string()),
            };
        }
    };
    match action {
        "ios_list_simulators" => {
            if params.is_empty() {
                Ok(NativeDeviceParams::None)
            } else {
                Err(
                    "native_device_run refused: ios_list_simulators does not accept params"
                        .to_string(),
                )
            }
        }
        "ios_boot_simulator" => Ok(NativeDeviceParams::IosDevice {
            device: required_string(params, "device")?.to_string(),
        }),
        "ios_read_logs" => Ok(NativeDeviceParams::IosLogs {
            device: required_string(params, "device")?.to_string(),
            duration: optional_string(params, "duration")?
                .or(optional_string(params, "last")?)
                .map(ToString::to_string),
        }),
        "ios_create_simulator" => Ok(NativeDeviceParams::IosCreate {
            name: required_string(params, "name")?.to_string(),
            device_type: required_string(params, "device_type")?.to_string(),
            runtime: optional_string(params, "runtime")?.map(ToString::to_string),
        }),
        "ios_install_app" => Ok(NativeDeviceParams::IosInstall {
            device: required_string(params, "device")?.to_string(),
            app_path: required_string(params, "app_path")?.to_string(),
        }),
        "ios_launch_app" => Ok(NativeDeviceParams::IosLaunch {
            device: required_string(params, "device")?.to_string(),
            app_id: required_string(params, "app_id")?.to_string(),
        }),
        "ios_cap_run" => Ok(NativeDeviceParams::IosCapRun {
            target: required_string(params, "target")?.to_string(),
        }),
        "android_list_devices" => Ok(NativeDeviceParams::AndroidSerial {
            serial: optional_string(params, "serial")?.map(ToString::to_string),
        }),
        "android_install_app" => Ok(NativeDeviceParams::AndroidInstall {
            serial: optional_string(params, "serial")?.map(ToString::to_string),
            apk_path: required_string(params, "apk_path")?.to_string(),
        }),
        "android_launch_app" => Ok(NativeDeviceParams::AndroidLaunch {
            serial: optional_string(params, "serial")?.map(ToString::to_string),
            app_id: required_string(params, "app_id")?.to_string(),
        }),
        "android_read_logcat" => {
            let lines = optional_u64(params, "lines")?
                .map(|value| {
                    u32::try_from(value)
                        .map_err(|_| "native_device_run refused: lines is too large".to_string())
                })
                .transpose()?;
            Ok(NativeDeviceParams::AndroidLogcat {
                serial: optional_string(params, "serial")?.map(ToString::to_string),
                lines,
            })
        }
        _ => Ok(NativeDeviceParams::None),
    }
}
