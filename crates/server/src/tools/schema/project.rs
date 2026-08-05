use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definition() -> Value {
    json!({
        "name": tools::project_execute::NAME,
        "description": "Describe or execute one guarded ContextPatch action for this configured project. Discovery is projectable, and the cheap projections are worth knowing before the expensive one: action `describe` with `arguments.name` set to an action name returns that one action's schema, action `capability_manifest` with `arguments.names_only` true returns the action-name list plus the build stamp, and action `preflight_health` with `arguments.response_mode` set to `minimal` returns readiness counts and still reports the configured repository root. Calling `describe` with no arguments returns every action schema at once, so reach for it only when the whole set is genuinely wanted.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "An existing ContextPatch action name, or `describe` for discovery. Reported action-name lists include `describe`, so enumerating actions finds it."
                },
                "arguments": {
                    "type": "object",
                    "description": "Arguments for the selected action. For `describe`, pass `name` (or `action`) to return a single action's schema instead of all of them."
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
