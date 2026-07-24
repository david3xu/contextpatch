# Universal MCP Setup Support for Native Project Work

## Audit result

This repository is `contextpatch`, a universal safe MCP layer, not the downstream Capacitor/mobile app. The implementation work here should add reusable MCP guardrails that can support real native project work without becoming a one-project helper or a broad shell.

The native background/Capacitor project is a forcing function: it exposes real missing capabilities (`npm install`, scaffold/sync commands, native build tools, simulator/device tools). It should drive concrete requirements, but the design must remain ecosystem-oriented and policy-driven rather than hardcoding one app's rollout.

The current directory structure matters:

- `crates/core` owns command policy and execution safety.
- `crates/server` owns MCP schemas, capability reporting, request parsing, and tool responses.
- `crates/cli` should remain untouched unless a human CLI surface is intentionally added.
- `docs/` must describe the contract changes at the same time as behavior changes.

The implementation should not add `package.json`, `capacitor.config.ts`, `ios/`, `android/`, app dependencies, or downstream mobile project files to this repository.

## Current constraint

`run_guarded_command` is currently validation support, not a setup/scaffolding tool. Its policy lives in `crates/core/src/process/guarded_command.rs` and allows only:

- `git`: `status`, `diff`, `log`, `show`, `rev-parse`, `ls-tree`
- `cargo`: `check`, `test`, `build`, `clippy`
- `bun`: `run`, `test`
- `npm`: `run`, `test`
- `rg`: search invocations

