//! `azure_containerapp_op`: the only route to Azure Container Apps writes.
//!
//! The core guard admits `az containerapp update` and `az acr build` (`AZURE_OPS_COMMANDS`), and
//! `run_guarded_command` refuses them and redirects here. This tool never forwards free-form
//! arguments: it builds the argument vector from typed fields, previews it by default, and applies
//! only with the `run azure containerapp op` confirmation, against a target the operator named in
//! the server's own launch environment:
//!
//! - `CONTEXTPATCH_AZURE_OPS_TARGETS`: comma-separated `resource-group/app` pairs `update` may touch;
//! - `CONTEXTPATCH_AZURE_OPS_REGISTRIES`: comma-separated registry names `build` may push to, and
//!   the only registries an `update` image may come from.
//!
//! Both are read from the server process environment, which a repository cannot set. With neither
//! set, every operation is refused, preview included. See `docs/execution-threat-model.md`.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use super::*;
use crate::tools::common::{
    optional_bool, optional_string, optional_string_array, optional_u64, required_string,
};

pub(crate) const CONFIRM: &str = "run azure containerapp op";
pub(crate) const TARGETS_ENV: &str = "CONTEXTPATCH_AZURE_OPS_TARGETS";
pub(crate) const REGISTRIES_ENV: &str = "CONTEXTPATCH_AZURE_OPS_REGISTRIES";
const DEFAULT_TIMEOUT_SECS: u64 = 900;
const MAX_ENV_VALUE_LEN: usize = 512;
const PLATFORMS: &[&str] = &["linux/amd64", "linux/arm64"];

/// The operator's allowlist, parsed from the server's launch environment.
#[derive(Debug, Default)]
pub(crate) struct OpsConfig {
    /// `resource-group/app` pairs.
    pub(crate) targets: BTreeSet<String>,
    /// Registry names (the part before `.azurecr.io`).
    pub(crate) registries: BTreeSet<String>,
}

impl OpsConfig {
    pub(crate) fn from_values(targets: &str, registries: &str) -> Self {
        let split = |raw: &str| {
            raw.split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
        };
        Self {
            targets: split(targets),
            registries: split(registries),
        }
    }

    fn from_environment() -> Self {
        Self::from_values(
            &std::env::var(TARGETS_ENV).unwrap_or_default(),
            &std::env::var(REGISTRIES_ENV).unwrap_or_default(),
        )
    }
}

fn refused(reason: impl std::fmt::Display) -> String {
    format!(
        "{} refused: {reason}",
        crate::tools::azure_containerapp_op::NAME
    )
}

/// An Azure resource name: letters, digits, `.`, `_`, `-`, not starting with `-`.
fn check_azure_name(field: &str, value: &str) -> Result<(), String> {
    let ok = !value.is_empty()
        && !value.starts_with('-')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(refused(format!(
            "{field} must be non-empty, must not start with `-`, and may contain only ASCII letters, digits, `.`, `_`, or `-`"
        )))
    }
}

/// `repo[/path]:tag` with lowercase repository segments.
fn check_repository_and_tag(field: &str, value: &str) -> Result<(), String> {
    let (repository, tag) = value
        .rsplit_once(':')
        .ok_or_else(|| refused(format!("{field} must be `repository:tag`")))?;
    let repository_ok = !repository.is_empty()
        && repository.starts_with(|ch: char| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        && repository.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-' | '/')
        });
    let tag_ok = !tag.is_empty()
        && tag
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
    if repository_ok && tag_ok {
        Ok(())
    } else {
        Err(refused(format!(
            "{field} {value:?} must be `repository:tag` with a lowercase repository and a tag of letters, digits, `.`, `_`, or `-`"
        )))
    }
}

/// Environment variable names that look like credentials. Secrets belong in a Container Apps
/// secret reference, never in a plain value, and a plain value would also land in tool logs.
fn looks_secret(key: &str) -> bool {
    [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "PRIVATE",
    ]
    .iter()
    .any(|marker| key.contains(marker))
        || key.ends_with("KEY")
        || key.contains("_KEY_")
}

