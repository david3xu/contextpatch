use serde_json::{json, Map, Value};

use crate::tools;

pub mod project_execute {
    pub const NAME: &str = "project_execute";
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolSurface {
    Full,
    Project,
}

impl ToolSurface {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "full" => Ok(Self::Full),
            "project" => Ok(Self::Project),
            _ => Err(format!(
                "--tool-surface must be `full` or `project`, got `{value}`"
            )),
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Project => "project",
        }
    }
}

pub(crate) enum ProjectCall {
    Describe {
        text: String,
        repository: Option<String>,
    },
    Execute {
        name: String,
        arguments: Map<String, Value>,
        repository: Option<String>,
    },
}

pub(crate) fn resolve(arguments: &Map<String, Value>) -> Result<ProjectCall, String> {
    reject_unknown_keys(arguments, &["action", "arguments", "repository"])?;
    let action = arguments
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "project_execute refused: missing or invalid string argument: action".to_string()
        })?;
    let nested = match arguments.get("arguments") {
        Some(value) => value.as_object().cloned().ok_or_else(|| {
            "project_execute refused: arguments must be an object when provided".to_string()
        })?,
        None => Map::new(),
    };
    let repository = arguments
        .get("repository")
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                "project_execute refused: repository must be a string when provided".to_string()
            })
        })
        .transpose()?;

    if action == "describe" {
        reject_unknown_keys(&nested, &["name", "action"])?;
        let name =
            match (nested.get("name"), nested.get("action")) {
                (Some(_), Some(_)) => return Err(
                    "project_execute refused: describe accepts either `name` or `action`, not both"
                        .to_string(),
                ),
                (Some(value), None) | (None, Some(value)) => value.as_str().ok_or_else(|| {
                    "project_execute refused: describe name/action must be a string".to_string()
                })?,
                (None, None) => "",
            };
        return describe(name).map(|text| ProjectCall::Describe { text, repository });
    }

    if action == project_execute::NAME {
        return Err(
            "project_execute refused: recursive wrapper dispatch is not allowed".to_string(),
        );
    }
    if tools::schema::internal_action_definition(action).is_none() {
        return Err(format!(
            "project_execute refused: unknown action `{action}`; use action `describe` to list \
             available actions"
        ));
    }

    Ok(ProjectCall::Execute {
        name: action.to_string(),
        arguments: nested,
        repository,
    })
}

fn describe(name: &str) -> Result<String, String> {
    let document = if name.is_empty() {
        json!({
            "server": "contextpatch",
            "build": tools::capability::build_metadata(),
            "tool_surface": ToolSurface::Project.as_str(),
            "action_count": tools::schema::internal_action_names().len(),
            "action_names": tools::schema::internal_action_names(),
            "action_definitions": tools::schema::internal_action_definitions(),
            "repository_selection": {
                "argument": "repository",
                "scope": "optional normalized workspace-relative path to an exact descendant Git worktree root",
                "omitted": "use the configured --repo-root unchanged"
            },
            "note": "Execute exactly one action by passing its name as action and its original \
                     arguments as arguments. Existing guards, deadlines, locks, confirmations, and \
                     receipts remain authoritative."
        })
    } else {
        let definition = tools::schema::internal_action_definition(name).ok_or_else(|| {
            format!(
                "project_execute refused: unknown action `{name}`; omit arguments.name to list \
                 available actions"
            )
        })?;
        json!({
            "server": "contextpatch",
            "build": tools::capability::build_metadata(),
            "tool_surface": ToolSurface::Project.as_str(),
            "action": name,
            "definition": definition
        })
    };

    serde_json::to_string_pretty(&document)
        .map_err(|error| format!("project_execute refused: {error}"))
}

fn reject_unknown_keys(arguments: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(key) = arguments
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
    {
        let mut permitted = allowed.to_vec();
        permitted.sort_unstable();
        return Err(format!(
            "project_execute refused: unknown argument `{key}`; permitted arguments: {}",
            permitted.join(", ")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_internal_action_without_changing_arguments() {
        let arguments = serde_json::from_value(json!({
            "repository": "child-repository",
            "action": "read_range",
            "arguments": {
                "path": "README.md",
                "start_line": 1,
                "end_line": 2
            }
        }))
        .unwrap();

        let ProjectCall::Execute {
            name,
            arguments,
            repository,
        } = resolve(&arguments).unwrap()
        else {
            panic!("expected executable action");
        };
        assert_eq!(name, "read_range");
        assert_eq!(repository.as_deref(), Some("child-repository"));
        assert_eq!(arguments["path"], "README.md");
        assert_eq!(arguments["start_line"], 1);
    }

    #[test]
    fn rejects_unknown_and_recursive_actions() {
        for action in ["missing_action", project_execute::NAME] {
            let arguments =
                serde_json::from_value(json!({"action": action, "arguments": {}})).unwrap();
            assert!(resolve(&arguments).is_err());
        }
    }

    #[test]
    fn rejects_malformed_wrapper_arguments() {
        let cases = [
            (json!({}), "missing or invalid string argument: action"),
            (
                json!({"action": 7}),
                "missing or invalid string argument: action",
            ),
            (
                json!({"action": "read_range", "arguments": []}),
                "arguments must be an object",
            ),
            (
                json!({"action": "describe", "arguments": {"name": 7}}),
                "describe name/action must be a string",
            ),
            (
                json!({"action": "describe", "arguments": {"extra": true}}),
                "unknown argument `extra`",
            ),
            (
                json!({"action": "read_range", "extra": true}),
                "unknown argument `extra`",
            ),
            (
                json!({"action": "read_range", "repository": 7}),
                "repository must be a string",
            ),
        ];

        for (value, expected) in cases {
            let error = resolve(value.as_object().unwrap()).err().unwrap();
            assert!(error.contains(expected), "{error}");
        }
    }
}
