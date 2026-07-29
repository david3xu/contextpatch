use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
                    "name": tools::read_range::NAME,
                    "description": "Read a bounded section of a UTF-8 text file with 1-based line numbers.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "File path relative to the configured repository root."
                            },
                            "start_line": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "First 1-based line number to read."
                            },
                            "end_line": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "Last 1-based line number to read."
                            }
                        },
                        "required": ["path", "start_line", "end_line"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::diff_preview::NAME,
                    "description": "Return a unified diff for an exact replacement without writing.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "File path relative to the configured repository root."
                            },
                            "old": {
                                "type": "string",
                                "description": "Existing text that must appear exactly once."
                            },
                            "new": {
                                "type": "string",
                                "description": "Replacement text to preview."
                            }
                        },
                        "required": ["path", "old", "new"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::replace_exact::NAME,
                    "description": "Replace text only when the old text matches exactly once.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "File path relative to the configured repository root."
                            },
                            "old": {
                                "type": "string",
                                "description": "Existing text that must appear exactly once."
                            },
                            "new": {
                                "type": "string",
                                "description": "Replacement text."
                            }
                        },
                        "required": ["path", "old", "new"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::status_guard::NAME,
                    "description": "Refuse when the repository or requested path has uncommitted Git changes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Optional file or directory path relative to the configured repository root."
                            }
                        },
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::write_new_file::NAME,
                    "description": "Create a new UTF-8 text file only when the destination does not already exist.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "File path relative to the configured repository root."
                            },
                            "content": {
                                "type": "string",
                                "description": "Full file content to write."
                            }
                        },
                        "required": ["path", "content"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::write_new_file_base64::NAME,
                    "description": "Create a new binary file from base64 only when the destination does not already exist.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "File path relative to the configured repository root. Parent directory must already exist."
                            },
                            "content_base64": {
                                "type": "string",
                                "description": "Base64-encoded file content to write."
                            },
                            "expected_bytes": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 20971520,
                                "description": "Optional decoded byte count guard."
                            }
                        },
                        "required": ["path", "content_base64"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::write_existing_file_exact_hash::NAME,
                    "description": "Overwrite an existing repository file only when the current SHA-256 hash matches the caller's expectation.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Existing file path relative to the configured repository root."
                            },
                            "content": {
                                "type": "string",
                                "description": "Full UTF-8 file content to write."
                            },
                            "expected_sha256": {
                                "type": "string",
                                "description": "Lowercase SHA-256 hex digest of the current file content."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Validate and preview without writing. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `write exact hash` when dry_run is false."
                            }
                        },
                        "required": ["path", "content", "expected_sha256"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::artifact_write_text::NAME,
                    "description": "Create a new UTF-8 text artifact outside the repository under the fixed contextpatch artifact directory.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Artifact path relative to the fixed artifact root. Parent directory must already exist unless parents is true."
                            },
                            "content": {
                                "type": "string",
                                "description": "Full artifact content to write."
                            },
                            "parents": {
                                "type": "boolean",
                                "description": "When true, create missing parent directories under the artifact root. Defaults to false."
                            }
                        },
                        "required": ["path", "content"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::artifact_write_base64::NAME,
                    "description": "Create a new binary artifact outside the repository under the fixed contextpatch artifact directory.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Artifact path relative to the fixed artifact root. Parent directory must already exist unless parents is true."
                            },
                            "content_base64": {
                                "type": "string",
                                "description": "Base64-encoded artifact content to write."
                            },
                            "expected_bytes": {
                                "type": "integer",
                                "minimum": 0,
                                "maximum": 20971520,
                                "description": "Optional decoded byte count guard."
                            },
                            "parents": {
                                "type": "boolean",
                                "description": "When true, create missing parent directories under the artifact root. Defaults to false."
                            }
                        },
                        "required": ["path", "content_base64"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::bulk_write_new_files_base64::NAME,
                    "description": "Create many new repository files from base64 entries in one bounded, create-only fixture import.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "entries": {
                                "type": "array",
                                "minItems": 1,
                                "maxItems": 500,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "path": {
                                            "type": "string",
                                            "description": "Repository-relative destination path. Existing files are refused."
                                        },
                                        "content_base64": {
                                            "type": "string",
                                            "description": "Base64-encoded file content."
                                        },
                                        "expected_bytes": {
                                            "type": "integer",
                                            "minimum": 0,
                                            "maximum": 20971520,
                                            "description": "Optional decoded byte count guard for this entry."
                                        }
                                    },
                                    "required": ["path", "content_base64"],
                                    "additionalProperties": false
                                },
                                "description": "Files to create. Total decoded size is limited to 20 MiB."
                            },
                            "parents": {
                                "type": "boolean",
                                "description": "When true, create missing parent directories inside the repository root. Defaults to false."
                            }
                        },
                        "required": ["entries"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::create_directory::NAME,
                    "description": "Create a new directory only when the destination does not already exist.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Directory path relative to the configured repository root."
                            },
                            "parents": {
                                "type": "boolean",
                                "description": "When true, create missing parent directories inside the repository root. Defaults to false."
                            }
                        },
                        "required": ["path"],
                        "additionalProperties": false
                    }
                }
        ),
    ]
}
