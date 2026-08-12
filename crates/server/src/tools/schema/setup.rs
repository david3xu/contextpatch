use serde_json::{json, Value};

use crate::tools;

pub(crate) fn setup_profile_run_definition() -> Value {
    json!({
                "name": tools::setup_profile_run::NAME,
                "description": "Plan or run a predefined repository setup action from a typed profile. Callers do not provide raw commands.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "profile": {
                            "type": "string",
                            "description": "Setup profile name. Supported: node-capacitor-shell."
                        },
                        "action": {
                            "type": "string",
                            // Derived, not restated: the members are the enum beside this field, so
                            // the description says what the field selects instead of listing them.
                            "enum": contextpatch_core::setup::advertised_action_names(),
                            "description": "Which step of the selected profile to plan or run."
                        },
                        "params": {
                            "type": "object",
                            "description": "Typed action parameters. cap_init requires app_id, app_name, and web_dir; cap_sync accepts platform: ios, android, or all."
                        },
                        "cwd": {
                            "type": "string",
                            "description": "Optional working directory relative to the configured repository root."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 600,
                            "description": "Optional timeout for the planned command. Defaults to 120, maximum 600."
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "Plan only without mutating. Defaults to true."
                        },
                        "confirm": {
                            "type": "string",
                            "description": "Required mutation confirmation literal `run setup profile` when dry_run is false."
                        }
                    },
                    "required": ["profile", "action"],
                    "additionalProperties": false
                }
            }
    )
}
