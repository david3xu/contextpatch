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

// Re-exported so the advertised-authority axes can be asserted alongside the deadline and
// mutation-lock axes that live in `dispatch`. Those six facts are currently decided in five separate
// files; the registry migration collapses them, and until it does, the snapshot that guards the
// migration needs to read all six from one place.
pub(crate) use authority::RemoteReach;
pub(crate) use capability::{capability_manifest_definition, preflight_health_definition};
pub(crate) use fixtures::{
    base_image_check_run_definition, fixture_generator_run_definition,
    fixture_manifest_refresh_definition, fixture_manifest_verify_definition,
};
pub(crate) use github::{github_fork_prepare_definition, github_pr_run_definition};
pub(crate) use native::{native_build_run_definition, native_device_run_definition};
pub(crate) use setup::setup_profile_run_definition;
// Still test-only: production reads these through `add_always_allow_annotations`, which lives here.
// They become ordinary reads once every tool is a descriptor and annotations come from its fields.
#[cfg(test)]
pub(crate) use authority::{is_read_only, remote_reach};

fn internal_tool_definitions() -> Vec<Value> {
    let mut definitions = Vec::new();
    // Migrated tools carry their own schema on their descriptor; the module lists below hold only
    // what has not moved yet, so the two sources are disjoint by construction.
    definitions.extend(
        crate::tools::registry::descriptors()
            .iter()
            .map(|entry| (entry.schema)()),
    );
    definitions.extend(files::definitions());
    definitions.extend(process::definitions());
    definitions.extend(git::definitions());
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

    /// The advertised surface, recorded before the tool registry migration.
    ///
    /// Names, schemas, descriptions, and annotations are the entire public contract of this server, so
    /// a migration that moves all of them must prove it produced the same thing rather than merely
    /// something that works. Definitions are sorted by name because registration order is not part of
    /// the contract and would otherwise churn when modules are split.
    #[test]
    fn the_advertised_tool_surface_matches_its_recorded_snapshot() {
        let mut definitions = internal_tool_definitions();
        definitions.push(project_tool_definition());
        definitions.sort_by(|left, right| {
            left.get("name")
                .and_then(Value::as_str)
                .cmp(&right.get("name").and_then(Value::as_str))
        });

        let rendered = serde_json::to_string_pretty(&Value::Array(definitions))
            .expect("tool definitions must serialize");
        crate::tools::snapshot_fixture::assert_matches("tools-surface.json", &(rendered + "\n"));
    }

    /// Refusal guidance must point at tools that exist.
    ///
    /// `core::process::guidance` names tools as the route forward from a refusal, but it lives in a
    /// crate that cannot see the tool registry, so nothing has ever checked those names resolve. A
    /// renamed or retired tool leaves advice pointing at nothing, which is worse than a bare refusal:
    /// the caller is told a capability exists under a name it cannot call.
    ///
    /// Every alternative is required to *begin* with a registered tool name. That is already the
    /// convention — trailing detail like "(actions: run_list, run_view)" or "with `git ls-tree`"
    /// qualifies the tool rather than replacing it — and it keeps the tool name first, where a caller
    /// reads it.
    #[test]
    fn every_refusal_alternative_begins_with_a_registered_tool() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../core/src/process/guidance.rs"),
        )
        .expect("core guidance module must be readable");

        let body = source
            .split_once("pub fn tool_redirects")
            .expect("guidance must expose tool_redirects")
            .1;
        let body = body.split_once("\npub fn ").map_or(body, |(head, _)| head);

        let registered = registered_names();
        let mut offenders = Vec::new();
        for literal in body.split('"').skip(1).step_by(2) {
            // Skip the match patterns, which are program names rather than alternatives.
            if !literal.contains('_') || literal.starts_with(char::is_uppercase) {
                continue;
            }
            if !registered
                .iter()
                .any(|tool| literal == tool || literal.starts_with(&format!("{tool} ")))
            {
                offenders.push(literal.to_string());
            }
        }

        assert!(
            offenders.is_empty(),
            "refusal guidance names tools that are not registered, so the advice cannot be \
             followed: {offenders:?}"
        );
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