fn check_env_key(key: &str) -> Result<(), String> {
    let ok = key.starts_with(|ch: char| ch.is_ascii_uppercase() || ch == '_')
        && key
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_');
    if !ok {
        return Err(refused(format!(
            "environment variable name {key:?} must be uppercase letters, digits, or `_`, not starting with a digit"
        )));
    }
    if looks_secret(key) {
        return Err(refused(format!(
            "environment variable {key:?} looks like a credential; set credentials through a Container Apps secret reference, not a plain value"
        )));
    }
    Ok(())
}

/// Build the `az containerapp update` argument vector, or refuse.
pub(crate) fn plan_update(
    arguments: &serde_json::Map<String, Value>,
    config: &OpsConfig,
) -> Result<Vec<String>, String> {
    let resource_group = required_string(arguments, "resource_group")?;
    let app = required_string(arguments, "app")?;
    check_azure_name("resource_group", resource_group)?;
    check_azure_name("app", app)?;
    let target = format!("{resource_group}/{app}");
    if !config.targets.contains(&target) {
        return Err(refused(format!(
            "{target} is not named in {TARGETS_ENV}; the operator must add it to the server's launch environment"
        )));
    }

    let image = optional_string(arguments, "image")?;
    let set_env = optional_string_array(arguments, "set_env")?;
    let remove_env = optional_string_array(arguments, "remove_env")?;
    if image.is_none() && set_env.is_empty() && remove_env.is_empty() {
        return Err(refused(
            "update needs at least one of image, set_env, or remove_env",
        ));
    }

    let mut args = vec![
        "containerapp".to_string(),
        "update".to_string(),
        "-n".to_string(),
        app.to_string(),
        "-g".to_string(),
        resource_group.to_string(),
    ];

    if let Some(image) = image {
        let (login_server, repository_and_tag) = image
            .split_once('/')
            .ok_or_else(|| refused("image must be `<registry>.azurecr.io/<repository>:<tag>`"))?;
        let registry = login_server.strip_suffix(".azurecr.io").ok_or_else(|| {
            refused("image must come from an Azure Container Registry (`<registry>.azurecr.io`)")
        })?;
        if !config.registries.contains(registry) {
            return Err(refused(format!(
                "registry {registry:?} is not named in {REGISTRIES_ENV}"
            )));
        }
        check_repository_and_tag("image", repository_and_tag)?;
        args.push("--image".to_string());
        args.push(image.to_string());
    }

    let mut set_keys = BTreeSet::new();
    if !set_env.is_empty() {
        args.push("--set-env-vars".to_string());
        for entry in &set_env {
            let (key, value) = entry
                .split_once('=')
                .ok_or_else(|| refused(format!("set_env entry {entry:?} must be `NAME=value`")))?;
            check_env_key(key)?;
            if value.len() > MAX_ENV_VALUE_LEN || value.chars().any(char::is_control) {
                return Err(refused(format!(
                    "set_env value for {key} must be at most {MAX_ENV_VALUE_LEN} characters with no control characters"
                )));
            }
            if !set_keys.insert(key.to_string()) {
                return Err(refused(format!("set_env names {key} more than once")));
            }
            args.push(entry.clone());
        }
    }

    if !remove_env.is_empty() {
        args.push("--remove-env-vars".to_string());
        for key in &remove_env {
            check_env_key(key)?;
            if set_keys.contains(key) {
                return Err(refused(format!("{key} is both set and removed")));
            }
            args.push(key.clone());
        }
    }

    // A short, stable result instead of the full resource document.
    args.extend([
        "--query".to_string(),
        "{revision:properties.latestRevisionName,ready:properties.latestReadyRevisionName,image:properties.template.containers[0].image,provisioning:properties.provisioningState}".to_string(),
        "-o".to_string(),
        "json".to_string(),
    ]);
    Ok(args)
}

