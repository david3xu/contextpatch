use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
                    "name": tools::fixture_generator_run::NAME,
                    "description": "Plan or run a repo-relative Python fixture generator that may mutate only declared output paths or prefixes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "script_path": {
                                "type": "string",
                                "description": "Repository-relative .py script to run with python3. The script itself may be the only pre-existing dirty path unless allowed_existing_dirty_paths is provided."
                            },
                            "args": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Arguments passed after the script path. Use only explicit argv strings; no shell interpolation is performed."
                            },
                            "cwd": {
                                "type": "string",
                                "description": "Optional working directory relative to the repository root. Defaults to the repository root."
                            },
                            "expected_output_paths": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Exact repository-relative paths the generator may create or modify."
                            },
                            "expected_output_prefixes": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Repository-relative directory prefixes under which the generator may create or modify files."
                            },
                            "allowed_existing_dirty_paths": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Pre-existing dirty paths allowed before the run. Defaults to the script path only, supporting temporary untracked generators that are deleted before commit."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600,
                                "description": "Optional timeout in seconds. Defaults to 120, maximum 600."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Plan only without running Python. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `run fixture generator` when dry_run is false."
                            }
                        },
                        "required": ["script_path"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::base_image_check_run::NAME,
                    "description": "Plan or run the exact repository base-image check script references/check-base-image.sh without enabling arbitrary shell access.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "project_path": {
                                "type": "string",
                                "description": "Optional repository-relative project directory argument. For Dynamo/Harbor tasks this is exactly `task`."
                            },
                            "timeout_secs": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 600,
                                "description": "Optional timeout in seconds. Defaults to 120, maximum 600."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Plan only without running the script. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `run base image check` when dry_run is false."
                            }
                        },
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::fixture_manifest_verify::NAME,
                    "description": "Verify a fixture manifest's exact file set and SHA-256 digests against declared repository fixture paths or prefixes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "manifest_path": {
                                "type": "string",
                                "description": "Repository-relative manifest JSON path."
                            },
                            "fixture_paths": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Exact fixture files to include in the file-set check."
                            },
                            "fixture_prefixes": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Repository-relative directories or file prefixes whose regular files must exactly match the manifest."
                            }
                        },
                        "required": ["manifest_path"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::fixture_manifest_refresh::NAME,
                    "description": "Regenerate a fixture manifest JSON from declared repository fixture paths or prefixes, guarded by dry-run, confirmation, and existing manifest hash.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "manifest_path": {
                                "type": "string",
                                "description": "Repository-relative manifest JSON path to create or overwrite."
                            },
                            "fixture_paths": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Exact fixture files to include."
                            },
                            "fixture_prefixes": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Repository-relative directories or file prefixes whose regular files are included."
                            },
                            "expected_manifest_sha256": {
                                "type": "string",
                                "description": "Required when overwriting an existing manifest; must match its current SHA-256."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Preview the generated manifest metadata without writing. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `refresh fixture manifest` when dry_run is false."
                            }
                        },
                        "required": ["manifest_path"],
                        "additionalProperties": false
                    }
                }
        ),
    ]
}
