pub mod capability_manifest {
    pub const NAME: &str = "capability_manifest";
}

pub mod preflight_health {
    pub const NAME: &str = "preflight_health";
}

use std::path::Path;
use std::process::Stdio;

use contextpatch_core::git::status::status_snapshot;
use contextpatch_core::process::guarded_command::{
    redact_and_truncate_output, resolve_guarded_program,
};
use serde_json::{json, Value};

use crate::tools::{self, ToolSurface};

const PREFLIGHT_FULL_STATUS_ENTRIES: usize = 100;
const PREFLIGHT_FULL_STATUS_BYTES: usize = 64 * 1024;
const PREFLIGHT_COMPACT_STATUS_ENTRIES: usize = 20;
const PREFLIGHT_COMPACT_STATUS_BYTES: usize = 12 * 1024;

#[derive(Clone, Copy)]
enum PreflightResponseMode {
    Minimal,
    Compact,
    Full,
}

impl PreflightResponseMode {
    fn parse(arguments: &serde_json::Map<String, Value>) -> Result<Self, String> {
        match crate::tools::common::optional_string(arguments, "response_mode")
            .map_err(|error| format!("preflight_health refused: {error}"))?
            .unwrap_or("full")
        {
            "minimal" => Ok(Self::Minimal),
            "compact" => Ok(Self::Compact),
            "full" => Ok(Self::Full),
            value => Err(format!(
                "preflight_health refused: response_mode must be one of minimal, compact, full; got {value:?}"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Minimal => "minimal",
            Self::Compact => "compact",
            Self::Full => "full",
        }
    }

    fn status_sample_limits(self) -> (usize, usize) {
        match self {
            Self::Minimal => (0, 0),
            Self::Compact => (
                PREFLIGHT_COMPACT_STATUS_ENTRIES,
                PREFLIGHT_COMPACT_STATUS_BYTES,
            ),
            Self::Full => (PREFLIGHT_FULL_STATUS_ENTRIES, PREFLIGHT_FULL_STATUS_BYTES),
        }
    }
}

pub(crate) fn build_metadata() -> Value {
    json!({
        "git_sha": contextpatch_core::BUILD_GIT_SHA,
        "git_dirty": contextpatch_core::BUILD_GIT_DIRTY,
        "git_dirty_fingerprint": contextpatch_core::BUILD_GIT_DIRTY_FINGERPRINT,
        "built_at": contextpatch_core::BUILD_TIMESTAMP,
        "profile": contextpatch_core::BUILD_PROFILE,
        "note": "Compare git_sha against the checked out commit. If they differ, this server \
                 predates the source tree, and a rebuild plus reinstall may provide capabilities \
                 that appear absent here. A matching git_sha is not sufficient while git_dirty is \
                 true, because the sha still matches HEAD when the binary carries uncommitted \
                 changes; compare git_dirty_fingerprint against a rebuild from the current tree to \
                 tell whether this server was built from it."
    })
}

/// Build the whole manifest, before any projection.
///
/// Split from the tool entry point so the projection in `call_capability_manifest` cannot drift from
/// the document: there is exactly one place the content is defined, and sections are selected from it
/// rather than assembled a second time.
fn full_manifest(root: &Path, surface: ToolSurface) -> Value {
    json!({
        "server": "contextpatch",
        "version": contextpatch_core::VERSION,
        // Provenance, not decoration. `version` is the crate version and does not move between
        // rebuilds, so a client that finds a tool absent cannot tell whether the capability does not
        // exist or whether this binary predates it. The natural inference is the wrong one, and the
        // failure is silent and confident. These fields make staleness detectable in one call.
        "build": build_metadata(),
        "tool_surface": {
            "mode": surface.as_str(),
            "public_tool_names": crate::tools::schema::public_tool_names(surface),
            "action_names": crate::tools::schema::wrapper_action_names(surface),
            "action_definitions": crate::tools::schema::internal_action_definitions(),
            "note": "Full mode advertises each action as a direct MCP tool. Project mode advertises \
                     only project_execute while preserving the same internal actions and safety \
                     policy."
        },
        "repo_root": root.display().to_string(),
        "scratch": {
            "token": contextpatch_core::fs::scratch::SCRATCH_TOKEN,
            "root": contextpatch_core::fs::scratch::scratch_root(root).display().to_string(),
            "note": "Write byproducts here instead of into the repository. The token expands inside \
                     data and output arguments, so `--output-dir={scratch}/run-1` works, and paths \
                     that climb out of it are refused. It cannot be used as the Python script path; \
                     use artifact_python_run for scratch-hosted code."
        },
        "deadlines_seconds": {
            "read": contextpatch_core::process::deadline::READ_DEADLINE.as_secs(),
            "write": contextpatch_core::process::deadline::WRITE_DEADLINE.as_secs(),
            "git": contextpatch_core::process::deadline::GIT_DEADLINE.as_secs(),
            "github": contextpatch_core::process::deadline::GIT_DEADLINE.as_secs(),
            "git_subprocess_timeout": contextpatch_core::process::GIT_SUBPROCESS_TIMEOUT.as_secs(),
            "max_active_workers": contextpatch_core::process::deadline::MAX_ACTIVE_WORKERS,
            "note": "A call that exceeds its deadline returns a refusal naming the operation and how \
                     to establish current state, rather than withholding a reply. The work is not \
                     cancelled, so the outcome is reported as unknown rather than as failed. Direct \
                     Git subprocesses have a shorter hard timeout and are terminated before the \
                     reply deadline; Unix process groups also terminate descendants. Once the \
                     worker cap is reached, new bounded calls are refused before they start."
        },
        "transport": {
            "concurrent_tool_calls": true,
            "max_active_tool_calls": crate::server::MAX_ACTIVE_TOOL_CALLS,
            "responses_may_arrive_out_of_order": true,
            "correlate_by": "JSON-RPC id",
            "background_jobs": {
                "tools": [
                    tools::task_image_python_run::NAME,
                    tools::harbor_run_start::NAME,
                    tools::validation_profile_run::NAME
                ],
                "max_active": crate::tools::process::MAX_ACTIVE_BACKGROUND_JOBS,
                "poll_with": tools::read_command_log::NAME,
                "same_server_required": true,
                "restart_active_status": "unknown"
            }
        },
        "mutation_coordination": {
            "mode": "cooperative_contextpatch_locking",
            "repository_serialized": true,
            "sha_guarded_file_edits": true,
            "note": "ContextPatch mutations for one repository are serialized across server \
                     processes, and exact replacement/hash writes also lock their target while \
                     checking it. External programs that ignore these advisory locks remain outside \
                     this guarantee."
        },
        "write_receipts": {
            "outcomes": ["applied", "refused", "unknown", "interrupted"],
            "note": "Applied means the operation returned success; refused means it failed with \
                     unchanged observed state; unknown means it failed after state changed or could \
                     not be read; interrupted means no settlement record exists."
        },
        "file_tools": {
            "read_range": true,
            "read_write_receipts": true,
            "diff_preview": true,
            "replace_exact": true,
            "bulk_replace_exact": true,
            "write_new_file": true,
            "write_new_file_base64": true,
            "write_existing_file_exact_hash": true,
            "file_info": true,
            "set_file_executable": true,
            "list_directory": true,
            "read_file_bytes": true,
            "artifact_write_text": true,
            "artifact_write_base64": true,
            "artifact_delete_exact": true,
            "create_directory": true,
            "status_guard": true,
            "read_command_log": true,
            "artifact_python_run": true,
            "task_image_python_run": true,
            "harbor_run_start": true,
            "image_cleanliness_check_run": true,
            "docker_image_inspect": true,
            "setup_profile_run": true,
            "native_build_run": true,
            "native_device_run": true,
            "git_commit_exact": true,
            "git_commit_scoped": true,
            "git_commit_prefix": true,
            "git_stage_exact": true,
            "git_staged_scope_check": true,
            "git_restore_exact": true,
            "move_tracked": true,
            "delete_guarded": true,
            "delete_untracked_exact": true,
            "delete_generated_prefix": true,
            "git_remote_list": true,
            "git_remote_check": true,
            "git_branch_prepare": true,
            "git_merge_readiness": true,
            "git_push_exact": true,
            "github_pr_run": true,
            "github_fork_prepare": true,
            "bulk_write_new_files_base64": true,
            "fixture_generator_run": true,
            "base_image_check_run": true,
            "fixture_manifest_verify": true,
            "fixture_manifest_refresh": true
        },
        "git_workflows": {
            "local_commit_exact_paths": true,
            "local_commit_scoped_paths": true,
            "local_commit_prefix_paths": true,
            "stage_exact_paths_without_commit": true,
            "staged_scope_check": true,
            "restore_exact_paths": true,
            "move_clean_tracked_file": true,
            "delete_clean_tracked_file_exact_hash": true,
            "delete_untracked_exact_paths": true,
            "delete_generated_prefix_paths": true,
            "remote_list": true,
            "remote_check": true,
            "branch_prepare": true,
            "merge_readiness": true,
            "push_exact_commit": true,
            "reset_checkout_clean_stash": false,
            "commit_attribution": "Do not include Claude, Anthropic, AI, or assistant attribution in commit messages. Use the repository user's authorship and project-owned commit message only.",
            "guards": [
                "requires exact complete dirty-path set",
                "scoped commits require requested dirty paths and a clean index before staging",
                "prefix commits expand explicit prefixes to exact dirty paths before staging",
                "stage-only workflow requires exact dirty paths and clean index",
                "staged scope checker is read-only and reports disallowed or missing staged paths",
                "defaults to dry_run",
                "requires confirm literal for mutation",
                "restores only explicit dirty paths from HEAD",
                "tracked moves require a clean source and index, absent destination, dry-run, and exact confirmation",
                "tracked deletion requires a clean regular file, exact current SHA-256, dry-run, and exact confirmation",
                "generated cleanup expands explicit prefixes to ignored/untracked files and empty dirs",
                "stages only explicit paths",
                "creates at most one local commit",
                "fetches only explicit remote branch",
                "branch preparation requires clean worktree and validates required files",
                "merge readiness is read-only and reports changed-on-both-sides candidates",
                "pushes only expected HEAD to matching remote branch",
                "never force pushes"
            ],
            "examples": [
                {
                    "tool": "git_restore_exact",
                    "description": "Restore generated dirty tracked paths before making an exact scoped commit.",
                    "arguments": {
                        "paths": ["data/processed/example.json"],
                        "dry_run": true
                    }
                }
            ]
        },
        "github_workflows": {
            "available": true,
            "tools": [tools::github_pr_run::NAME, tools::github_fork_prepare::NAME],
            "actions": ["auth_status", "pr_view", "pr_comments", "pr_checks", "workflow_runs_for_commit", "workflow_run_view", "workflow_job_log", "workflow_run_rerun_failed", "pr_create"],
            "repository_targeting": "PR and Actions actions accept a validated OWNER/REPO selector for fork/upstream workflows.",
            "fork_actions": ["dry_run fork plan", "confirmed gh repo fork"],
            "guards": [
                "uses gh with explicit argv only",
                "repository selectors use validated OWNER/REPO syntax",
                "PR checks return structured names, states, workflows, and links",
                "PR comments can be filtered locally by a bounded marker and returned newest-first",
                "workflow run discovery requires a full commit SHA and bounded result limit",
                "workflow run metadata and bounded job logs are read-only evidence",
                "failed-job workflow reruns default to dry_run and require an exact confirmation literal",
                "pr_create defaults to dry_run",
                "pr_create requires confirm literal when mutating",
                "fork prepare defaults to dry_run",
                "fork prepare requires confirm literal when mutating",
                "no arbitrary gh passthrough"
            ]
        },
        "process_execution": {
            "available": true,
            "mode": "allowlisted_no_shell",
            "default_timeout_secs": 120,
            "default_max_timeout_secs": 600,
            "harbor_run_max_timeout_secs": 3600,
            "programs": {
                "git": ["status", "diff", "log", "show", "rev-parse", "ls-tree"],
                "cargo": ["check", "test", "build", "clippy"],
                "bun": ["run", "test"],
                "npm": ["run", "test"],
                "pnpm": ["run", "test"],
                "python": ["repo-relative .py script"],
                "python3": ["repo-relative .py script"],
                "pytest": ["validation invocation"],
                "bash": ["references/check-base-image.sh", "references/check-base-image.sh task"],
                "rg": ["search"]
            },
            "typed_workflows": {
                "read_command_log": "Reads command logs with max_chars and offset paging.",
                "harbor_run_start": "Starts one typed Harbor run asynchronously, returns a stable log_id, and reports running/completed/failed/timed_out/unknown status through read_command_log.",
                "validation_profile_run": "Starts a named validation sequence asynchronously and adds structured Dynamo/Harbor oracle/nop reward summaries for the task profile.",
                "artifact_python_run": "Runs artifact-root Python scratch scripts without placing temporary analysis code in the repository.",
                "task_image_python_run": "Plans or asynchronously builds task/environment/Dockerfile and runs one repository-relative Python script in the image with no network, a read-only repository mount, dropped capabilities, bounded temporary storage, and explicit confirmation.",
                "fixture_generator_run": "Runs repo-relative Python generators after planning, confirmation, and declared-output verification.",
                "base_image_check_run": "Runs only references/check-base-image.sh, optionally with the exact task project argument; arbitrary shell scripts remain unsupported.",
                "bulk_write_new_files_base64": "Imports many create-only binary/text fixture files with per-file and total size bounds.",
                "write_existing_file_exact_hash": "Overwrites an existing file only when the current SHA-256 matches.",
                "file_info": "Reports metadata, mode, symlink status, file size, SHA-256, and UTF-8 line counts for one path or up to 64 paths.",
                "set_file_executable": "Changes only executable bits on one regular non-symlink file after a hash/mode dry run and exact confirmation.",
                "list_directory": "Lists one repository directory with optional depth- and entry-bounded recursion without following symlink directories.",
                "read_file_bytes": "Reads bounded binary file ranges as hex or base64 with SHA-256.",
                "fixture_manifest_verify": "Verifies exact fixture file sets and SHA-256 digests.",
                "fixture_manifest_refresh": "Regenerates fixture manifests with dry-run, confirmation, and existing-manifest hash guard.",
                "image_cleanliness_check_run": "Runs a narrow Docker image file-name scan without exposing generic docker.",
                "docker_image_inspect": "Runs docker image inspect for one validated image reference without exposing generic docker."
            },
            "validation_profiles": ["repo-basic", "rust-workspace", "datacore-vscode", "datacore-m6-vscode", "dynamo-harbor-task"],
            "guards": [
                "repo-root-confined cwd",
                "no shell interpolation",
                "allowlisted program and subcommand",
                "timeout bounded",
                "secret-like output redaction",
                "stdout/stderr truncation",
                "command/cwd/exit-code/duration metadata"
                ,"task image repository mount is read-only and network is disabled"
                ,"Harbor, task-image, and validation-profile background jobs share a two-job cap and become unknown after server restart"
                ,"direct harbor run is refused by run_guarded_command; use harbor_run_start"
            ],
            "not_supported": [
                "arbitrary shell",
                "force push/pull/reset/checkout/stash/clean",
                "ungated or automatic commits",
                "path traversal outside repo root",
                "secret-printing environment inspection"
            ]
        },
        "setup_profiles": {
            "mode": "declarative_profile_actions",
            "profiles": {
                "node-capacitor-shell": {
                    "actions": [
                        "install_capacitor_dependencies",
                        "install_capacitor_filesystem",
                        "cap_init",
                        "cap_add_ios",
                        "cap_add_android",
                        "cap_sync",
                        "ios_pod_install"
                    ],
                    "external_mutator": true,
                    "caller_supplies_raw_command": false,
                    "ios_pod_install_requirement": "requires Podfile in cwd; Swift Package Manager based Capacitor projects do not need this action",
                    "supported_package_managers": ["npm", "pnpm"],
                    "package_manager_selection": "uses pnpm when pnpm-lock.yaml exists in cwd; otherwise npm",
                    "dependency_layout": {
                        "dependencies": ["@capacitor/core", "@capacitor/ios", "@capacitor/android"],
                        "optional_runtime_dependencies_by_action": {
                            "install_capacitor_filesystem": ["@capacitor/filesystem"]
                        },
                        "dev_dependencies": ["@capacitor/cli"]
                    },
                    "mutation_enabled": true,
                    "required_confirm_for_mutation": "run setup profile"
                }
            },
            "examples": [
                {
                    "tool": "setup_profile_run",
                    "description": "Initialize Capacitor shell files without raw npx/cap commands.",
                    "arguments": {
                        "profile": "node-capacitor-shell",
                        "action": "cap_init",
                        "params": {
                            "app_id": "com.example.app",
                            "app_name": "Example",
                            "web_dir": "dist"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "setup_profile_run",
                    "description": "Install the Capacitor Filesystem plugin needed to pass fetched local files into native plugins.",
                    "arguments": {
                        "profile": "node-capacitor-shell",
                        "action": "install_capacitor_filesystem",
                        "dry_run": true
                    }
                },
                {
                    "tool": "setup_profile_run",
                    "description": "Install iOS CocoaPods dependencies without raw pod access.",
                    "arguments": {
                        "profile": "node-capacitor-shell",
                        "action": "ios_pod_install",
                        "cwd": "ios/App",
                        "dry_run": true
                    }
                }
            ],
            "guards": [
                "profile derives exact command plan",
                "package-manager commands stay inside setup profiles, not run_guarded_command",
                "typed action parameters only",
                "repo-root-confined cwd",
                "dry-run first",
                "mutation requires clean worktree and exact confirmation",
                "mutation reports before/after Git status and changed paths"
            ]
        },
        "native_build": {
            "mode": "typed_native_build_actions",
            "actions": {
                "ios_build": {
                    "program": "xcodebuild",
                    "caller_supplies_raw_command": false,
                    "supports_repo_relative_derived_data_path": true
                },
                "ios_test": {
                    "program": "xcodebuild",
                    "caller_supplies_raw_command": false,
                    "supports_repo_relative_derived_data_path": true
                },
                "android_assemble_debug": { "program": "./gradlew", "caller_supplies_raw_command": false },
                "android_unit_test": { "program": "./gradlew", "caller_supplies_raw_command": false }
            },
            "examples": [
                {
                    "tool": "native_build_run",
                    "description": "Plan an iOS simulator build.",
                    "arguments": {
                        "action": "ios_build",
                        "params": {
                            "workspace": "ios/App/App.xcworkspace",
                            "scheme": "App",
                            "derived_data_path": ".contextpatch-derived-data"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_build_run",
                    "description": "Plan an Android debug build using the repository Gradle wrapper.",
                    "arguments": {
                        "action": "android_assemble_debug",
                        "params": {},
                        "dry_run": true
                    }
                }
            ],
            "guards": [
                "typed action parameters only",
                "repo-root-confined cwd",
                "repo-relative Gradle wrapper is canonicalized under the repository",
                "source Git status is checked before and after execution",
                "large output is available through read_command_log"
            ]
        },
        "native_device": {
            "mode": "typed_native_device_actions",
            "actions": [
                "ios_list_simulators",
                "ios_create_simulator",
                "ios_boot_simulator",
                "ios_cap_run",
                "ios_install_app",
                "ios_launch_app",
                "ios_read_logs",
                "android_list_devices",
                "android_install_app",
                "android_launch_app",
                "android_read_logcat"
            ],
            "required_confirm_for_device_state": "run native device",
            "examples": [
                {
                    "tool": "native_device_run",
                    "description": "Plan an iOS app install from repo-relative Xcode DerivedData output.",
                    "arguments": {
                        "action": "ios_install_app",
                        "params": {
                            "device": "booted",
                            "app_path": ".contextpatch-derived-data/Build/Products/Debug-iphonesimulator/App.app"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_device_run",
                    "description": "Plan an iOS app launch on the booted simulator.",
                    "arguments": {
                        "action": "ios_launch_app",
                        "params": {
                            "device": "booted",
                            "app_id": "com.example.app"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_device_run",
                    "description": "Create a named iOS simulator when runtimes exist but no devices are configured.",
                    "arguments": {
                        "action": "ios_create_simulator",
                        "params": {
                            "name": "ContextPatch iPhone",
                            "device_type": "iPhone 16",
                            "runtime": "iOS 26.4"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_device_run",
                    "description": "Run a synced Capacitor iOS app on a specific simulator target without raw npx/pnpm/xcrun commands.",
                    "arguments": {
                        "action": "ios_cap_run",
                        "params": {
                            "target": "00000000-0000-0000-0000-000000000000"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_device_run",
                    "description": "Read a timed iOS simulator log stream without starting an unbounded stream.",
                    "arguments": {
                        "action": "ios_read_logs",
                        "params": {
                            "device": "booted",
                            "duration": "5"
                        },
                        "dry_run": true
                    }
                },
                {
                    "tool": "native_device_run",
                    "description": "Read recent Android logcat output without changing device state.",
                    "arguments": {
                        "action": "android_read_logcat",
                        "params": {
                            "serial": "emulator-5554",
                            "lines": 200
                        },
                        "dry_run": true
                    }
                }
            ],
            "guards": [
                "typed action parameters only",
                "no arbitrary xcrun, adb, or package-manager exec",
                "device-state changes require dry_run=false plus exact confirmation",
                "bounded timeout and redacted log output",
                "repository source files are not mutated"
            ]
        }
    })
}

fn projected_build(document: &Value) -> Result<Value, String> {
    let build = document
        .get("build")
        .cloned()
        .ok_or_else(|| "capability_manifest refused: build metadata is missing".to_string())?;
    if !build.get("git_sha").is_some_and(Value::is_string) {
        return Err("capability_manifest refused: build.git_sha is missing".to_string());
    }
    Ok(build)
}

/// Keys a caller may request individually, so orientation does not cost the whole document.
///
/// The full manifest runs to hundreds of lines, which made the discovery tool expensive enough to avoid
/// calling, a perverse outcome for the one tool whose job is orientation. Absent arguments still return
/// the complete document, so nothing that already depends on it changes.
pub(crate) const PROJECTABLE_SECTIONS: &[&str] = &[
    "build",
    "tool_surface",
    "scratch",
    "deadlines_seconds",
    "mutation_coordination",
    "write_receipts",
    "file_tools",
    "git_workflows",
    "github_workflows",
    "process_execution",
    "setup_profiles",
    "native_build",
    "native_device",
];

pub(crate) fn call_capability_manifest(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
    surface: ToolSurface,
) -> Result<String, String> {
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("capability_manifest refused: {error}"))?;
    let document = full_manifest(&root, surface);

    let names_only = crate::tools::common::optional_bool(arguments, "names_only")
        .map_err(|error| format!("capability_manifest refused: {error}"))?
        .unwrap_or(false);
    let section = crate::tools::common::optional_string(arguments, "section")
        .map_err(|error| format!("capability_manifest refused: {error}"))?;

    let projected = match (names_only, section) {
        (true, _) => {
            let build = projected_build(&document)?;
            json!({
                "server": "contextpatch",
                "build": build,
                "tool_surface": surface.as_str(),
                "tool_names": crate::tools::schema::public_tool_names(surface),
                "action_names": crate::tools::schema::wrapper_action_names(surface)
            })
        }
        (false, Some(section)) => {
            if !PROJECTABLE_SECTIONS.contains(&section) {
                return Err(format!(
                    "capability_manifest refused: unknown section {section:?}; valid choices: {}",
                    PROJECTABLE_SECTIONS.join(", ")
                ));
            }
            let build = projected_build(&document)?;
            let selected = document.get(section).cloned().ok_or_else(|| {
                format!("capability_manifest refused: section {section:?} is unavailable")
            })?;
            let mut projection = serde_json::Map::new();
            projection.insert(
                "server".to_string(),
                Value::String("contextpatch".to_string()),
            );
            projection.insert("build".to_string(), build);
            projection.insert("section".to_string(), Value::String(section.to_string()));
            if section != "build" {
                projection.insert(section.to_string(), selected);
            }
            Value::Object(projection)
        }
        (false, None) => document,
    };

    serde_json::to_string_pretty(&projected)
        .map_err(|error| format!("capability_manifest refused: {error}"))
}

pub(crate) fn call_preflight_health(
    repo_root: &Path,
    arguments: &serde_json::Map<String, Value>,
) -> Result<String, String> {
    let response_mode = PreflightResponseMode::parse(arguments)?;
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("preflight_health refused: {error}"))?;
    let (max_status_entries, max_status_bytes) = response_mode.status_sample_limits();
    let git_status = match status_snapshot(&root, max_status_entries, max_status_bytes) {
        Ok(snapshot) => json!({
            "available": true,
            "clean": snapshot.clean,
            "summary": snapshot.summary(),
            "change_count": snapshot.change_count,
            "sampled_changes": snapshot.sampled_changes,
            "sample_truncated": snapshot.sample_truncated,
            "sample_limits": {
                "max_entries": max_status_entries,
                "max_bytes": max_status_bytes
            }
        }),
        Err(error) => {
            let (summary, _) = redact_and_truncate_output(&error.to_string(), 4_096);
            json!({
                "available": false,
                "clean": false,
                "summary": summary,
                "change_count": null,
                "sampled_changes": [],
                "sample_truncated": false,
                "sample_limits": {
                    "max_entries": max_status_entries,
                    "max_bytes": max_status_bytes
                }
            })
        }
    };
    let document = json!({
        "server": "contextpatch",
        "version": contextpatch_core::VERSION,
        "build": build_metadata(),
        "response_mode": response_mode.as_str(),
        "repo_root": root.display().to_string(),
        "repository": git_status,
        "guarded_process_execution": {
            "available": true,
            "mode": "allowlisted_no_shell",
            "default_timeout_secs": 120,
            "max_timeout_secs": 600,
            "harbor_run_max_timeout_secs": 3600,
            "configured_validation_paths_env": "CONTEXTPATCH_VALIDATION_PATHS"
        },
        "tools": {
            "git": executable_available("git"),
            "cargo": executable_available("cargo"),
            "bun": executable_available("bun"),
            "npm": executable_available("npm"),
            "pnpm": executable_available("pnpm"),
            "python": executable_available("python"),
            "python3": executable_available("python3"),
            "pytest": executable_available("pytest"),
            "harbor": executable_available("harbor"),
            "bash": executable_available("bash"),
            "rg": executable_available("rg")
        },
        "validation_tools": {
            "git": executable_available("git"),
            "cargo": executable_available("cargo"),
            "bun": executable_available("bun"),
            "npm": executable_available("npm"),
            "pnpm": executable_available("pnpm"),
            "python": executable_available("python"),
            "python3": executable_available("python3"),
            "pytest": executable_available("pytest"),
            "harbor": executable_available("harbor"),
            "bash": executable_available("bash"),
            "rg": executable_available("rg"),
            "base_image_check": base_image_check_available(&root)
        },
        "validation_profiles": {
            "repo-basic": {
                "available": executable_is_available("git"),
                "required_tools": ["git"]
            },
            "rust-workspace": {
                "available": executable_is_available("git") && executable_is_available("cargo"),
                "required_tools": ["git", "cargo"]
            },
            "datacore-vscode": {
                "available": executable_is_available("git") && executable_is_available("bun") && executable_is_available("rg"),
                "required_tools": ["git", "bun", "rg"]
            },
            "datacore-m6-vscode": {
                "available": executable_is_available("git") && executable_is_available("bun") && executable_is_available("rg"),
                "required_tools": ["git", "bun", "rg"]
            },
            "dynamo-harbor-task": {
                "available": executable_is_available("git") && executable_is_available("harbor"),
                "required_tools": ["git", "harbor"],
                "fallback_when_unavailable": "If harbor is unavailable, verify task-local oracle/verifier logic directly but do not claim Harbor scoring succeeded."
            }
        },
        "setup_profiles": {
            "node-capacitor-shell": {
                "available": executable_is_available("npm") || executable_is_available("pnpm"),
                "package_managers": {
                    "npm": executable_available("npm"),
                    "pnpm": executable_available("pnpm")
                },
                "optional_tools": {
                    "pod": executable_available("pod")
                },
                "mutation_enabled": true
            }
        },
        "native_build": {
            "available": xcodebuild_is_available() || gradlew_available(&root),
            "required_tools": {
                "xcodebuild": xcodebuild_available(),
                "xcode_select": xcode_select_available(),
                "gradlew": gradlew_available(&root),
                "swift": executable_available("swift"),
                "kotlinc": executable_available("kotlinc")
            }
        },
        "native_device": {
            "available": executable_is_available("xcrun") || executable_is_available("adb"),
            "required_tools": {
                "xcrun": executable_available("xcrun"),
                "adb": executable_available("adb")
            }
        }
    });
    let projected = match response_mode {
        PreflightResponseMode::Minimal => minimal_preflight(&document),
        PreflightResponseMode::Compact => compact_preflight(&document),
        PreflightResponseMode::Full => document,
    };

    serde_json::to_string_pretty(&projected)
        .map_err(|error| format!("preflight_health refused: {error}"))
}

fn compact_preflight(document: &Value) -> Value {
    json!({
        "server": document["server"].clone(),
        "version": document["version"].clone(),
        "build": document["build"].clone(),
        "response_mode": "compact",
        "repo_root": document["repo_root"].clone(),
        "repository": document["repository"].clone(),
        "guarded_process_execution": document["guarded_process_execution"].clone(),
        "tools": availability_projection(&document["tools"]),
        "validation_tools": availability_projection(&document["validation_tools"]),
        "validation_profiles": availability_projection(&document["validation_profiles"]),
        "setup_profiles": availability_projection(&document["setup_profiles"]),
        "native_build": compact_native_status(&document["native_build"]),
        "native_device": compact_native_status(&document["native_device"])
    })
}

fn minimal_preflight(document: &Value) -> Value {
    json!({
        "server": document["server"].clone(),
        "version": document["version"].clone(),
        "build": document["build"].clone(),
        "response_mode": "minimal",
        "repo_root": document["repo_root"].clone(),
        "repository": document["repository"].clone(),
        "guarded_process_execution": {
            "available": document["guarded_process_execution"]["available"].clone()
        },
        "validation_tools": availability_summary(&document["validation_tools"]),
        "validation_profiles": availability_summary(&document["validation_profiles"]),
        "setup_profiles": availability_summary(&document["setup_profiles"]),
        "native_build_available": document["native_build"]["available"].clone(),
        "native_device_available": document["native_device"]["available"].clone()
    })
}

fn compact_native_status(status: &Value) -> Value {
    json!({
        "available": status["available"].clone(),
        "required_tools": availability_projection(&status["required_tools"])
    })
}

fn availability_projection(statuses: &Value) -> Value {
    let Some(statuses) = statuses.as_object() else {
        return Value::Object(serde_json::Map::new());
    };
    Value::Object(
        statuses
            .iter()
            .map(|(name, status)| (name.clone(), availability_flag(status)))
            .collect(),
    )
}

fn availability_summary(statuses: &Value) -> Value {
    let projected = availability_projection(statuses);
    let Some(projected) = projected.as_object() else {
        return json!({ "total": 0, "available": 0, "unavailable": [] });
    };
    let unavailable = projected
        .iter()
        .filter_map(|(name, available)| {
            (!available.as_bool().unwrap_or(false)).then_some(name.clone())
        })
        .collect::<Vec<_>>();
    json!({
        "total": projected.len(),
        "available": projected.len().saturating_sub(unavailable.len()),
        "unavailable": unavailable
    })
}

fn availability_flag(status: &Value) -> Value {
    status
        .as_bool()
        .map(Value::Bool)
        .or_else(|| {
            status
                .get("available")
                .and_then(Value::as_bool)
                .map(Value::Bool)
        })
        .unwrap_or(Value::Bool(false))
}

fn executable_available(program: &str) -> Value {
    let resolved = resolve_guarded_program(program);
    let executable = resolved.as_deref().unwrap_or_else(|| Path::new(program));
    let output = std::process::Command::new(executable)
        .arg("--version")
        .output();
    match output {
        Ok(output) => json!({
            "available": output.status.success(),
            "exit_code": output.status.code().unwrap_or(-1),
            "resolved_path": resolved.map(|path| path.display().to_string())
        }),
        Err(error) => json!({
            "available": false,
            "resolved_path": resolved.map(|path| path.display().to_string()),
            "error": error.to_string()
        }),
    }
}

fn xcodebuild_available() -> Value {
    let output = std::process::Command::new("xcodebuild")
        .arg("-version")
        .output();
    match output {
        Ok(output) => json!({
            "available": output.status.success(),
            "exit_code": output.status.code().unwrap_or(-1),
            "probe": "xcodebuild -version",
            "version": String::from_utf8_lossy(&output.stdout).trim(),
            "diagnostic": xcodebuild_diagnostic(&output)
        }),
        Err(error) => json!({
            "available": false,
            "probe": "xcodebuild -version",
            "error": error.to_string()
        }),
    }
}

fn xcodebuild_is_available() -> bool {
    std::process::Command::new("xcodebuild")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn xcodebuild_diagnostic(output: &std::process::Output) -> Value {
    if output.status.success() {
        return Value::Null;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}\n{stderr}");
    let lower = combined.to_ascii_lowercase();
    if lower.contains("license") {
        return json!(
            "xcodebuild is installed but requires accepting the Xcode license outside contextpatch"
        );
    }
    if lower.contains("requires xcode") || lower.contains("full xcode") {
        return json!("xcodebuild is present but full Xcode may not be installed or selected");
    }
    Value::Null
}

fn xcode_select_available() -> Value {
    let output = std::process::Command::new("xcode-select")
        .arg("-p")
        .output();
    match output {
        Ok(output) => json!({
            "available": output.status.success(),
            "exit_code": output.status.code().unwrap_or(-1),
            "path": String::from_utf8_lossy(&output.stdout).trim()
        }),
        Err(error) => json!({
            "available": false,
            "error": error.to_string()
        }),
    }
}

fn executable_is_available(program: &str) -> bool {
    let resolved = resolve_guarded_program(program);
    let executable = resolved.as_deref().unwrap_or_else(|| Path::new(program));
    std::process::Command::new(executable)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn base_image_check_available(root: &Path) -> Value {
    let script = root.join("references/check-base-image.sh");
    json!({
        "available": executable_is_available("bash") && script.is_file(),
        "script": "references/check-base-image.sh",
        "bash": executable_available("bash"),
        "script_present": script.is_file()
    })
}

fn gradlew_available(root: &Path) -> bool {
    root.join("gradlew").is_file()
}
