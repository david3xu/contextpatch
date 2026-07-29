use serde_json::{json, Value};

mod capability;
mod files;
mod fixtures;
mod git;
mod github;
mod native;
mod process;
mod setup;

pub(crate) fn tool_definitions() -> Value {
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
    Value::Array(definitions)
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
            | "diff_preview"
            | "status_guard"
            | "file_info"
            | "list_directory"
            | "read_file_bytes"
            | "read_command_log"
            | "fixture_manifest_verify"
            | "git_remote_list"
            | "git_merge_readiness"
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
