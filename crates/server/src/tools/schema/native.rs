use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
                    "name": tools::native_build_run::NAME,
                    "description": "Plan or run a typed native build/test action without exposing raw xcodebuild or Gradle commands.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "description": "Native build action: ios_build, ios_test, android_assemble_debug, or android_unit_test."
                            },
                            "params": {
                                "type": "object",
                                "description": "Typed action parameters. iOS uses workspace, scheme, optional configuration/sdk/destination/derived_data_path. Android accepts optional gradlew path."
                            },
                            "cwd": {
                                "type": "string",
                                "description": "Optional working directory relative to the configured repository root."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Plan only without running the build. Defaults to true."
                            }
                        },
                        "required": ["action", "params"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::native_device_run::NAME,
                    "description": "Plan or run bounded typed native simulator/emulator/device smoke actions without arbitrary xcrun or adb access.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "description": "Native device action such as ios_list_simulators, ios_create_simulator, ios_boot_simulator, ios_cap_run, ios_install_app, ios_launch_app, ios_read_logs, android_list_devices, android_install_app, android_launch_app, or android_read_logcat."
                            },
                            "params": {
                                "type": "object",
                                "description": "Typed action parameters such as device, serial, app_id, app_path, apk_path, Android lines, or iOS log duration."
                            },
                            "cwd": {
                                "type": "string",
                                "description": "Optional working directory relative to the configured repository root."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Plan only without touching simulator/device state. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal `run native device` when dry_run is false for device-state changes."
                            }
                        },
                        "required": ["action"],
                        "additionalProperties": false
                    }
                }
        ),
    ]
}
