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
                                "description": "Allowlisted executable name: git, cargo, bun, npm, pnpm, python/python3, pytest, bash for the exact base-image script, or rg. Use harbor_run_start for Harbor."
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
                                "description": "Optional timeout in seconds. Defaults to 120 and remains capped at 600. Harbor uses its typed asynchronous action."
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
                                "description": "Opaque log id returned by guarded commands or asynchronous Harbor, task-image, and validation-profile actions."
                            },
                            "max_chars": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 200000,
                                "description": "Optional maximum characters to return. Defaults to 12000."
                            },
                            "offset": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "Optional character offset for paging long logs. Defaults to 0."
                            }
                        },
                        "required": ["log_id"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::artifact_python_run::NAME,
                    "description": "Run a Python script previously written under the fixed artifact root, without a shell and without placing scratch code in the repository.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "script": {
                                "type": "string",
                                "description": "Artifact-root-relative Python script path."
                            },
                            "program": {
                                "type": "string",
                                "enum": ["python3", "python"],
                                "description": "Python executable name. Defaults to python3."
                            },
                            "args": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "description": "Optional script arguments, passed without a shell."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600,
                                "description": "Optional timeout in seconds. Defaults to 120."
                            }
                        },
                        "required": ["script"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::task_image_python_run::NAME,
                    "description": "Plan a hardened task-image Python run. With dry_run=false, start the build and execution in the background, return a log_id immediately, and poll with read_command_log.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "script": {
                                "type": "string",
                                "description": "Existing normalized repository-relative .py file; symlinks are refused."
                            },
                            "program": {
                                "type": "string",
                                "enum": ["python3", "python"],
                                "description": "Python executable inside the image. Defaults to python3."
                            },
                            "args": {
                                "type": "array",
                                "items": {"type": "string", "maxLength": 4096},
                                "maxItems": 32,
                                "description": "Arguments passed to the script without a shell."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600,
                                "description": "Script timeout in seconds. Defaults to 120."
                            },
                            "build_timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 1800,
                                "description": "Image build timeout in seconds. Defaults to 600."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Return the exact build and run plan without invoking Docker. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Execution requires the exact phrase: run task image python"
                            }
                        },
                        "required": ["script"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::harbor_run_start::NAME,
                    "description": "Start one typed Harbor run in the background and return a log_id immediately. Poll with read_command_log; completed logs include structured Harbor evidence. Harbor, task-image, and validation-profile jobs share a two-job cap.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project": {
                                "type": "string",
                                "description": "Normalized repository-relative Harbor project directory. Defaults to task."
                            },
                            "agent": {
                                "type": "string",
                                "minLength": 1,
                                "maxLength": 128,
                                "pattern": "^[A-Za-z0-9._][A-Za-z0-9._-]*$",
                                "description": "Harbor agent identifier. A leading hyphen is refused."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 3600,
                                "description": "Run timeout in seconds. Defaults to 3600."
                            }
                        },
                        "required": ["agent"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::image_cleanliness_check_run::NAME,
                    "description": "Plan or run a narrow Docker image cleanliness check for a simple file name without exposing generic docker.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "image": {
                                "type": "string",
                                "description": "Docker image reference to inspect."
                            },
                            "filename": {
                                "type": "string",
                                "description": "Simple file name to search for with find. Defaults to solve.sh."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview without running Docker. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `run image cleanliness check` when dry_run is false."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600,
                                "description": "Optional timeout in seconds. Defaults to 120."
                            }
                        },
                        "required": ["image"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::docker_image_inspect::NAME,
                    "description": "Plan or run `docker image inspect` for one validated image reference without exposing generic Docker.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "image": {
                                "type": "string",
                                "description": "Docker image reference to inspect."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview without running Docker. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `inspect docker image` when dry_run is false."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600,
                                "description": "Optional timeout in seconds. Defaults to 120."
                            }
                        },
                        "required": ["image"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::validation_profile_run::NAME,
                    "description": "Start a predefined validation profile in the background and return a log_id immediately. Poll with read_command_log; polling never restarts work.",
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
                                "description": "Optional per-command timeout override up to 600. Defaults to each profile command timeout; Dynamo Harbor commands default to 3600."
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