/// Build the `az acr build` argument vector, or refuse. The build context is always the
/// repository root.
pub(crate) fn plan_build(
    arguments: &serde_json::Map<String, Value>,
    config: &OpsConfig,
) -> Result<Vec<String>, String> {
    let registry = required_string(arguments, "registry")?;
    check_azure_name("registry", registry)?;
    if !config.registries.contains(registry) {
        return Err(refused(format!(
            "registry {registry:?} is not named in {REGISTRIES_ENV}"
        )));
    }
    let image = required_string(arguments, "image")?;
    check_repository_and_tag("image", image)?;
    let dockerfile = normalize_repo_relative_path(
        crate::tools::azure_containerapp_op::NAME,
        required_string(arguments, "dockerfile")?,
    )?;
    let platform = optional_string(arguments, "platform")?.unwrap_or("linux/amd64");
    if !PLATFORMS.contains(&platform) {
        return Err(refused(format!("platform must be one of {PLATFORMS:?}")));
    }
    Ok(vec![
        "acr".to_string(),
        "build".to_string(),
        "--registry".to_string(),
        registry.to_string(),
        "--platform".to_string(),
        platform.to_string(),
        "--image".to_string(),
        image.to_string(),
        "--file".to_string(),
        dockerfile,
        ".".to_string(),
    ])
}

pub(crate) fn plan_operation(
    arguments: &serde_json::Map<String, Value>,
    config: &OpsConfig,
) -> Result<Vec<String>, String> {
    match required_string(arguments, "operation")? {
        "update" => plan_update(arguments, config),
        "build" => plan_build(arguments, config),
        other => Err(refused(format!(
            "operation {other:?} must be `update` or `build`"
        ))),
    }
}

/// The text between `stdout:` and `stderr:` in a guarded command's output.
fn guarded_stdout(output: &str) -> &str {
    let start = output
        .find("\nstdout:\n")
        .map(|index| index + "\nstdout:\n".len());
    let Some(start) = start else { return "" };
    let rest = &output[start..];
    let end = rest.find("\nstderr:").unwrap_or(rest.len());
    rest[..end].trim()
}