It also rejects program names containing `/` or `\`, so repo-relative executables such as `./gradlew` cannot work under the current model.

Adding `npm install`, `npx cap add`, or `cap sync` directly to `run_guarded_command` would change its contract because those commands can mutate dependency manifests, lockfiles, native project folders, generated files, and package caches. Treating them as ordinary validation commands would blur the product boundary.

## Recommended architecture

Add a separate MCP-facing setup/scaffolding capability rather than widening `run_guarded_command` silently.

Suggested shape:

- Shared command-runner internals if useful, but separate policy types for validation versus setup.
- New core policy/execution module under `crates/core/src/process/` for guarded setup commands.
- New MCP tool in `crates/server/src/tools/`, preferably `setup_profile_run`.
- Schema and dispatch in `crates/server/src/main.rs`.
- Capability manifest entries that distinguish validation commands from mutating setup/scaffolding commands.
- Command logs should use the same redaction/truncation/log-id approach as `run_guarded_command`.

The setup capability should be explicit that it may mutate the repository. It should require:

- no shell command strings; program plus args only
- repo-root-confined cwd
- dry-run default for mutating setup commands
- explicit confirmation for mutation
- clean worktree before mutation
- command-specific allowlist rules
- timeout and output redaction
- before/after Git status summary in the response
- changed-path summary after mutation
- refusal if paths outside the repository root are referenced
- refusal for signing, credential, package-manager-global, or machine-mutating commands

## Universal policy model

Do not encode "Capacitor app setup" as the only concept. Model setup support as small policy families that can be expanded over time:

- `package_install`: dependency-manager operations that mutate manifests and lockfiles.
- `project_scaffold`: framework CLIs that create or synchronize project structure.
- `native_dependency_setup`: native dependency resolution such as CocoaPods.
- `native_build_validation`: native build/test commands.
- `device_smoke`: bounded simulator/emulator/device install, launch, and logs.

Each policy family should define:

- allowed programs
- allowed subcommands or task shapes
- whether the command mutates the repository
- whether dry-run is possible or required first
- required confirmation string for mutation
- expected changed paths or changed-path classes
- commands that remain explicitly unsupported

Named setup profiles can then compose those families for real ecosystems without making the MCP app-specific. For this real need, the first profile could be `node-capacitor-shell`, but the underlying implementation should still be reusable for future `node`, `rust`, `python`, `ios`, or `android` profiles.

## Pre-implementation audit findings

The setup feature should not start as a generic "run this setup command" API. Even with an allowlist, that shape would make the public MCP contract command-centric and would encourage adding exceptions for every real project. The safer universal shape is profile-centric:

- Public tool name: `setup_profile_run`.
- Required input: `profile`.
- Required input: a profile-owned `step` or `action`, not a free-form shell string.
- Optional inputs: typed action parameters, repo-confined `cwd`, `timeout_secs`, `dry_run`, and `confirm`.
- Core maps each profile/action to one exact command plan, validates it, then either returns the dry-run plan or executes it.

For the first profile, the action vocabulary can stay small:

- `install_capacitor_dependencies`
- `cap_init`
- `cap_add_ios`
- `cap_add_android`
- `cap_sync`

This keeps the real Capacitor need implementable while preserving the universal MCP model: future support adds profiles/actions, not broad package-manager authority.

Current implementation surfaces imply these engineering constraints:

- `crates/core/src/process/guarded_command.rs` currently owns private timeout, cwd, process-runner, redaction, and display helpers. Before adding setup support, extract reusable runner pieces into a shared core module such as `crates/core/src/process/runner.rs`; keep validation policy in `guarded_command.rs`.
- Changed-path reporting should belong in `crates/core`, not be copied from server helpers. Add core Git status helpers that can return short status and a parsed path set.
- Existing `crates/server/src/main.rs` already has a large tool-definition and dispatch table. Add the setup tool by following the existing constant-module pattern in `crates/server/src/tools/`, but avoid moving unrelated server code during this feature.
- The response should use the command-log flow already used by `run_guarded_command` and `validation_profile_run`: compact text response plus `log_id`, with full output available through `read_command_log`.
- Tests should not require npm, Capacitor, Xcode, Android SDK, or CocoaPods to be installed. Unit tests should validate dry-run planning and refusals. Integration tests should exercise MCP schema/listing and dry-run/refusal behavior only.
- The first mutation path may be untested against real external tools in CI. That is acceptable only if dry-run/profile validation is covered and the docs state external native tooling still needs local machine verification.

Important refusal cases before any mutation:

- unknown profile
- unknown action for a known profile
- `dry_run=false` without the exact confirmation literal
- dirty worktree before mutation
- cwd outside the repository
- absolute paths, traversal, NUL bytes, or oversized arguments in action parameters
- unsupported package names, arbitrary `npx`, global installs, signing tools, credential tooling, or machine package managers

## Second audit findings

The profile model is the right direction, but the implementation must explicitly handle one product-boundary tension: setup/scaffolding commands are external repository mutators. Unlike `replace_exact` or `write_new_file`, contextpatch cannot make `npm install` or `cap add ios` atomic or anchored internally. That means setup support must be documented and implemented as a narrow **delegated-mutator exception**, not as a normal edit primitive.

Rules for that exception:

- It lives outside `run_guarded_command`.
- It is never exposed as arbitrary package-manager or framework-CLI access.
- It defaults to plan/dry-run.
- Actual mutation requires a clean worktree and exact confirmation.
- The response must clearly say the mutation was performed by an external tool.
- The response must include before/after Git status and changed paths so the caller can review, commit, or revert manually.
- It must not claim atomicity for external tool writes.
- It must refuse if the tool changes paths outside the expected changed-path classes for the selected action.

The public API should be declarative:

```json
{
  "profile": "node-capacitor-shell",
  "action": "cap_init",
  "params": {
    "app_id": "com.example.app",
    "app_name": "Example",
    "web_dir": "dist"
  },
  "dry_run": true
}
```

The caller should not provide `program` or raw `args`. Core should derive an exact command plan from `profile`, `action`, and typed `params`.

Typed parameter implications for the first profile:

- `install_capacitor_dependencies`: no caller-supplied package list at first; the profile owns the exact package list.
- `cap_init`: require explicit `app_id`, `app_name`, and `web_dir`; do not hardcode downstream-project placeholders in contextpatch.
- `cap_add_ios`: no untyped args.
- `cap_add_android`: no untyped args.
- `cap_sync`: optional `platform` enum of `ios`, `android`, or `all`; no arbitrary platform string.

Capability reporting should add a separate top-level section, not bury setup under validation execution:

- `setup_profiles.available`
- `setup_profiles.mode = "profiled_external_mutators"`
- supported profiles/actions
- mutation confirmation literal
- unsupported boundaries

Preflight health should report only local tool availability needed by supported setup profiles. It should not run package installs, `cap` commands, native builds, simulators, or device commands.

Naming audit:

- `setup_profile_run` is acceptable because it mirrors `validation_profile_run`.
- Avoid `run_guarded_setup_command` because it exposes command-running as the concept and invites broad allowlist growth.
- Avoid `native_background_*` names because the feature is universal project setup support, not a mobile-app-specific subsystem.

Implementation-order audit:

1. Extract shared no-shell process execution helpers from `guarded_command.rs` without changing `run_guarded_command` behavior.
2. Add core setup profile planning and dry-run validation only.
3. Add core Git status path helpers for changed-path reporting.
4. Wire MCP schema/dispatch/capability manifest for `setup_profile_run`.
5. Add server dry-run/refusal tests.
6. Only then enable `dry_run=false` execution for the first narrow actions.

This sequencing prevents a half-built external mutator from landing before profile planning, refusals, docs, and protocol shape are stable.

## Directory structure audit

The current crate split is correct, but the setup feature is large enough that dumping more logic into existing monolithic files would make the codebase harder to maintain.

Current structure:

- `crates/core/src/fs/`, `git/`, `patch/`, `replace/`, `process/`, and `policy/` are already feature-oriented.
- `crates/core/src/process/guarded_command.rs` currently mixes validation policy, process execution, redaction, timeout handling, and tests.
- `crates/server/src/tools/` contains tool-name modules, but most schemas and handlers still live in `crates/server/src/main.rs`.
- `crates/server/src/main.rs` is already large; adding setup-profile parsing and response logic there directly would worsen organization.

Recommended incremental organization:

```text
crates/core/src/process/
  mod.rs
  runner.rs              # shared no-shell process execution, timeout, cwd, output drain, redaction
  guarded_command.rs     # validation-command policy only, using runner.rs

