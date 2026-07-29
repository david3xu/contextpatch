pub mod capability_manifest {
    pub const NAME: &str = "capability_manifest";
}

pub mod preflight_health {
    pub const NAME: &str = "preflight_health";
}

use std::path::Path;
use std::process::Stdio;

use contextpatch_core::git::status::status_summary;
use serde_json::{json, Value};

use crate::tools;

pub(crate) fn call_capability_manifest(repo_root: &Path) -> Result<String, String> {
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("capability_manifest refused: {error}"))?;
    serde_json::to_string_pretty(&json!({
        "server": "contextpatch",
        "version": contextpatch_core::VERSION,
        "repo_root": root.display().to_string(),
        "file_tools": {
            "read_range": true,
            "diff_preview": true,
            "replace_exact": true,
            "write_new_file": true,
            "write_new_file_base64": true,
            "write_existing_file_exact_hash": true,
            "artifact_write_text": true,
            "artifact_write_base64": true,
            "create_directory": true,
            "status_guard": true,
            "read_command_log": true,
            "setup_profile_run": true,
            "native_build_run": true,
            "native_device_run": true,
            "git_commit_exact": true,
            "git_commit_scoped": true,
            "git_restore_exact": true,
            "delete_untracked_exact": true,
            "git_remote_list": true,
            "git_remote_check": true,
            "git_branch_prepare": true,
            "git_merge_readiness": true,
            "git_push_exact": true,
            "github_pr_run": true,
            "github_fork_prepare": true
            ,
            "bulk_write_new_files_base64": true,
            "fixture_generator_run": true,
            "base_image_check_run": true
            ,
            "fixture_manifest_verify": true,
            "fixture_manifest_refresh": true
        },
        "git_workflows": {
            "local_commit_exact_paths": true,
            "local_commit_scoped_paths": true,
            "restore_exact_paths": true,
            "delete_untracked_exact_paths": true,
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
                "defaults to dry_run",
                "requires confirm literal for mutation",
                "restores only explicit dirty paths from HEAD",
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
            "actions": ["auth_status", "pr_view", "pr_checks", "pr_create"],
            "fork_actions": ["dry_run fork plan", "confirmed gh repo fork"],
            "guards": [
                "uses gh with explicit argv only",
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
            "programs": {
                "git": ["status", "diff", "log", "show", "rev-parse", "ls-tree"],
                "cargo": ["check", "test", "build", "clippy"],
                "bun": ["run", "test"],
                "npm": ["run", "test"],
                "pnpm": ["run", "test"],
                "python": ["repo-relative .py script"],
                "python3": ["repo-relative .py script"],
                "pytest": ["validation invocation"],
                "harbor": ["run"],
                "bash": ["references/check-base-image.sh", "references/check-base-image.sh task"],
                "rg": ["search"]
            },
            "typed_workflows": {
                "fixture_generator_run": "Runs repo-relative Python generators after planning, confirmation, and declared-output verification.",
                "base_image_check_run": "Runs only references/check-base-image.sh, optionally with the exact task project argument; arbitrary shell scripts remain unsupported.",
                "bulk_write_new_files_base64": "Imports many create-only binary/text fixture files with per-file and total size bounds.",
                "write_existing_file_exact_hash": "Overwrites an existing file only when the current SHA-256 matches.",
                "fixture_manifest_verify": "Verifies exact fixture file sets and SHA-256 digests.",
                "fixture_manifest_refresh": "Regenerates fixture manifests with dry-run, confirmation, and existing-manifest hash guard."
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
            ],
            "not_supported": [
                "arbitrary shell",
                "force push/pull/reset/checkout/stash/clean",
                "ungated or automatic commits",
                "path traversal outside repo root",
                "secret-printing environment inspection"
            ]
        }
        ,
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
    }))
    .map_err(|error| format!("capability_manifest refused: {error}"))
}

pub(crate) fn call_preflight_health(repo_root: &Path) -> Result<String, String> {
    let root = repo_root
        .canonicalize()
        .map_err(|error| format!("preflight_health refused: {error}"))?;
    let git_status = match status_summary(&root) {
        Ok(summary) => json!({ "clean": true, "summary": summary }),
        Err(error) => json!({ "clean": false, "summary": error.to_string() }),
    };
    serde_json::to_string_pretty(&json!({
        "server": "contextpatch",
        "version": contextpatch_core::VERSION,
        "repo_root": root.display().to_string(),
        "repository": git_status,
        "guarded_process_execution": {
            "available": true,
            "mode": "allowlisted_no_shell",
            "default_timeout_secs": 120,
            "max_timeout_secs": 600
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
    }))
    .map_err(|error| format!("preflight_health refused: {error}"))
}

fn executable_available(program: &str) -> Value {
    let output = std::process::Command::new(program)
        .arg("--version")
        .output();
    match output {
        Ok(output) => json!({
            "available": output.status.success(),
            "exit_code": output.status.code().unwrap_or(-1)
        }),
        Err(error) => json!({
            "available": false,
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
    std::process::Command::new(program)
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
