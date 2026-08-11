use serde_json::{json, Value};

use crate::tools;

pub(crate) fn run_guarded_command_definition() -> Value {
    json!({
                "name": tools::run_guarded_command::NAME,
                "description": "Run an allowlisted validation command with repo-root-confined arguments and no shell interposed by this server. That narrows the entry point and the arguments; it is not a sandbox. The child inherits this server's environment, and reviewed repository code it runs (cargo build scripts and tests, npm-family scripts, Python, pytest collection) can read files, start subprocesses, and use the network with the server user's permissions. npm-family scripts may invoke their own shell. Container isolation with networking disabled belongs to the tools classified for isolated execution, which this is not.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "program": {
                            "type": "string",
                            "description": "Allowlisted executable name: git, cargo, bun, npm, pnpm, python/python3, pytest, bash for a script on the fixed validation-script list, or rg. Use harbor_run_start for Harbor."
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
    )
}

pub(crate) fn read_command_log_definition() -> Value {
    json!({
                "name": tools::read_command_log::NAME,
                "description": "Read a previously captured guarded command log by log_id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "log_id": {
                            "type": "string",
                            "description": "Opaque log id returned by guarded commands or asynchronous Harbor, task-image, validation-profile, Compose-stack, and artifact-build actions."
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
    )
}

pub(crate) fn artifact_python_run_definition() -> Value {
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
    )
}

pub(crate) fn task_image_python_run_definition() -> Value {
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
    )
}

pub(crate) fn artifact_build_check_run_definition() -> Value {
    json!({
                "name": tools::artifact_build_check_run::NAME,
                "description": "Plan or start one artifact packaging gate: docker build of a repository Dockerfile, then an import smoke run of the image that was built. Catches packaging failures and dead exports that reading source cannot. The caller names the Dockerfile, build context, and the arguments passed to the built image; this server derives every Docker flag, the unique image tag, and the cleanup. Smoke arguments are placed after the image name, so they are the container command and can never be reinterpreted as Docker options. The smoke run is pinned to --network none so a dead export cannot be masked by a successful download; the build itself has the network. With dry_run=false, run in the background and return a log_id to poll with read_command_log. The built image is always removed afterwards. Shares the two-job background cap.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "dockerfile": {
                            "type": "string",
                            "description": "Existing normalized repository-relative Dockerfile; symlinked and traversing paths are refused."
                        },
                        "context": {
                            "type": "string",
                            "description": "Normalized repository-relative build context directory. Defaults to the repository root."
                        },
                        "smoke_args": {
                            "type": "array",
                            "items": {"type": "string", "maxLength": 4096},
                            "maxItems": 32,
                            "description": "Command and arguments run inside the built image, for example [\"node\",\"-e\",\"require('.')\"]. Empty runs the image's own default command."
                        },
                        "build_timeout_secs": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 3600,
                            "description": "Build timeout in seconds. Defaults to 1800."
                        },
                        "smoke_timeout_secs": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 600,
                            "description": "Import smoke timeout in seconds. Defaults to 300."
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "Return the exact build, smoke, and cleanup plan without invoking Docker. Defaults to true."
                        },
                        "confirm": {
                            "type": "string",
                            "description": "Execution requires the exact phrase: run artifact build check"
                        }
                    },
                    "required": ["dockerfile"],
                    "additionalProperties": false
                }
            }
    )
}

pub(crate) fn compose_stack_run_definition() -> Value {
    json!({
                "name": tools::compose_stack_run::NAME,
                "description": "Plan or start one named Docker Compose stack proof. The compose file and every Docker argument are derived by this server from the action name; no caller-supplied Docker arguments are accepted. With dry_run=false, start the stack in the background, return a log_id immediately, and poll with read_command_log. Teardown is scoped to this server's own Compose project name and is always attempted, so a proof cannot stop an operator's own stack and cannot leave containers running. Unlike task_image_python_run, this path runs with networking enabled because a stack proof exercises service-to-service traffic. Compose, task-image, Harbor, and validation-profile jobs share a two-job cap.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": contextpatch_core::process::compose_stack::action_names(),
                            "description": "Named stack proof. Each action is pinned to one reviewed compose file."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 3600,
                            "description": "Stack timeout in seconds. Defaults to 1800."
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "Return the exact up and teardown plan without invoking Docker. Defaults to true."
                        },
                        "confirm": {
                            "type": "string",
                            "description": "Execution requires the exact phrase: run compose stack"
                        }
                    },
                    "required": ["action"],
                    "additionalProperties": false
                }
            }
    )
}

pub(crate) fn harbor_run_start_definition() -> Value {
    json!({
                "name": tools::harbor_run_start::NAME,
                "description": "Start one typed Harbor run in the background and return a log_id immediately. Poll with read_command_log; completed logs include structured Harbor evidence. Harbor, task-image, validation-profile, Compose-stack, and artifact-build jobs share a two-job cap.",
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
    )
}

pub(crate) fn image_cleanliness_check_run_definition() -> Value {
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
    )
}

pub(crate) fn docker_image_inspect_definition() -> Value {
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
    )
}

pub(crate) fn validation_profile_run_definition() -> Value {
    json!({
                "name": tools::validation_profile_run::NAME,
                "description": "Start a predefined validation profile in the background and return a log_id immediately. Poll with read_command_log; polling never restarts work.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "profile": {
                            "type": "string",
                            "enum": crate::tools::process::VALIDATION_PROFILE_NAMES,
                            "description": "Validation profile name."
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
    )
}
