use serde_json::{json, Value};

mod authority;
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

/// Action names a wrapper caller may pass, including the meta action where it is dispatchable.
///
/// [`internal_action_names`] reports the tool registry, which the documentation-contract tests compare
/// against the specification. `describe` is dispatched by the wrapper rather than registered as a tool,
/// so it belongs in what a project-surface caller is told it can call, and nowhere in full mode where
/// the wrapper is not advertised at all. Without it, a client that enumerates actions cannot discover
/// the one action it needs in order to enumerate anything else.
pub(crate) fn wrapper_action_names(surface: ToolSurface) -> Vec<String> {
    let mut names = internal_action_names();
    if surface == ToolSurface::Project {
        names.push(crate::tools::project_execute::DESCRIBE_ACTION.to_string());
        names.sort();
    }
    names
}

pub(crate) fn internal_action_definitions() -> Vec<Value> {
    internal_tool_definitions()
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

pub(crate) fn validate_internal_action_arguments(
    name: &str,
    arguments: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    let definition =
        internal_action_definition(name).ok_or_else(|| format!("unknown tool: {name}"))?;
    let input_schema = definition
        .get("inputSchema")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{name} refused: internal input schema is missing"))?;
    if input_schema.get("additionalProperties") != Some(&Value::Bool(false)) {
        return Err(format!(
            "{name} refused: internal input schema is not closed"
        ));
    }
    let properties = input_schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{name} refused: internal input properties are missing"))?;
    if let Some(argument) = arguments
        .keys()
        .find(|argument| !properties.contains_key(*argument))
    {
        let mut permitted = properties.keys().map(String::as_str).collect::<Vec<_>>();
        permitted.sort_unstable();
        return Err(format!(
            "{name} refused: unknown argument `{argument}`; permitted arguments: {}",
            permitted.join(", ")
        ));
    }
    Ok(())
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
    let read_only = authority::is_read_only(name);
    let open_world = authority::remote_reach(name).is_open_world();

    object.insert(
        "annotations".to_string(),
        json!({
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": read_only,
            "openWorldHint": open_world
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
    fn the_wrapper_description_advertises_the_cheap_discovery_projections() {
        // The wrapper schema is the only thing a project-surface client reads before its first call. If
        // the narrowed forms are absent from it, the natural first call returns every action schema and
        // pays that cost in every session, which is exactly what happened before this test existed.
        let definition = project_tool_definition();
        let description = definition["description"].as_str().unwrap();

        for advertised in [
            "arguments.name",
            "names_only",
            "response_mode",
            "minimal",
            crate::tools::project_execute::DESCRIBE_ACTION,
            "capability_manifest",
            "preflight_health",
        ] {
            assert!(
                description.contains(advertised),
                "the wrapper description must advertise `{advertised}`: {description}"
            );
        }

        let arguments = definition["inputSchema"]["properties"]["arguments"]["description"]
            .as_str()
            .unwrap();
        assert!(
            arguments.contains("name"),
            "the arguments property must point at the narrowed describe form: {arguments}"
        );
    }

    #[test]
    fn the_meta_action_is_reported_only_where_it_is_dispatchable() {
        let project = wrapper_action_names(ToolSurface::Project);
        let full = wrapper_action_names(ToolSurface::Full);
        let describe = crate::tools::project_execute::DESCRIBE_ACTION.to_string();

        // Project mode dispatches the meta action through the wrapper, so a client enumerating actions
        // must see it. Full mode does not advertise the wrapper at all, so reporting it there would name
        // something uncallable.
        assert!(project.contains(&describe), "{project:?}");
        assert!(!full.contains(&describe), "{full:?}");
        assert_eq!(project.len(), full.len() + 1);
        assert_eq!(full, internal_action_names());

        // Still sorted, so the reported list stays deterministic.
        let mut sorted = project.clone();
        sorted.sort();
        assert_eq!(project, sorted);
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

    #[test]
    fn runtime_argument_validation_rejects_names_outside_closed_schema() {
        let valid = serde_json::from_value(json!({
            "path": "README.md",
            "start_line": 1,
            "end_line": 2
        }))
        .unwrap();
        validate_internal_action_arguments("read_range", &valid).unwrap();

        let invalid = serde_json::from_value(json!({
            "path": "README.md",
            "start_line": 1,
            "end_line": 2,
            "dry_run": true
        }))
        .unwrap();
        let error = validate_internal_action_arguments("read_range", &invalid).unwrap_err();
        assert_eq!(
            error,
            "read_range refused: unknown argument `dry_run`; permitted arguments: end_line, path, start_line"
        );
    }
}