crates/core/src/setup/
  mod.rs
  profile.rs             # public setup_profile_run entrypoint and common request/response types
  plan.rs                # command-plan and expected-changed-path abstractions
  node_capacitor.rs      # first concrete setup profile

crates/core/src/git/
  mod.rs
  status.rs              # existing status_guard summaries
  porcelain.rs           # parsed status path sets for changed-path reporting, if status.rs would get too large

crates/server/src/tools/
  setup_profile_run.rs   # tool name plus setup-specific schema/adapter helpers where practical
```

Avoid these refactors in this round:

- Do not reorganize all existing server tools just to make the new one look perfect.
- Do not move every Git workflow out of `main.rs` in the same commit as setup support.
- Do not rename existing public modules or tool names.
- Do not create app-specific directories such as `native/`, `capacitor/`, or `mobile/` at top level.

Feature organization rule:

- New reusable execution machinery goes under `core/process`.
- New delegated-mutator setup policy goes under `core/setup`.
- Git status parsing shared by multiple features goes under `core/git`.
- MCP protocol adaptation stays under `server`, with only thin setup-specific adapters there.
- The CLI remains unchanged unless a separate human-facing setup command is intentionally added later.

## Updated implementation plan

Implement in small reviewable slices:

1. **Shared process runner extraction**
   - Move timeout validation, cwd resolution, no-shell process execution, concurrent stdout/stderr draining, command display, redaction, and truncation into a shared core process helper.
   - Target `crates/core/src/process/runner.rs`.
   - Keep `run_guarded_command`'s public behavior and allowlist unchanged.
   - Validate with existing guarded-command tests before adding setup behavior.

2. **Core setup profile planner**
   - Add a setup-profile module that accepts `profile`, `action`, typed params, `cwd`, `timeout_secs`, `dry_run`, and `confirm`.
   - Target `crates/core/src/setup/` with common profile/plan types and one `node_capacitor.rs` profile module.
   - Implement dry-run planning for `node-capacitor-shell` only.
   - Core derives exact command plans; callers never supply raw `program,args`.
   - Add refusals for unknown profile/action, invalid params, path traversal, and missing confirmation.

3. **Core Git changed-path helpers**
   - Add reusable core helpers for short status and parsed dirty-path sets.
   - Keep helpers in `crates/core/src/git/status.rs` unless the parsing logic grows enough to justify `git/porcelain.rs`.
   - Use these helpers for clean-worktree checks and before/after changed-path reporting.
   - Do not copy server-only Git parsing logic into the setup module.

4. **MCP protocol wiring**
   - Add `setup_profile_run` tool constant, schema, dispatch, argument parsing, command-log writing, and compact response.
   - Keep setup-specific adapter helpers close to `crates/server/src/tools/setup_profile_run.rs` where that improves readability, while avoiding a broad server refactor.
   - Add `setup_profiles` capability-manifest output and preflight tool availability reporting.
   - Keep CLI unchanged.

5. **Tests and docs**
   - Add core tests for dry-run plans and refusals.
   - Add server tests for tool listing, schema-level flow, dry-run success, and `isError` refusals.
   - Update `docs/tool-spec.md`, `docs/safety-contract.md`, `docs/implementation-roadmap.md`, `docs/claude-desktop.md`, and `README.md` if the public tool list changes.

6. **External mutation enablement**
   - Enable `dry_run=false` only after the plan/refusal/protocol layer is stable.
   - Require clean worktree and exact confirmation.
   - Return external-mutator metadata, command log id, before/after status, and changed paths.
   - Refuse unexpected changed-path classes for each action.

## Native project scenario requirements

### First profile: `node-capacitor-shell`

This profile is justified by the immediate real project need, but should be represented as one named ecosystem profile, not as universal behavior for all `npm` or `npx`.

Candidate commands:

- `npm install @capacitor/core @capacitor/cli @capacitor/ios @capacitor/android`
- `npm exec -- cap init ...` or `npx cap init ...`
- `npm exec -- cap add ios`
- `npm exec -- cap add android`
- `npm exec -- cap sync`

Guardrails:

- Require clean worktree before mutation.
- Default to dry-run or plan output when possible.
- Require explicit confirmation for mutation.
- Return exact changed path list after the command.
- Restrict package names for this profile to the Capacitor packages above.
- Restrict `npx`/`npm exec` to the Capacitor CLI for this profile.
- Do not make arbitrary `npm install` or arbitrary `npx` globally available.

### Later profiles or policy additions

- `pod install`
- `xcodebuild` build/test invocations
- `xcrun simctl ...`
- `./gradlew` or `gradle` build/test invocations
- `adb` install/launch/logcat
- optional `swift`
- optional `kotlinc`

Guardrails:

- `pod install` is setup/mutation, not validation; require clean worktree and report changed files.
- `xcodebuild`, `gradle`, `swift`, and `kotlinc` can be treated as build/test validation when args are restricted to build/test tasks.
- `xcrun` should be limited to `simctl` subcommands needed for simulator boot/install/launch.
- `adb` should be limited to device/emulator inspection, install, launch, and bounded logcat.
- `./gradlew` requires a new policy for repo-relative executable wrappers, because current program validation rejects paths.

### Native plugin-round capability plan

The first implementation intentionally unblocks only the foundation round through `setup_profile_run` and the `node-capacitor-shell` profile. That is correct for the immediate Capacitor shell work, but it does not cover the next native plugin round. The missing tools should be added as easy-to-use declarative MCP capabilities, not as broad raw allowlist entries.

The philosophy stays the same:

- callers choose named tools, profiles, and actions
- callers supply typed parameters, not shell strings
- core derives exact command plans
- dry-run or bounded inspection is available first
- mutation requires clean worktree and exact confirmation
- device operations are separated from repository mutation
- capability manifest and preflight explain what is available without requiring developers to know low-level commands

Recommended next surfaces:

| Capability | Purpose | Programs covered | Public shape |
| --- | --- | --- | --- |
| Extend `setup_profile_run` | Native dependency setup that may mutate repo files | `pod install`, narrowly scoped Gradle sync if needed | Profile/action such as `ios_pod_install`; clean-worktree plus confirmation |
| Add `native_build_run` | Typed native build/test validation | `xcodebuild`, `./gradlew`, optionally `swift`/`kotlinc` later | Platform/action such as `ios_build`, `ios_test`, `android_assemble_debug`, `android_unit_test` |
| Add `native_device_run` | Bounded simulator/emulator/device smoke operations | `xcrun simctl`, `adb` | Platform/action such as `list_devices`, `boot_simulator`, `install_app`, `launch_app`, `read_logs` |

Do not put these programs into `run_guarded_command` as generic entries. That would make the validation tool a backdoor shell for native workflows and would force developers to know exact command syntax. The MCP should instead advertise high-level action names that map to safe, profile-owned command plans.

### `setup_profile_run` extensions

Add native setup actions only when they are repository setup mutators:

- `ios_pod_install`
  - command plan: `pod install`
  - allowed cwd: an iOS project directory under the repository, usually `ios/App`
  - mutation policy: clean worktree and `confirm: "run setup profile"`
  - expected changed-path classes: `ios_project`, CocoaPods lockfiles, workspace/project metadata
- `android_gradle_sync`
  - add only if there is a concrete CLI sync workflow that mutates expected Gradle files
  - prefer build validation through `native_build_run` if no repository mutation is needed

### New `native_build_run`

This should be a build/test validator, not a setup mutator and not a raw command runner.

Inputs should be high-level and typed:

```json
{
  "platform": "ios",
  "action": "build",
  "params": {
    "workspace": "ios/App/App.xcworkspace",
    "scheme": "App",
    "configuration": "Debug",
    "sdk": "iphonesimulator"
  }
}
```

Initial action set:

- iOS:
  - `ios_build`
  - `ios_test`
  - optional later `swift_test` only if a package-based plugin workflow needs it
- Android:
  - `android_assemble_debug`
  - `android_unit_test`
  - optional later `android_connected_test` only when device access is intentionally enabled

Guardrails:

- no shell strings
- repo-confined cwd
- only known workspace/project/Gradle wrapper paths under the repo
- only allowlisted xcodebuild flags and Gradle tasks
- timeout, redaction, truncation, and command-log support
- no signing/export/archive/store-submission actions by default
- `./gradlew` requires explicit repo-relative wrapper handling instead of weakening generic program validation

### New `native_device_run`

Device and simulator actions should be separate because they affect local simulator/device state rather than repository files.

Initial action set:

- iOS simulator:
  - `ios_list_simulators`
  - `ios_boot_simulator`
  - `ios_install_app`
  - `ios_launch_app`
  - `ios_read_logs`
- Android:
  - `android_list_devices`
  - `android_install_app`
  - `android_launch_app`
  - `android_read_logcat`

Guardrails:

- no arbitrary `xcrun` or `adb`
- only `xcrun simctl` subcommands required for bounded smoke testing
- bounded `adb logcat` with timeout and line/byte limits
- app identifiers and artifact paths validated as typed params
- no credential, signing, pairing, root, backup/restore, screen recording, or broad file-transfer commands by default
- clear response metadata that the operation targets local simulator/device state, not repository state

### Developer-use principle

The developer should not need to know whether the underlying command is `npm exec`, `pod install`, `xcodebuild`, `./gradlew`, `xcrun simctl`, or `adb`. They should be able to inspect `capability_manifest`, see supported profiles/actions, and call one named action with typed params. Low-level command syntax remains an implementation detail owned by core profiles.

### Fresh audit before implementation

The native plugin-round plan is directionally right, but the next implementation should not simply append more variants to the current `setup_profile_run` code. The current implementation was intentionally shaped for one setup profile and one mutating command-plan model. Native build and device workflows have different safety properties and need a small supporting refactor before adding tools.

Audit findings:

1. Current setup params are Capacitor-specific.
   - `SetupActionParams` currently contains `CapInit` and `CapSync`, which is fine for `node-capacitor-shell` but will become messy if `ios_pod_install`, xcodebuild params, Gradle params, and device params are added there.
   - Before adding plugin-round actions, keep setup params scoped to setup profiles and introduce separate typed params for native build/device tools.
   - Do not make the server pass generic JSON blobs to core without typed validation.

2. Current `CommandPlan` assumes setup mutation.
   - `crates/core/src/setup/plan.rs` sets `mutates_repo: true` and `external_mutator: true` for every setup plan.
   - That is correct for Capacitor setup and `pod install`, but wrong for build/test and device smoke actions.
   - Native build/device tools should use their own plan/result types, or a shared lower-level process plan with an explicit operation class: `repo_mutator`, `repo_validation`, or `device_operation`.

3. `./gradlew` cannot fit the current executable-name validator.
   - The shared runner currently rejects program names containing `/` or `\`, which protects `run_guarded_command` from path execution.
   - Do not weaken that rule globally.
   - Add an explicit program representation for native build only, such as `ExecutableName("xcodebuild")` versus `RepoRelativeExecutable("gradlew")`, with canonicalization under the repo root and executable-file checks.

4. Build tools may write outside the repository even when they do not edit source.
   - `xcodebuild`, Gradle, SwiftPM, and Kotlin tooling can write DerivedData, build caches, and package caches outside the repo.
   - The response should not claim these are pure read-only operations.
   - Treat them as external build validators that must leave Git source status unchanged, and report before/after repository status.
   - Do not require clean worktree for read-only validation by default unless the action needs reproducibility; but always report dirty state clearly.

5. Device tools operate on local simulator/device state, not repo state.
   - `native_device_run` should not reuse setup mutation confirmation.
   - It should have its own safety model: typed device/app identifiers, bounded logs, artifact path validation, timeout, and a clear statement that the operation changes local simulator/device state.
   - It should not stage, commit, or check changed paths as if it were a repository mutator.

6. Output handling needs to be more compact than setup output.
   - Native build logs and device logs can be much larger than setup output.
   - `native_build_run` and `native_device_run` should return compact summaries plus `log_id`; full stdout/stderr/logcat should live in `read_command_log`.
   - Avoid dumping full xcodebuild/Gradle/logcat output into the MCP text response.

7. Tool availability should be precise and non-invasive.
   - `preflight_health` may check whether `pod`, `xcodebuild`, `xcrun`, `adb`, `swift`, and `kotlinc` exist with stdout/stderr suppressed.
   - It must not boot simulators, run builds, install packages, or touch devices during preflight.
   - Missing optional tools should be reported as unavailable prerequisites for specific actions, not server failure.

8. Start with the minimum native plugin round, not every possible command.
   - First native build slice should likely include `ios_build`, `android_assemble_debug`, and maybe `ios_pod_install`.
   - Device smoke actions should come after build actions unless the immediate downstream workflow already has built artifacts to install.
   - `swift` and `kotlinc` should remain optional until there is a real package/compiler-only workflow that `xcodebuild` or Gradle cannot cover.

9. Server organization should improve incrementally.
   - `crates/server/src/main.rs` already holds schema and adapter logic for many tools.
   - Adding two new native tools entirely inline would worsen maintainability.
   - Use `crates/server/src/tools/native_build_run.rs` and `native_device_run.rs` for names and, where practical, schema/adapter helpers; keep the server as a thin protocol adapter and avoid command derivation there.

10. The public UX should be profile/action first.
    - Capability manifest should advertise action names, required params, confirmation requirements, and unavailable prerequisites.
    - Developers should not need to know the underlying command syntax or whether the implementation uses `xcodebuild`, `./gradlew`, `xcrun simctl`, or `adb`.
    - Refusals should tell the developer which high-level action or parameter is unsupported, not expose low-level allowlist internals first.

Implementation adjustment from this audit:

1. Add shared core plan/execution primitives only where they reduce duplication; keep policy modules separate.
2. Extend `setup_profile_run` only for repository setup mutators such as `ios_pod_install`.
3. Add `core::native_build` and MCP `native_build_run` for build/test validation with source-status before/after checks.
4. Add repo-relative executable support only inside native build policy for `gradlew`.
5. Add `core::native_device` and MCP `native_device_run` later, after build support, for simulator/emulator smoke actions.
6. Update docs, capability manifest, preflight, and tests with each surface as it lands.

## Do not support by default

- `security`
- `fastlane`
- `brew`
- store submission CLIs or web-portal automation
- signing/provisioning credential workflows
- arbitrary package-manager global installs

These grant machine-level or credential-level authority and should be handled as a separate release/security decision.

## Files likely impacted in this repository

- `crates/core/src/process/guarded_command.rs`: keep validation-only behavior intact; extract shared runner pieces only if it reduces duplication without changing behavior.
- `crates/core/src/process/mod.rs`: expose any new setup-command module.
- `crates/server/src/tools/mod.rs`: add the new tool constant module.
- `crates/server/src/main.rs`: add schema, dispatch, capability manifest output, argument parsing, and response formatting.
- `crates/server/tests/stage1_mcp.rs`: add MCP integration tests for allowed setup commands and refusals.
- `docs/tool-spec.md`: document the new tool and command policy.
- `docs/safety-contract.md`: document the new mutation boundary.
- `docs/implementation-roadmap.md`: add this as a later-stage workflow support item.
- `docs/claude-desktop.md`: document how agents should use setup support without falling back to a broad shell.
- `.github/copilot-instructions.md`: update only if the repo conventions or test commands change.

## Tests to add

- Core policy refuses `npm install` through `run_guarded_command`.
- New setup/profile policy allows the exact intended `node-capacitor-shell` command shapes.
- New setup policy refuses arbitrary `npx`, global installs, path traversal, empty args, absolute paths, and non-allowlisted programs.
- New setup policy refuses a Capacitor package install outside the profile or without explicit mutation confirmation.
- Repo-relative wrapper handling is tested separately before enabling `./gradlew`.
- MCP tool returns `isError: true` refusals rather than JSON-RPC transport errors.
- Mutating setup command response includes before/after status or changed paths.
- Existing `run_guarded_command` validation tests still pass unchanged.

## Validation for contextpatch changes

Run the Rust workspace checks, not downstream app commands:

```bash
cargo fmt --all -- --check
cargo test -p core guarded_command
cargo test -p server stage1_mcp_tools_work_together
cargo test -p server <new_setup_tool_test_name>
cargo check --workspace
```

## Completion rule

This contextpatch round is complete only when the MCP server can advertise and enforce a narrow, profile-driven setup/scaffolding capability that supports the native project scenario without weakening `run_guarded_command` into a broad shell or broad package-manager runner.
