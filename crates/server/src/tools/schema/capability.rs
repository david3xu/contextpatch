use serde_json::{json, Value};

use crate::tools;

pub(crate) fn definitions() -> Vec<Value> {
    vec![
        json!({
                    "name": tools::capability_manifest::NAME,
                    "description": "Report the contextpatch server capability contract: available tools, allowlisted programs, guards, and the build stamp. Call this first. Pass names_only for a cheap tool list, or section for one part of the contract; both always include the build stamp so staleness stays checkable.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "names_only": {
                                "type": "boolean",
                                "description": "Return just the sorted tool names plus the build stamp. Cheapest way to check whether a tool exists."
                            },
                            "section": {
                                "type": "string",
                                "enum": crate::tools::capability::PROJECTABLE_SECTIONS,
                                "description": "Return only this part of the contract. Ignored when names_only is true."
                            }
                        },
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
