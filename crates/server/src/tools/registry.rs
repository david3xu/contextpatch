//! One table describing every tool, replacing five parallel ones.
//!
//! A tool's identity is currently asserted in five independent places: its schema in a
//! `schema/*.rs` module, its handler in the `dispatch` match, its reply deadline in `deadline_for`,
//! its mutation-lock membership in `serializes_repository_mutation`, and its advertised authority in
//! the `authority` classifiers. Nothing checks those agree, and adding a tool means remembering all
//! five. The observed failure is not that a tool breaks; it is that a tool is registered and
//! classified wrongly, or described in one place and not another, and every test still passes.
//!
//! A descriptor states all of it once.
//!
//! # Migration
//!
//! The registry is authoritative for the tools it contains and silent about the rest, so tools move
//! over one at a time. Each consumer — dispatch, deadlines, locking, authority, schema assembly —
//! asks the registry first and falls back to its original code path. A tool is fully migrated when
//! its descriptor exists and its old entries are deleted; between those two moments both paths are
//! live and must agree, which the recorded surface and matrix snapshots enforce.
//!
//! That is what keeps a 55-tool migration reviewable: every step is a small diff that either changes
//! the snapshots or does not, and only the final step removes the fallback.

use std::time::Duration;

use serde_json::Value;

use crate::tools::dispatch::EffectiveRepository;
use crate::tools::schema::RemoteReach;
use crate::tools::ToolSurface;

/// The uniform call shape every tool is adapted to.
///
/// Handlers themselves take three different argument shapes — most want the repository root and the
/// arguments, `capability_manifest` also needs the surface, and a few need neither — so descriptors
/// adapt with a non-capturing closure rather than forcing every handler to change signature. The
/// adaptation is visible in the table, which is where a reader looks anyway.
pub(crate) type ToolHandler = fn(
    &EffectiveRepository,
    ToolSurface,
    &serde_json::Map<String, Value>,
) -> Result<String, String>;

pub(crate) struct ToolDescriptor {
    pub(crate) name: &'static str,
    /// The advertised schema, including its `inputSchema` and description. Annotations are added
    /// centrally from `reach` and `read_only`, so a descriptor cannot advertise an authority that
    /// disagrees with the one it is classified under.
    pub(crate) schema: fn() -> Value,
    pub(crate) handler: ToolHandler,
    /// Reply deadline, or `None` for work that returns a pollable log id instead of a result.
    pub(crate) deadline: Option<Duration>,
    pub(crate) reach: RemoteReach,
    pub(crate) read_only: bool,
    /// Whether this tool takes the cooperative per-repository mutation lock.
    pub(crate) serializes_mutation: bool,
}

/// Every migrated tool. Tools absent from this table still run through the original dispatch.
static REGISTRY: &[ToolDescriptor] = &[ToolDescriptor {
    name: crate::tools::capability_manifest::NAME,
    schema: crate::tools::schema::capability_manifest_definition,
    handler: |repository, surface, arguments| {
        crate::tools::capability::call_capability_manifest(repository.root(), arguments, surface)
    },
    deadline: Some(contextpatch_core::process::deadline::READ_DEADLINE),
    reach: RemoteReach::Local,
    read_only: true,
    serializes_mutation: false,
}];

pub(crate) fn descriptor(name: &str) -> Option<&'static ToolDescriptor> {
    REGISTRY.iter().find(|entry| entry.name == name)
}

pub(crate) fn descriptors() -> &'static [ToolDescriptor] {
    REGISTRY
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A duplicate name would make dispatch depend on table order, and the first entry would silently
    /// win. Cheap to check, and impossible to see by reading a growing table.
    #[test]
    fn descriptor_names_are_unique() {
        let mut names: Vec<&str> = REGISTRY.iter().map(|entry| entry.name).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(
            names.len(),
            total,
            "registry contains a duplicate tool name"
        );
    }

    /// A descriptor whose schema advertises a different name than the descriptor claims would make
    /// the registry disagree with the surface it generates.
    #[test]
    fn each_descriptor_schema_advertises_its_own_name() {
        for entry in REGISTRY {
            let schema = (entry.schema)();
            assert_eq!(
                schema.get("name").and_then(Value::as_str),
                Some(entry.name),
                "descriptor {} generates a schema for a different tool",
                entry.name
            );
        }
    }

    /// Every migrated tool must be reachable by name, or dispatch would fall through to a path that
    /// no longer has an arm for it.
    #[test]
    fn every_descriptor_resolves_by_name() {
        for entry in REGISTRY {
            assert!(
                descriptor(entry.name).is_some(),
                "{} is in the table but does not resolve",
                entry.name
            );
        }
    }
}
