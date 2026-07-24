# Implementation Roadmap

`contextpatch` should ship in small stages. Each stage must preserve the product thesis: AI coding agents get guarded edit primitives, not broad filesystem write power.

## Stage 1: useful safe-edit MVP

Stage 1 should finish the smallest serious product surface.

| Tool | Why it ships first |
| --- | --- |
| `read_range` | Safe bounded inspection before editing |
| `replace_exact` | Core anchored edit primitive |
| `diff_preview` | Reviewability before mutation |
| `status_guard` | Repository state visibility and edit gating |
| `write_new_file` | Safe create-only file creation |

Stage 1 is complete: these tools work through the core crate, CLI, and MCP server, with success/refusal tests and protocol-facing schemas.

## Stage 1 implementation order

1. `replace_exact` core behavior - implemented
2. `replace-exact` CLI command - implemented
3. `read_range` core behavior and `read-range` CLI command - implemented
4. `write_new_file` core behavior and `write-new-file` CLI command - implemented
5. `diff_preview` core behavior and `diff-preview` CLI command - implemented
6. `status_guard` core behavior and `status-guard` CLI command - implemented
7. Server tool schemas for implemented Stage 1 tools - implemented for `read_range`, `diff_preview`, `replace_exact`, `write_new_file`, and `status_guard`
8. Server transport for implemented Stage 1 tools - implemented for stdio MCP

## Stage 1 refusal tests

Stage 1 must test these refusals:

1. Empty old text for `replace_exact`
2. Zero-match replacement
3. Multi-match replacement
4. Path outside repository root
5. Create-only write where destination already exists
6. Dirty repository when a clean guard is requested

## Stage 2: advanced edit operations

| Tool | Reason for Stage 2 |
| --- | --- |
| `insert_at_anchor` | Useful convenience built on exact-anchor semantics |
| `apply_patch` | More complex atomicity and partial-apply behavior |
| `move_tracked` | Needs careful Git tracked/untracked behavior |
| `delete_guarded` | Higher-risk destructive primitive requiring hash confirmation |

Stage 2 should not start until Stage 1 has stable tests and docs.

## Stage 2A: Claude Desktop readiness and validation

Claude Desktop can continue real project work only if it can discover capabilities and run bounded validation without falling back to a broad shell. Stage 2A adds that bridge while preserving the safety contract.

| Tool | Reason for Stage 2A |
| --- | --- |
| `capability_manifest` | Let clients know exactly which file and process capabilities exist |
| `preflight_health` | Report repo cleanliness and expected validation-tool availability |
| `run_guarded_command` | Run repo-confined allowlisted validation commands without shell access |
| `read_command_log` | Let clients retrieve captured command output without forcing large JSON-RPC responses into the first call |
| `validation_profile_run` | Collapse common multi-command validation sequences into one auditable MCP call |
| `git_commit_exact` | Allow a narrow local commit checkpoint after exact-path validation |
| `git_remote_check` | Fetch exactly one explicit remote branch and report local/remote divergence without source edits |
| `git_branch_prepare` | Prepare one local branch from one explicit remote base branch after clean-worktree and required-file gates |
| `git_merge_readiness` | Analyze PR/merge readiness between two refs without exposing generic merge commands |
| `git_push_exact` | Publish the exact current commit only after branch, clean-worktree, expected-HEAD, remote-divergence, and confirmation guards |
| `setup_profile_run` | Plan typed, profile-owned setup actions for real projects without exposing generic package-manager or shell authority |
| `native_build_run` | Plan or run typed native build/test validators without exposing raw `xcodebuild` or Gradle authority |
| `native_device_run` | Plan or run bounded simulator/emulator/device smoke actions without exposing raw `xcrun` or `adb` authority |

Stage 2A is implemented for the MCP server. It intentionally does not add arbitrary shell command strings, destructive Git operations, merge/merge-tree command exposure, broad automatic commits, generic package-manager execution, or generic native-tool execution. Git mutation remains split by risk boundary: `git_commit_exact` is local only, `git_remote_check` is explicit single-branch fetch/report only, `git_branch_prepare` is clean-worktree branch setup from one remote base with explicit reset confirmation for existing branches, `git_merge_readiness` is read-only merge planning with optional single-branch fetch, and `git_push_exact` is exact current-HEAD push only with no force or multi-ref publishing. Setup support is split into `setup_profile_run`, which plans external-mutator commands from typed server-owned profiles and executes them only behind dry-run, clean-worktree, changed-path, and confirmation guards. Native build/device support is split into typed build/test validators and bounded device-smoke actions so agents do not need to know raw command syntax.

