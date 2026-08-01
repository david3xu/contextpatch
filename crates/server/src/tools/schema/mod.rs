use serde_json::{json, Value};

mod capability;
mod files;
mod fixtures;
mod git;
mod github;
mod native;
mod process;
mod project;
mod setup;

use crate::tools::ToolSurface;

fn internal_tool_definitions() -> Vec<Value> {
    let mut definitions = Vec::new();
    definitions.extend(capability::definitions());
    definitions.extend(files::definitions());
    definitions.extend(process::definitions());
    definitions.extend(fixtures::definitions());
    definitions.extend(setup::definitions());
    definitions.extend(native::definitions());
    definitions.extend(git::definitions());
    definitions.extend(github::definitions());
    for definition in &mut definitions {
        add_always_allow_annotations(definition);
    }
    definitions
}

pub(crate) fn tool_definitions(surface: ToolSurface) -> Value {
    match surface {
        ToolSurface::Full => Value::Array(internal_tool_definitions()),
        ToolSurface::Project => Value::Array(vec![project_tool_definition()]),
    }
}

pub(crate) fn internal_action_names() -> Vec<String> {
    sorted_definition_names(&internal_tool_definitions())
}

pub(crate) fn public_tool_names(surface: ToolSurface) -> Vec<String> {
    let definitions = match surface {
        ToolSurface::Full => internal_tool_definitions(),
        ToolSurface::Project => vec![project_tool_definition()],
    };
    sorted_definition_names(&definitions)
}

fn project_tool_definition() -> Value {
    let mut definition = project::definition();
    add_always_allow_annotations(&mut definition);
    definition
}

pub(crate) fn internal_action_definition(name: &str) -> Option<Value> {
    internal_tool_definitions()
        .into_iter()
        .find(|definition| definition.get("name").and_then(Value::as_str) == Some(name))
}

#[cfg(test)]
fn documented_tool_names() -> Vec<String> {
    let mut names = internal_action_names();
    names.push(crate::tools::project_execute::NAME.to_string());
    names.sort();
    names.dedup();
    names
}

fn sorted_definition_names(definitions: &[Value]) -> Vec<String> {
    let mut names: Vec<String> = definitions
        .iter()
        .filter_map(|definition| definition.get("name"))
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    names
}

fn add_always_allow_annotations(definition: &mut Value) {
    let Some(object) = definition.as_object_mut() else {
        return;
    };
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let read_only = matches!(
        name,
        "capability_manifest"
            | "preflight_health"
            | "read_range"
            | "read_write_receipts"
            | "diff_preview"
            | "status_guard"
            | "file_info"
            | "list_directory"
            | "read_file_bytes"
            | "read_command_log"
            | "fixture_manifest_verify"
            | "git_remote_list"
            | "git_merge_readiness"
            | "git_staged_scope_check"
    );

    object.insert(
        "annotations".to_string(),
        json!({
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_spec() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/tool-spec.md")
            .canonicalize()
            .expect("docs/tool-spec.md must exist");
        std::fs::read_to_string(path).expect("docs/tool-spec.md must be readable")
    }

    fn registered_names() -> Vec<String> {
        documented_tool_names()
    }

    #[test]
    fn every_registered_tool_has_a_documented_contract() {
        // The drift this catches is real and has happened repeatedly: a tool is registered, works, and is
        // absent from the spec, so a reader concludes it does not exist. Checking the contract heading
        // rather than the summary table, because the heading is where the guarantees live.
        let spec = tool_spec();
        let undocumented: Vec<String> = registered_names()
            .into_iter()
            .filter(|name| !spec.contains(&format!("### `{name}`")))
            .collect();
        assert!(
            undocumented.is_empty(),
            "registered tools missing a `### `name`` contract in docs/tool-spec.md: {undocumented:?}"
        );
    }

    #[test]
    fn every_documented_contract_is_a_registered_tool() {
        // The reverse drift is worse, because a documented tool that does not exist wastes a caller's
        // time and teaches them to distrust the document.
        let spec = tool_spec();
        let registered = registered_names();
        let mut phantom = Vec::new();
        for line in spec.lines() {
            let Some(rest) = line.strip_prefix("### `") else {
                continue;
            };
            let Some(name) = rest.strip_suffix('`') else {
                continue;
            };
            if registered.iter().any(|known| known == name) {
                continue;
            }
            phantom.push(name.to_string());
        }
        assert!(
            phantom.is_empty(),
            "docs/tool-spec.md documents tools that are not registered: {phantom:?}"
        );
    }

    #[test]
    fn summary_table_exactly_matches_registered_tools() {
        // The table is what a reader skims first. Both omissions and phantom rows misstate the actual
        // MCP surface, so compare the parsed set rather than searching one direction by substring.
        let spec = tool_spec();
        let summary = spec
            .split_once("## Naming")
            .map(|(summary, _)| summary)
            .expect("docs/tool-spec.md must keep the summary before Naming");
        let mut advertised: Vec<String> = summary
            .lines()
            .filter_map(|line| line.strip_prefix("| `"))
            .filter_map(|line| line.split_once("` |").map(|(name, _)| name.to_string()))
            .collect();
        advertised.sort();
        let advertised_count = advertised.len();
        advertised.dedup();
        assert_eq!(
            advertised.len(),
            advertised_count,
            "docs/tool-spec.md summary table contains duplicate tool rows"
        );
        assert_eq!(
            advertised,
            registered_names(),
            "docs/tool-spec.md summary table must advertise exactly the registered MCP tools"
        );
    }
}
