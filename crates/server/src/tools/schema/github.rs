use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
                    "name": tools::github_pr_run::NAME,
                    "description": "Run narrow GitHub review workflows through gh: auth status, PR view/checks, Actions run/job evidence, or confirmation-gated PR creation.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "action": {
                                "type": "string",
                                "enum": ["auth_status", "pr_view", "pr_checks", "workflow_run_view", "workflow_job_log", "workflow_run_rerun_failed", "pr_create"],
                                "description": "GitHub workflow action."
                            },
                            "number": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Pull request number for pr_view or pr_checks."
                            },
                            "repository": {
                                "type": "string",
                                "pattern": "^[A-Za-z0-9-]+/[A-Za-z0-9._-]+$",
                                "description": "Optional explicit OWNER/REPO target for PR and Actions workflows. Use this when the checkout remote is a fork but the PR or run belongs to upstream."
                            },
                            "run_id": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Workflow run database ID for workflow_run_view or workflow_run_rerun_failed."
                            },
                            "job_id": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Workflow job database ID for workflow_job_log."
                            },
                            "base": {
                                "type": "string",
                                "description": "Base branch for pr_create."
                            },
                            "head": {
                                "type": "string",
                                "description": "Head branch for pr_create."
                            },
                            "title": {
                                "type": "string",
                                "description": "Pull request title for pr_create."
                            },
                            "body": {
                                "type": "string",
                                "description": "Pull request body for pr_create."
                            },
                            "draft": {
                                "type": "boolean",
                                "description": "Create a draft pull request. Defaults to false."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "For pr_create and workflow_run_rerun_failed, default true. When true, returns the gh command plan without mutating GitHub."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `create pull request` for confirmed PR creation or `rerun failed workflow jobs` for a confirmed failed-job rerun."
                            }
                        },
                        "required": ["action"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::github_fork_prepare::NAME,
                    "description": "Plan or run a guarded gh repo fork workflow for the current repository.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "remote": {
                                "type": "boolean",
                                "description": "Pass --remote to add fork remotes. Defaults to true."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Plan only without mutating GitHub or remotes. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `prepare github fork` when dry_run is false."
                            }
                        },
                        "additionalProperties": false
                    }
                }
        ),
    ]
}
