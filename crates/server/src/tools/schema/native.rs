use serde_json::{json, Value};

use crate::tools;

pub(crate) fn native_build_run_definition() -> Value {
    json!({
                "name": tools::native_build_run::NAME,
                "description": "Plan or run a typed native build/test action without exposing raw xcodebuild or Gradle commands.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            // Derived, not restated: the members are the enum beside this field, so
                            // the description says what the field selects instead of listing them.
                            "enum": contextpatch_core::native_build::Action::advertised_names(),
                            "description": "Which native build to plan or run."
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
    )
}

pub(crate) fn native_device_run_definition() -> Value {
    json!({
                "name": tools::native_device_run::NAME,
                "description": "Plan or run bounded typed native simulator/emulator/device smoke actions without arbitrary xcrun or adb access.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": contextpatch_core::native_device::DeviceAction::advertised_names(),
                            "description": "Which device operation to plan or run."
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
    )
}