pub(crate) fn call_azure_containerapp_op<'a>(
    repository_root: impl Into<contextpatch_core::git::RepositoryRoot<'a>>,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let config = OpsConfig::from_environment();
    let args = plan_operation(arguments, &config)?;
    let is_build = args.first().is_some_and(|arg| arg == "acr");
    let display = format!(
        "az {}",
        args.iter()
            .map(|arg| shell_display_arg(arg))
            .collect::<Vec<_>>()
            .join(" ")
    );

    if optional_bool(arguments, "dry_run")?.unwrap_or(true) {
        return serde_json::to_string_pretty(&json!({
            "tool": crate::tools::azure_containerapp_op::NAME,
            "mode": "preview",
            "status": "previewed",
            "command": display,
            "next": format!("Re-call with dry_run=false and confirm=\"{CONFIRM}\" to apply.")
        }))
        .map_err(refused);
    }

    if required_string(arguments, "confirm")? != CONFIRM {
        return Err(refused(format!(
            "applying requires confirm=\"{CONFIRM}\"; use dry_run to preview"
        )));
    }

    let root = repository_root.into();

    // A build uploads the working tree, not a commit. Refuse uncommitted changes so an image tagged
    // for a commit is built from that commit.
    if is_build {
        let status = run_guarded_command(
            root,
            None,
            "git",
            &["status".to_string(), "--porcelain".to_string()],
            Some(60),
        )
        .map_err(refused)?;
        if !guarded_stdout(&status).is_empty() {
            return Err(refused(
                "the repository has uncommitted changes; commit or stash them so the image matches its commit",
            ));
        }
    }

    let max_timeout_secs =
        contextpatch_core::process::guarded_command::AZURE_DEPLOY_MAX_TIMEOUT_SECS;
    let timeout_secs = optional_u64(arguments, "timeout_secs")?.unwrap_or(DEFAULT_TIMEOUT_SECS);
    if timeout_secs == 0 || timeout_secs > max_timeout_secs {
        return Err(refused(format!(
            "timeout_secs must be between 1 and {max_timeout_secs}"
        )));
    }

    let initial_log = format!("Azure Container Apps operation is applying.\ncommand: {display}\n");
    let worker_authority =
        contextpatch_core::git::OwnedRepositoryRoot::retain(root).map_err(refused)?;
    let log_id = start_background_job(
        crate::tools::azure_containerapp_op::NAME,
        "az",
        &initial_log,
        move |worker_log_id| {
            let output = run_guarded_command(
                worker_authority.borrow(),
                None,
                "az",
                &args,
                Some(timeout_secs),
            )
            .map_err(|error| {
                format!(
                    "{} failed: {error}",
                    crate::tools::azure_containerapp_op::NAME
                )
            })?;
            let exit_code = extract_field(&output, "exit_code")
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or(-1);
            let timed_out = extract_field(&output, "timed_out") == Some("true");
            let status = if timed_out {
                "timed_out"
            } else if exit_code == 0 {
                "completed"
            } else {
                "failed"
            };
            Ok(BackgroundJobOutcome {
                status,
                exit_code,
                timed_out,
                log: serde_json::to_string_pretty(&json!({
                    "tool": crate::tools::azure_containerapp_op::NAME,
                    "log_id": worker_log_id,
                    "status": status,
                    "command_output": output
                }))
                .map_err(|error| {
                    format!(
                        "{} failed: {error}",
                        crate::tools::azure_containerapp_op::NAME
                    )
                })?,
            })
        },
    )?;

    serde_json::to_string_pretty(&json!({
        "tool": crate::tools::azure_containerapp_op::NAME,
        "mode": "apply",
        "status": "running",
        "command": display,
        "log_id": log_id,
        "poll_with": {
            "action": crate::tools::read_command_log::NAME,
            "arguments": {"log_id": log_id}
        }
    }))
    .map_err(refused)
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Map, Value};

    use super::{guarded_stdout, plan_operation, OpsConfig};

    fn config() -> OpsConfig {
        OpsConfig::from_values(
            "rg-datacore-platform-hosted/dcp2-runtime, rg-datacore-platform-hosted/dcp2-coordinator",
            "dcp2acr",
        )
    }

    fn arguments(value: Value) -> Map<String, Value> {
        value.as_object().expect("object").clone()
    }

    fn plan(value: Value) -> Result<Vec<String>, String> {
        plan_operation(&arguments(value), &config())
    }

    #[test]
    fn plans_an_image_and_env_update_for_a_named_target() {
        let args = plan(json!({
            "operation": "update",
            "resource_group": "rg-datacore-platform-hosted",
            "app": "dcp2-runtime",
            "image": "dcp2acr.azurecr.io/datacore-runtime:0.1.76-9f5f5cc",
            "set_env": ["DATACORE_RUNTIME_IMAGE=dcp2acr.azurecr.io/datacore-runtime:0.1.76-9f5f5cc"],
            "remove_env": ["DATACORE_REQUIRE_EXPLICIT_PROJECT_PROJECTS"]
        }))
        .expect("planned");
        assert_eq!(
            &args[..10],
            &[
                "containerapp",
                "update",
                "-n",
                "dcp2-runtime",
                "-g",
                "rg-datacore-platform-hosted",
                "--image",
                "dcp2acr.azurecr.io/datacore-runtime:0.1.76-9f5f5cc",
                "--set-env-vars",
                "DATACORE_RUNTIME_IMAGE=dcp2acr.azurecr.io/datacore-runtime:0.1.76-9f5f5cc",
            ]
        );
        assert!(args.contains(&"--remove-env-vars".to_string()));
    }

    #[test]
    fn plans_a_build_from_the_repository_root() {
        let args = plan(json!({
            "operation": "build",
            "registry": "dcp2acr",
            "image": "datacore-runtime:0.1.76-9f5f5cc",
            "dockerfile": "services/runtime/Dockerfile"
        }))
        .expect("planned");
        assert_eq!(
            args,
            [
                "acr",
                "build",
                "--registry",
                "dcp2acr",
                "--platform",
                "linux/amd64",
                "--image",
                "datacore-runtime:0.1.76-9f5f5cc",
                "--file",
                "services/runtime/Dockerfile",
                ".",
            ]
        );
    }

    #[test]
    fn refuses_a_target_the_operator_did_not_name() {
        let error = plan(json!({
            "operation": "update",
            "resource_group": "rg-datacore-platform-hosted",
            "app": "dcp2-gateway",
            "set_env": ["A=b"]
        }))
        .unwrap_err();
        assert!(
            error.contains("not named in CONTEXTPATCH_AZURE_OPS_TARGETS"),
            "{error}"
        );
    }

    #[test]
    fn refuses_everything_when_the_operator_named_nothing() {
        let error = plan_operation(
            &arguments(json!({
                "operation": "update",
                "resource_group": "rg-datacore-platform-hosted",
                "app": "dcp2-runtime",
                "set_env": ["A=b"]
            })),
            &OpsConfig::default(),
        )
        .unwrap_err();
        assert!(error.contains("not named"), "{error}");

        let error = plan_operation(
            &arguments(json!({
                "operation": "build",
                "registry": "dcp2acr",
                "image": "x:y",
                "dockerfile": "Dockerfile"
            })),
            &OpsConfig::default(),
        )
        .unwrap_err();
        assert!(error.contains("not named"), "{error}");
    }

    #[test]
    fn refuses_an_image_from_an_unnamed_or_non_acr_registry() {
        for image in [
            "otheracr.azurecr.io/datacore-runtime:1",
            "docker.io/library/alpine:3",
            "dcp2acr.azurecr.io/Datacore:1",
            "dcp2acr.azurecr.io/datacore-runtime",
        ] {
            let result = plan(json!({
                "operation": "update",
                "resource_group": "rg-datacore-platform-hosted",
                "app": "dcp2-runtime",
                "image": image
            }));
            assert!(result.is_err(), "{image} must be refused");
        }
    }

    #[test]
    fn refuses_credential_like_environment_variables() {
        for key in [
            "DATACORE_HTTP_API_KEY",
            "DATACORE_RUNTIME_BEARER_TOKEN",
            "DB_PASSWORD",
            "CLIENT_SECRET",
            "SIGNING_KEY_PATH",
        ] {
            let result = plan(json!({
                "operation": "update",
                "resource_group": "rg-datacore-platform-hosted",
                "app": "dcp2-runtime",
                "set_env": [format!("{key}=value")]
            }));
            let error = result.unwrap_err();
            assert!(error.contains("looks like a credential"), "{key}: {error}");
        }
    }

    #[test]
    fn refuses_malformed_or_ambiguous_environment_changes() {
        for value in [
            json!({"set_env": ["lowercase=1"]}),
            json!({"set_env": ["NO_EQUALS"]}),
            json!({"set_env": ["A=1", "A=2"]}),
            json!({"set_env": ["A=1"], "remove_env": ["A"]}),
            json!({"set_env": ["A=line\nbreak"]}),
            json!({}),
        ] {
            let mut object = arguments(json!({
                "operation": "update",
                "resource_group": "rg-datacore-platform-hosted",
                "app": "dcp2-runtime"
            }));
            for (key, entry) in value.as_object().expect("object") {
                object.insert(key.clone(), entry.clone());
            }
            assert!(
                plan_operation(&object, &config()).is_err(),
                "{value} must be refused"
            );
        }
    }

    #[test]
    fn refuses_unknown_operations_and_hostile_names() {
        assert!(plan(json!({"operation": "exec"})).is_err());
        assert!(plan(json!({
            "operation": "update",
            "resource_group": "--subscription",
            "app": "dcp2-runtime",
            "set_env": ["A=b"]
        }))
        .is_err());
        assert!(plan(json!({
            "operation": "build",
            "registry": "dcp2acr",
            "image": "datacore-runtime:1",
            "dockerfile": "../outside/Dockerfile"
        }))
        .is_err());
        assert!(plan(json!({
            "operation": "build",
            "registry": "dcp2acr",
            "image": "datacore-runtime:1",
            "dockerfile": "Dockerfile",
            "platform": "windows/amd64"
        }))
        .is_err());
    }

    #[test]
    fn reads_the_stdout_section_of_a_guarded_command() {
        let clean = "command: git status --porcelain\nexit_code: 0\nstdout:\n\nstderr:\n";
        let dirty =
            "command: git status --porcelain\nexit_code: 0\nstdout:\n M file.rs\n\nstderr:\n";
        assert_eq!(guarded_stdout(clean), "");
        assert_eq!(guarded_stdout(dirty), "M file.rs");
    }
}
