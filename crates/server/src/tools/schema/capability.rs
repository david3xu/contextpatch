use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
                    "name": tools::capability_manifest::NAME,
                    "description": "Report the contextpatch server capability contract, including whether guarded process execution is available.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }
                }
        ),
        json!({
                    "name": tools::preflight_health::NAME,
                    "description": "Check repository and local tool readiness for Claude Desktop workflows without mutating the repository.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {},
                        "additionalProperties": false
                    }
                }
        ),
    ]
}
