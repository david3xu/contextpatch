use serde_json::Value;

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
    Value::Array(definitions)
}
