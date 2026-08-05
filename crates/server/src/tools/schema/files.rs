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
                    "name": tools::artifact_delete_exact::NAME,
                    "description": "Delete one exact regular artifact file outside the repository after a hash-reporting dry run and explicit confirmation.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Artifact path relative to the fixed artifact root. Must be an existing regular file."
                            },
                            "expected_sha256": {
                                "type": "string",
                                "pattern": "^[0-9a-f]{64}$",
                                "description": "Current lowercase SHA-256 reported by a dry run. Required when dry_run is false."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Inspect and report the current digest without deleting. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Required literal value `delete artifact exact` when dry_run is false."
                            }
                        },
                        "required": ["path"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::read_write_receipts::NAME,
                    "description": "Report recent mutation attempts and whether each one settled. Call this after a tool call is interrupted or times out to find out whether the write landed.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "interrupted_only": {
                                "type": "boolean",
                                "description": "Return only attempts that began and never settled, which is what a caller wants after a timeout. Defaults to false."
                            },
                            "limit": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 100,
                                "description": "Maximum number of attempts to return, newest first. Defaults to 25."
                            }
                        },
                        "required": [],
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
                            },
                            "expected_sha256": {
                                "type": "string",
                                "pattern": "^[0-9a-f]{64}$",
                                "description": "Optional lowercase SHA-256 digest of the complete current file. Use this to refuse if another agent changed the file after it was read."
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
                    "name": tools::file_info::NAME,
                    "description": "Return read-only metadata for one path or a bounded batch of paths, including SHA-256 for files, UTF-8 line count when available, mode, and symlink status.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "File or directory path relative to the configured repository root."
                            },
                            "paths": {
                                "type": "array",
                                "items": {
                                    "type": "string"
                                },
                                "minItems": 1,
                                "maxItems": 64,
                                "uniqueItems": true,
                                "description": "A bounded batch of repository-relative file or directory paths."
                            }
                        },
                        "oneOf": [
                            {"required": ["path"], "not": {"required": ["paths"]}},
                            {"required": ["paths"], "not": {"required": ["path"]}}
                        ],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::set_file_executable::NAME,
                    "description": "Plan or apply an exact-hash, exact-mode change to only the executable bits of one regular repository file.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Existing regular repository file; symlinks are refused."
                            },
                            "executable": {
                                "type": "boolean",
                                "description": "Whether owner, group, and other executable bits should be enabled."
                            },
                            "expected_sha256": {
                                "type": "string",
                                "pattern": "^[0-9a-f]{64}$",
                                "description": "Required for execution; copy from a current dry-run plan."
                            },
                            "expected_mode": {
                                "type": "string",
                                "pattern": "^[0-7]{3,4}$",
                                "description": "Required for execution; copy from a current dry-run plan."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "description": "Plan without changing the file. Defaults to true."
                            },
                            "confirm": {
                                "type": "string",
                                "description": "Execution requires the exact phrase: set file executable"
                            }
                        },
                        "required": ["path", "executable"],
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::list_directory::NAME,
                    "description": "List one repository directory with optional bounded recursion, entry type, symlink flag, and file sizes without following symlink directories.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Directory path relative to the configured repository root. Defaults to repository root."
                            },
                            "include_hidden": {
                                "type": "boolean",
                                "description": "Include dotfiles and dot-directories. Defaults to false."
                            },
                            "recursive": {
                                "type": "boolean",
                                "description": "Recurse into ordinary directories. Defaults to false."
                            },
                            "max_depth": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 16,
                                "description": "Maximum recursive depth. Defaults to 4."
                            },
                            "max_entries": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 2000,
                                "description": "Maximum entries to return. Defaults to 2000."
                            }
                        },
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::read_file_bytes::NAME,
                    "description": "Read a bounded byte range from a repository file as hex or base64, including total size and SHA-256.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "Existing regular file path relative to the configured repository root."
                            },
                            "offset": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "Byte offset to start reading from. Defaults to 0."
                            },
                            "max_bytes": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 1048576,
                                "description": "Maximum bytes to return. Defaults to 4096."
                            },
                            "encoding": {
                                "type": "string",
                                "enum": ["hex", "base64"],
                                "description": "Output encoding. Defaults to hex."
                            }
                        },
                        "required": ["path"],
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
                    "name": tools::bulk_replace_exact::NAME,
                    "description": "Validate every exact replacement before the first write, then apply one atomic write per file. Several entries may target the same file; those hunks resolve against a single snapshot of it and land in its single write. Validation refusals leave all files unchanged; interruption or apply failure can leave a prefix of files applied, recoverable through per-file receipts.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "entries": {
                                "type": "array",
                                "minItems": 1,
                                "maxItems": contextpatch_core::replace::exact::MAX_BULK_REPLACE_ENTRIES,
                                "description": "Replacements to validate together. Repeating one path adds another hunk to that file; two different paths that resolve to the same filesystem file, including hard-link and case aliases, are refused. Hunks in one file must not resolve to the same or intersecting byte ranges, and must not demand different expected_sha256 values.",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "path": {
                                            "type": "string",
                                            "description": "Repository relative path to an existing UTF-8 text file."
                                        },
                                        "old": {
                                            "type": "string",
                                            "description": "Text to replace. Must occur exactly once in the file."
                                        },
                                        "new": {
                                            "type": "string",
                                            "description": "Replacement text."
                                        },
                                        "expected_sha256": {
                                            "type": "string",
                                            "pattern": "^[0-9a-f]{64}$",
                                            "description": "Optional lowercase SHA-256 digest checked during validation. Apply also revalidates the exact bytes captured during planning."
                                        }
                                    },
                                    "required": ["path", "old", "new"],
                                    "additionalProperties": false
                                }
                            }
                        },
                        "required": ["entries"],
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
