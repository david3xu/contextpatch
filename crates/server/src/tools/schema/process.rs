use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
                    "name": tools::run_guarded_command::NAME,
                    "description": "Run a repo-root-confined allowlisted validation command without using a shell.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "program": {
                                "type": "string",
                                "description": "Allowlisted executable name: git, cargo, bun, npm, pnpm, python/python3, pytest, harbor, bash for the exact base-image script, or rg."
                            },
                            "args": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "description": "Command arguments. The first argument must be an allowlisted subcommand."
                            },
                            "cwd": {
                                "type": "string",
                                "description": "Optional working directory relative to the configured repository root."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600,
                                "description": "Optional timeout in seconds. Defaults to 120, maximum 600."
                            }
                        },
                        "required": ["program", "args"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::read_command_log::NAME,
                    "description": "Read a previously captured guarded command log by log_id.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "log_id": {
                                "type": "string",
                                "description": "Opaque log id returned by run_guarded_command or validation_profile_run."
                            },
                            "max_chars": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 200000,
                                "description": "Optional maximum characters to return. Defaults to 12000."
                            }
                        },
                        "required": ["log_id"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::validation_profile_run::NAME,
                    "description": "Run a predefined sequence of allowlisted validation commands and return compact results with log ids.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "profile": {
                                "type": "string",
                                "description": "Validation profile name: repo-basic, rust-workspace, datacore-vscode, datacore-m6-vscode, or dynamo-harbor-task."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600,
                                "description": "Optional per-command timeout. Defaults to each profile command timeout."
                            },
                            "stop_on_failure": {
                                "type": "boolean",
                                "description": "Stop after the first non-zero or timed-out command. Defaults to true."
                            }
                        },
                        "required": ["profile"],
                        "additionalProperties": false
                    }
                }
        ),
    ]
}
