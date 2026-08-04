use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definition() -> Value {
    json!({
        "name": tools::project_execute::NAME,
        "description": "Describe or execute one guarded ContextPatch action for this configured project.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "An existing ContextPatch action name, or describe."
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments for the selected action."
                },
                "repository": {
                    "type": "string",
                    "description": "Optional normalized workspace-relative path to an exact descendant Git worktree root. Omit it to use the configured --repo-root unchanged."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }
    })
}