## Stage 2A-setup: profile-driven setup mutation

`setup_profile_run` may execute `dry_run=false` only under these gates:

1. Exact `confirm: "run setup profile"`
2. Clean worktree before execution
3. No-shell execution through shared process runner
4. Before/after Git status capture
5. Changed-path reporting
6. Refusal when changed paths fall outside the profile's expected changed-path classes
7. Clear reporting that the external tool performed the mutation and contextpatch did not provide atomic edit semantics

The first profile is `node-capacitor-shell` because it matches a real project setup need while staying universal: the profile owns dependency names, package-manager selection, and Capacitor actions, and callers never provide arbitrary package lists or commands. It uses pnpm when `pnpm-lock.yaml` is present and otherwise uses npm.

The profile includes a fixed `install_capacitor_filesystem` action for native-plugin workflows that need to hand fetched local files to native code. This is intentionally not a generic package-add surface.

The profile also includes `ios_pod_install` for CocoaPods-based native dependency setup. It remains a setup-profile action because CocoaPods mutates dependency state and should not be exposed through `run_guarded_command`, but it is intentionally refused when the cwd has no `Podfile`; Swift Package Manager based Capacitor projects do not need it.

## Stage 2A-native: typed native build and device actions

`native_build_run` supports:

1. `ios_build`
2. `ios_test`
3. `android_assemble_debug`
4. `android_unit_test`

Build actions derive exact `xcodebuild` or repo-relative Gradle wrapper plans from typed params. They default to dry-run and, when executed, compare Git source status before and after the build so generated source changes are reported as refusals instead of silent success.

`native_device_run` supports:

1. `ios_list_simulators`
2. `ios_create_simulator`
3. `ios_boot_simulator`
4. `ios_cap_run`
5. `ios_install_app`
6. `ios_launch_app`
7. `ios_read_logs`
8. `android_list_devices`
9. `android_install_app`
10. `android_launch_app`
11. `android_read_logcat`

Device actions derive exact `xcrun simctl` or `adb` plans from typed params. They default to dry-run, keep repository mutation out of scope, and require `confirm: "run native device"` before executing actions that change simulator, emulator, or device state.

## Stage 2B: latency and workflow compression

These tools should reduce MCP round trips and process-spawn overhead while keeping the same guardrails:

| Tool | Reason |
| --- | --- |
| `repo_snapshot` | One call for branch, HEAD, dirty paths, and short status |
| `read_many_ranges` | Batch related bounded reads instead of repeated JSON-RPC calls |
| `grep_profile` | Run named drift-search bundles without many separate `rg` calls |
| `list_files` | Native repo-confined file listing for simple `rg --files` use cases |

Stage 2B tools should prefer native Rust operations where possible and should return compact summaries plus log ids for large details.

Latency instrumentation is a Stage 2B must-have, not a cosmetic metric. Each MCP tool should be able to report timing and size metadata precise enough to separate Claude/Desktop round-trip overhead from server-side work. At minimum, record:

1. `request_received_to_dispatch_ms`
2. `argument_validation_ms`
3. `allowlist_validation_ms`
4. `child_spawn_ms`
5. `child_runtime_ms`
6. `stdout_stderr_drain_ms`
7. `redaction_truncation_ms`
8. `log_write_ms`
9. `response_bytes`
10. `total_tool_ms`

The goal is to stop guessing whether latency comes from JSON-RPC handling, process spawn, child runtime, output draining, redaction, log writes, or response size. Instrument first, then optimize based on measured p50/p95/p99 behavior across long Claude Desktop sessions.

## Always out of scope by default

- Generic `write_file`
- Recursive directory writes
- Unrestricted delete
- Unrestricted shell execution
- Broad or automatic Git commits
- Git fetch/push, resets, checkouts, or stashes
- Generic package-manager or scaffold execution outside declarative setup profiles
- Generic native build, simulator, emulator, or device command execution outside typed native tools
