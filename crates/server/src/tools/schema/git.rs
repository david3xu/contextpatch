use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
                    "name": tools::git_commit_exact::NAME,
                    "description": "Dry-run or create one local Git commit from an exact full dirty-path set. Never pushes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "paths": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "minItems": 1,
                                "description": "Exact repository-relative dirty paths that must be the complete changed-path set."
                            },
                            "subject": {
                                "type": "string",
                                "description": "Commit subject line."
                            },
                            "body": {
                                "type": "string",
                                "description": "Optional commit body/trailers."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview without staging or committing. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `commit exact paths` when dry_run is false."
                            }
                        },
                        "required": ["paths", "subject"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_commit_scoped::NAME,
                    "description": "Dry-run or create one local Git commit from an explicit subset of dirty paths while preserving unrelated dirty files. Never pushes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "paths": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "minItems": 1,
                                "description": "Repository-relative dirty paths to stage and commit. Other dirty paths are preserved."
                            },
                            "subject": {
                                "type": "string",
                                "description": "Commit subject line."
                            },
                            "body": {
                                "type": "string",
                                "description": "Optional commit body/trailers."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview without staging or committing. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `commit scoped paths` when dry_run is false."
                            }
                        },
                        "required": ["paths", "subject"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_commit_prefix::NAME,
                    "description": "Dry-run or create one local Git commit from dirty paths under explicit prefixes after expanding and reporting the exact path list. Never pushes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "prefixes": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "minItems": 1,
                                "description": "Repository-relative dirty path prefixes or directories to expand."
                            },
                            "subject": {
                                "type": "string",
                                "description": "Commit subject line."
                            },
                            "body": {
                                "type": "string",
                                "description": "Optional commit body/trailers."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview expanded paths without staging or committing. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `commit prefix paths` when dry_run is false."
                            }
                        },
                        "required": ["prefixes", "subject"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_stage_exact::NAME,
                    "description": "Dry-run or stage explicit dirty paths without creating a commit. Requires a clean index first.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "paths": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "minItems": 1,
                                "description": "Repository-relative dirty paths to stage. Other dirty paths are preserved unstaged."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview without staging. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `stage exact paths` when dry_run is false."
                            }
                        },
                        "required": ["paths"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_staged_scope_check::NAME,
                    "description": "Read-only check that staged paths are limited to allowed exact paths and prefixes, with optional required staged paths.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "allowed_paths": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "description": "Repository-relative paths that may be staged exactly."
                            },
                            "allowed_prefixes": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "description": "Repository-relative prefixes or directories under which staged paths are allowed."
                            },
                            "required_paths": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "description": "Optional repository-relative paths that must be staged."
                            }
                        },
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_restore_exact::NAME,
                    "description": "Dry-run or restore exact dirty repository paths from HEAD. Use for generated noise cleanup before exact commits; never resets the whole worktree.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "paths": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "minItems": 1,
                                "description": "Repository-relative dirty paths to restore from HEAD. Every path must currently be dirty."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview without restoring. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `restore exact paths` when dry_run is false."
                            }
                        },
                        "required": ["paths"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::delete_untracked_exact::NAME,
                    "description": "Dry-run or delete explicit untracked files only. Refuses tracked files, directories, globs, and broad cleanup.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "paths": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "minItems": 1,
                                "description": "Repository-relative untracked file paths to delete."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview without deleting. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `delete untracked files` when dry_run is false."
                            }
                        },
                        "required": ["paths"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::delete_generated_prefix::NAME,
                    "description": "Dry-run or delete ignored/untracked generated files and empty directories under explicit prefixes without exposing broad git clean.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "prefixes": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "minItems": 1,
                                "description": "Repository-relative prefixes to expand for ignored/untracked generated cleanup."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview without deleting. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `delete generated paths` when dry_run is false."
                            }
                        },
                        "required": ["prefixes"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_remote_list::NAME,
                    "description": "Read-only Git remote inspection, equivalent to a parsed git remote -v.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_remote_check::NAME,
                    "description": "Fetch one remote branch and report whether the remote branch is ahead of HEAD. Does not modify source files.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "remote": {
                                "type": "string",
                                "description": "Git remote name. Defaults to origin."
                            },
                            "branch": {
                                "type": "string",
                                "description": "Branch name to compare with the remote-tracking ref."
                            }
                        },
                        "required": ["branch"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_branch_prepare::NAME,
                    "description": "Prepare and switch to a local branch from one explicit remote base branch with guard checks.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "remote": {
                                "type": "string",
                                "description": "Git remote name. Defaults to origin."
                            },
                            "base_branch": {
                                "type": "string",
                                "description": "Remote base branch to fetch and prepare from."
                            },
                            "branch": {
                                "type": "string",
                                "description": "Local branch to create or switch to."
                            },
                            "required_files": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "description": "Optional repository-relative files that must exist after preparation."
                            },
                            "reset_existing": {
                                "type": "boolean",
                                "description": "Reset an existing local branch to the fetched remote base. Defaults to false."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `reset branch from remote base` when reset_existing is true."
                            }
                        },
                        "required": ["base_branch", "branch"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_merge_readiness::NAME,
                    "description": "Read-only merge/PR readiness analysis between two refs, including changed-on-both-sides conflict candidates.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "base_ref": {
                                "type": "string",
                                "description": "Base ref to compare, such as HEAD, main, origin/main, refs/heads/main, or a commit hash."
                            },
                            "target_ref": {
                                "type": "string",
                                "description": "Target ref to compare, such as feature, origin/feature, refs/remotes/origin/feature, or a commit hash."
                            },
                            "fetch": {
                                "type": "boolean",
                                "description": "Optionally fetch one explicit remote branch before analysis. Defaults to false."
                            },
                            "remote": {
                                "type": "string",
                                "description": "Remote name used only when fetch is true. Defaults to origin."
                            },
                            "target_branch": {
                                "type": "string",
                                "description": "Remote branch to fetch when fetch is true. Inferred from target_ref when target_ref is a remote-tracking ref."
                            }
                        },
                        "required": ["base_ref", "target_ref"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::git_push_exact::NAME,
                    "description": "Push the current branch HEAD to the matching remote branch only after exact hash and divergence checks.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "remote": {
                                "type": "string",
                                "description": "Git remote name."
                            },
                            "branch": {
                                "type": "string",
                                "description": "Current branch name and matching remote branch name."
                            },
                            "expected_head": {
                                "type": "string",
                                "description": "Full or short commit hash expected at HEAD."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `push exact commit`."
                            }
                        },
                        "required": ["remote", "branch", "expected_head", "confirm"],
                        "additionalProperties": false
                    }
                }
        ),
    ]
}
