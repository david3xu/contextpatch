# Claude Desktop

`contextpatch` is intended to run as a local context server for desktop agent clients.

## Behavior

The server should expose safe edit and validation tools rather than generic filesystem writes or broad shell access. Claude Desktop or another client can request bounded reads, preview diffs, apply guarded edits, discover capabilities, run preflight health checks, and run allowlisted validation commands.

## Tool surfaces

`contextpatch-server` supports two public surfaces over the same internal action handlers:

- `--tool-surface full` advertises all 49 actions directly. It is the server default when the option
  is omitted.
- `--tool-surface project` advertises only `project_execute` for the fixed `--repo-root`. Call it
  with `action: "describe"` to list actions, or include `arguments.name` to retrieve one action's
  exact schema. A normal call selects one action and passes its original argument object.

Project mode reduces Claude's persistent approval scope to one stable public tool identity per
configured project. It does not combine repositories, execute batches, or weaken guards. The server
resolves the internal action before choosing its deadline, mutation lock, handler, log/receipt name,
or recovery guidance. Direct hidden-tool calls are refused.

## Server boundary

The server should not expose broad filesystem write tools. In particular, the default server must not expose:

- generic `write_file`
- unrestricted delete
- recursive directory writes
- shell execution
- broad Git reset/checkout/stash/fetch/push
- Git merge or merge-tree as generic commands
- broad or automatic Git commits
- generic package-manager or scaffold execution
- generic native build, simulator, emulator, or device command execution

The expected agent workflow is:

1. Use `read_range` to inspect a bounded file section.
2. Use `diff_preview` before `replace_exact` when reviewing exact anchored edits.
3. Use `status_guard` before writes when a clean repository or clean target path is required.
4. Use `create_directory` for create-only directory creation, then `write_new_file` or `write_new_file_base64` for create-only repository file creation inside that directory.
5. Use `file_info`, `list_directory`, and `read_file_bytes` for digests, line counts, directory metadata, symlink visibility, or binary spot checks instead of writing ad hoc repo-local scripts.
6. Use `capability_manifest` and `preflight_health` to determine whether this server can support the current workflow.
7. Compare the capability manifest's build SHA with the checkout before concluding that a source capability is unavailable.
8. Use `{scratch}` inside guarded command arguments for durable byproducts that must stay outside the repository.
9. Use `read_write_receipts` after an interrupted or timed-out receipt-enabled mutation, then compare the recorded digest or Git HEAD with current state before retrying.
10. Use `run_guarded_command` only for allowlisted validation commands such as `git status`, `git diff`, `cargo check`, project `bun run` checks, repo-relative Python scripts, `pytest`, `harbor run`, the exact `references/check-base-image.sh` or `references/check-base-image.sh task` checks, or `rg` drift searches.
11. Use `git_stage_exact` when files must be staged/tracked before the work is ready to commit.
12. Use `git_staged_scope_check` to audit that staged files are limited to allowed exact paths/prefixes, such as `.gitignore` plus `task/`, before reporting staged-scope readiness.
13. Use `git_commit_exact` only when the desired local commit path set is explicit and complete.
14. Use `git_commit_scoped` when the desired commit paths are explicit but unrelated dirty files should be preserved for later work. It requires a clean index before staging the requested paths.
15. Use `git_commit_prefix` when the desired commit is a large generated or fixture tree under known prefixes; inspect the dry-run expanded path list before confirming.
16. When committing, use project-owned commit messages only. Do not add Claude, Anthropic, AI, assistant, or co-authored-by attribution unless the repository owner explicitly asks for it for that specific commit.
17. Use `artifact_write_text` or `artifact_write_base64` for generator, trap-check, or other sidecar files that must not enter the repository tree.
18. Use `artifact_delete_exact` to clean up one sidecar file after reviewing its dry-run SHA-256; do not create successive filenames merely to iterate a helper.
19. Use `artifact_python_run` for Python scratch analysis from the sidecar artifact root instead of creating temporary scripts inside the repository.
20. Use `bulk_write_new_files_base64` for bounded create-only fixture-tree imports when a generator cannot be run directly.
21. Use `fixture_generator_run` for repo-relative Python fixture generators that need declared output mutation; it supports temporary untracked generator scripts and verifies all changed paths stay within declared outputs.
22. Use `base_image_check_run` for the exact `references/check-base-image.sh` validation, optionally with `project_path: "task"`, instead of asking for generic shell.
23. Use `image_cleanliness_check_run` for the narrow Docker image check that searches the built image for a forbidden file name such as `solve.sh`; keep dry-run unless Docker execution is explicitly needed.
24. Use `docker_image_inspect` for the narrow read-only Docker image metadata workflow; do not ask for arbitrary Docker commands.
25. Use `delete_untracked_exact` only for explicit untracked regular files that would otherwise fail clean-worktree or no-extraneous-files gates.
26. Use `delete_generated_prefix` for ignored/untracked generated files and empty directories under known prefixes, such as `__pycache__`, after reviewing the dry-run path list.
27. Use `git_remote_list` to inspect whether `origin` and fork/upstream remotes are configured before publishing.
28. Use `git_remote_check` before publishing to fetch one explicit remote branch and inspect whether the remote is ahead.
29. Use `git_branch_prepare` to create or switch to a local branch from one explicit remote base branch after a clean-worktree check.
30. Use `git_merge_readiness` before PR/merge work when the question is whether two refs changed the same files since their merge base.
31. Use `git_push_exact` only after a clean local exact commit, matching branch, matching expected HEAD, no remote-ahead divergence, and explicit confirmation.
32. Use `setup_profile_run` for server-owned setup profiles instead of broad `npm install`, `pnpm add`, `npx`, or shell execution; keep `dry_run` enabled until the plan is reviewed.
33. Use `github_pr_run` for GitHub PR auth/status/check/view/create workflows and Actions evidence instead of asking for arbitrary `gh` access. Supply `repository: "OWNER/REPO"` when the checkout is a fork but the PR belongs to upstream. Use `pr_view` for the exact head SHA, `workflow_runs_for_commit` to separate current from stale runs, `pr_comments` with a short marker for sticky findings, and then `workflow_run_view` or `workflow_job_log` for execution evidence. If the evidence proves a transient failure, review the `workflow_run_rerun_failed` dry-run before confirming the failed-job rerun. Keep PR creation in dry-run until the title, body, base, and head are reviewed.
34. Use `github_fork_prepare` to plan or run `gh repo fork` with explicit confirmation instead of arbitrary `gh repo` commands.
35. Use `native_build_run` for typed iOS/Android build and test actions instead of raw `xcodebuild` or `./gradlew`.
36. Use `native_device_run` for typed simulator/emulator/device smoke actions instead of raw `xcrun` or `adb`; keep `dry_run` enabled until the plan is reviewed.
37. Use `write_existing_file_exact_hash` for whole-file synchronization only when the current file SHA-256 is known and reviewed.
38. Use `fixture_manifest_verify` and `fixture_manifest_refresh` to enforce exact fixture file sets and SHA-256 digests instead of asking for ad hoc hashing scripts.

## Configure ordinary Claude Desktop entries

Build both release binaries:

```bash
cargo build --release -p cli --bin contextpatch
cargo build --release -p server --bin contextpatch-server
```

Claude Desktop's normal local configuration uses the `mcpServers` map in `claude_desktop_config.json`. After adding one or more entries whose command basename is `contextpatch-server`, validate them with:

```bash
./target/release/contextpatch configure-claude-desktop
```

The command keeps each detected entry in the ordinary `mcpServers` map, preserves its command,
repository root, environment, unrelated arguments, unrelated entries/data, and user-authored
policies, and sets `--tool-surface project` by default. It writes no approval policy. If an older
ContextPatch version added the exact wildcard `toolPolicy: {"*":"allow"}` to a targeted entry, the
command removes only that legacy value; every other `toolPolicy` value is retained. A changed config
receives a timestamped sibling backup containing its exact original bytes; permissions are preserved,
a cooperative lock coordinates updates, and a final stale-read check protects the atomic replacement.
Repeated runs are idempotent.

Use `--dry-run` to preview the migration without changing the config. Use
`--config /path/to/claude_desktop_config.json` for a custom config path. Use
`--tool-surface full` as the explicit rollback when direct tools are preferred.

The local ContextPatch MCP connection requires no authentication. Claude Desktop owns runtime tool authorization. Neither ordinary MCP configuration nor local DXT/Desktop Extension metadata can preapprove tools, and ContextPatch does not consult or write `Claude-3p/configLibrary`. Project mode usually reduces the required persistent choice to **Always allow** or **Allow for all tasks** for `project_execute` once per configured project; it cannot silently grant that choice. Approval UX never bypasses ContextPatch's repository, path, hash, Git-state, dry-run, confirmation, deadline, lock, or receipt guards.

## Build and configure Claude Desktop

If the MCP server entry does not exist yet, point Claude Desktop at the release server. Use absolute paths because Claude Desktop does not inherit your shell's current directory:

```json
{
  "mcpServers": {
    "contextpatch": {
      "command": "/absolute/path/to/contextpatch/target/release/contextpatch-server",
      "args": [
        "--repo-root",
        "/absolute/path/to/repo",
        "--tool-surface",
        "project"
      ]
    }
  }
}
```

Change `--repo-root` to the repository Claude should edit. The server treats that directory as the path guard root.

On macOS, Claude Desktop reads this configuration from:

```text
~/Library/Application Support/Claude/claude_desktop_config.json
```

Run `contextpatch configure-claude-desktop` to validate the ordinary entry, add or normalize the
project surface, and clean the exact legacy ContextPatch wildcard `toolPolicy` if present. Restart
Claude Desktop after changing configuration or rebuilding the server binary.

## Local development command

Before packaging, the server can be launched from the workspace with:

```bash
cargo run -p server --bin contextpatch-server -- --repo-root /path/to/repo \
  --tool-surface project
```

After installation, the intended command is:

```bash
contextpatch-server --repo-root /path/to/repo --tool-surface project
```

The workspace package is named `server`; the installed binary remains `contextpatch-server` to avoid colliding with generic commands.

## Quick MCP smoke test

After restarting Claude Desktop, ask it to list available `contextpatch` tools. A project-surface
entry should expose exactly `project_execute`; call `{"action":"describe"}` to verify the 49 internal
actions and build stamp. A full-surface entry should expose:

- `read_range`
- `read_write_receipts`
- `diff_preview`
- `replace_exact`
- `status_guard`
- `write_new_file`
- `write_new_file_base64`
- `write_existing_file_exact_hash`
- `file_info`
- `list_directory`
- `read_file_bytes`
- `artifact_write_text`
- `artifact_write_base64`
- `artifact_delete_exact`
- `bulk_write_new_files_base64`
- `create_directory`
- `capability_manifest`
- `preflight_health`
- `run_guarded_command`
- `artifact_python_run`
- `image_cleanliness_check_run`
- `docker_image_inspect`
- `fixture_generator_run`
- `base_image_check_run`
- `fixture_manifest_verify`
- `fixture_manifest_refresh`
- `read_command_log`
- `validation_profile_run`
- `setup_profile_run`
- `native_build_run`
- `native_device_run`
- `git_commit_exact`
- `git_stage_exact`
- `git_staged_scope_check`
- `git_commit_scoped`
- `git_commit_prefix`
- `git_restore_exact`
- `move_tracked`
- `delete_guarded`
- `delete_untracked_exact`
- `delete_generated_prefix`
- `git_remote_list`
- `git_remote_check`
- `git_branch_prepare`
- `git_merge_readiness`
- `git_push_exact`
- `github_pr_run`
- `github_fork_prepare`

Each advertised tool includes MCP annotations, but annotations do not suppress Claude Desktop approval prompts. Local MCP and Desktop Extension metadata cannot grant runtime approval; that decision belongs to Claude Desktop and its account or organization policy.

The rebuilt release binary advertises 49 direct tools in full mode or one wrapper in project mode. If Claude Desktop lists a different surface, inspect the entry's `--tool-surface` argument. In full mode compare `capability_manifest.build.git_sha` with the checkout; in project mode use `project_execute` action `describe`. A mismatched SHA means the configured binary is stale; a matching SHA with stale tools usually means Claude Desktop cached an older tool list. Fully quit and restart Claude Desktop, then confirm the ordinary `claude_desktop_config.json` entry points at the rebuilt binary:

```text
/Users/291928k/Developer/contextpatch/target/release/contextpatch-server
```

Then list contextpatch tools again. If the list is still partial, the chat is likely connected to an older binary/config entry or Claude Desktop cached a partial MCP tool list for that session; start a fresh chat after restart.

Tool loading and exact-name search ranking are controlled by Claude Desktop. The configurator leaves distinct ordinary ContextPatch entries for distinct repositories in place and does not create a second managed source.

## Currently exposed tools

The current server exposes these implemented safe primitives directly in full mode and as internal
actions behind `project_execute` in project mode:

- `read_range`
- `read_write_receipts`
- `diff_preview`
- `replace_exact`
- `status_guard`
- `write_new_file`
- `write_new_file_base64`
- `write_existing_file_exact_hash`
- `file_info`
- `list_directory`
- `read_file_bytes`
- `artifact_write_text`
- `artifact_write_base64`
- `artifact_delete_exact`
- `bulk_write_new_files_base64`
- `create_directory`
- `capability_manifest`
- `preflight_health`
- `run_guarded_command`
- `artifact_python_run`
- `image_cleanliness_check_run`
- `docker_image_inspect`
- `fixture_generator_run`
- `base_image_check_run`
- `fixture_manifest_verify`
- `fixture_manifest_refresh`
- `read_command_log`
- `validation_profile_run`
- `setup_profile_run`
- `native_build_run`
- `native_device_run`
- `git_commit_exact`
- `git_stage_exact`
- `git_staged_scope_check`
- `git_commit_scoped`
- `git_commit_prefix`
- `git_restore_exact`
- `move_tracked`
- `delete_guarded`
- `delete_untracked_exact`
- `delete_generated_prefix`
- `git_remote_list`
- `git_remote_check`
- `git_branch_prepare`
- `git_merge_readiness`
- `git_push_exact`
- `github_pr_run`
- `github_fork_prepare`

Other documented boundary tools, including `apply_patch` and `insert_at_anchor`, remain roadmap items until implemented.

`capability_manifest` reports the build Git SHA, dirty state, UTC build timestamp, and Cargo profile so the active binary can be identified without inference. It also reports the stable repository-specific scratch root, cooperative mutation coordination, receipt outcomes, and direct reply deadlines. Reads return within 30 seconds, direct writes within 60 seconds, and Git/GitHub operations within 120 seconds or return an unknown-outcome refusal with recovery guidance. Git subprocesses have a separate 90-second hard timeout beneath that reply deadline; the child is terminated on every platform, and Unix process-group termination also stops hooks, credential helpers, or other descendants holding inherited pipes. At most 16 deadline workers may remain active, after which new bounded calls are refused before starting.

Use `{scratch}` inside guarded command data/output arguments, such as `--output-dir={scratch}/run-1`, for byproducts that must persist outside the repository. ContextPatch expands only that token to its stable scratch root and refuses traversal outside it. The token cannot select a Python script; use `artifact_python_run` for scratch-hosted Python code.

`read_write_receipts` returns up to 25 recent attempts by default or 100 when requested. Use `interrupted_only: true` after a lost reply. `applied` means the operation returned success, `refused` means it failed with unchanged observed state, `unknown` means it failed after state changed or became unreadable, and `interrupted` means no settlement exists. Current receipt coverage includes `replace_exact`, `write_existing_file_exact_hash`, `delete_untracked_exact`, and the three local commit tools. For an interrupted or unknown file operation, compare the receipt digest with `file_info`; for an interrupted or unknown commit, compare the recorded Git HEAD with `git rev-parse HEAD`.

ContextPatch serializes its MCP mutations for one repository with a cooperative cross-process lock; exact replacement and exact-hash writes also lock the target while rechecking it. A competing ContextPatch mutation is refused before starting. External editors and tools that ignore the advisory lock can still race, so use `expected_sha256` where available and inspect current state after an unknown result.

`run_guarded_command` is not a shell. It accepts an executable name and argument array, runs from a repo-root-confined working directory, allows only documented validation-oriented programs/subcommands, drains stdout/stderr concurrently, times out, redacts probable secret values without hiding ordinary paths or docs, and returns command/cwd/exit-code/duration metadata. It defaults to 120 seconds and remains capped at 600 seconds except for exact `harbor run` commands, which may use up to 3600 seconds. Package-manager script execution is limited to `npm run`/`npm test`, `pnpm run`/`pnpm test`, and `bun run`/`bun test`; install/add/exec-style package-manager commands remain outside this boundary. Project validation can also run repo-relative Python scripts (`python3 scripts/name.py`), `pytest`, `harbor run`, and the exact `references/check-base-image.sh` or `references/check-base-image.sh task` scripts through `base_image_check_run`; `pip`, Docker, arbitrary `python -m`, arbitrary shell scripts, and shell command strings remain refused.

Use `fixture_generator_run` for Dynamo-style fixture creation when the generator should live temporarily inside the repo. The tool runs a repo-relative `.py` script with `python3`, defaults to dry-run, requires `confirm: "run fixture generator"` for execution, allows only declared pre-existing dirty paths, and refuses if the generator leaves changes outside `expected_output_paths` or `expected_output_prefixes`. Afterward, delete the temporary generator with `delete_untracked_exact` before committing if it should not be submitted.

Use `bulk_write_new_files_base64` as a fallback for many binary/text fixture files when direct generator execution is unsuitable. It is create-only, repo-root-confined, bounded to 500 files and 20 MiB decoded total content, and can create missing parents only when `parents: true`.

Use `artifact_delete_exact` to iterate or retire a sidecar helper without accumulating filenames. First call it in its default dry-run mode to obtain the current SHA-256, then pass that digest with `dry_run: false` and `confirm: "delete artifact exact"`. It deletes only that regular file under the fixed artifact root, rejects symlinks and traversal, leaves parent directories intact, and reports that repository state was not mutated.

Use `base_image_check_run` for `references/check-base-image.sh`, and set `project_path: "task"` for Dynamo/Harbor tasks that require `bash references/check-base-image.sh task`. This is the only shell-script-shaped validation exception; it does not make arbitrary `bash` or shell snippets available.

Use `image_cleanliness_check_run` for the Docker image cleanliness gate that checks whether a task image contains a forbidden file such as `solve.sh`. It plans or runs only `docker run --rm --network none --entrypoint find <image> / -name <filename>`, defaults to dry-run, and requires `confirm: "run image cleanliness check"` for execution. Do not ask for generic Docker access for this gate.

Use `write_existing_file_exact_hash` when an existing text file needs whole-file synchronization and exact anchoring by current content hash is easier than a text replacement. It defaults to dry-run and requires `confirm: "write exact hash"` before writing.

Use `fixture_manifest_refresh` after fixture or SPEC changes to regenerate a manifest from explicit `fixture_paths` or `fixture_prefixes`; overwriting an existing manifest requires its current `expected_manifest_sha256`. Use `fixture_manifest_verify` before deriving truth or committing fixture changes; it fails on missing, modified, or unlisted fixture files.

Use `validation_profile_run` when a workflow has a named validation sequence, such as `repo-basic`, `rust-workspace`, `datacore-vscode`, `datacore-m6-vscode`, or `dynamo-harbor-task`. It reduces MCP round trips by running the server-owned allowlisted commands in sequence and returning a compact summary plus `log_id` values; the Dynamo/Harbor profile reads per-trial rewards from Harbor's repository-confined `result.json` and uses rendered output only as a fallback before producing its structured oracle/nop and determinism summary. Use `read_command_log` only for logs that need inspection.

Before launching a profile that depends on optional host tools, call `preflight_health`. It must report availability and resolved paths for validation executables used by shipped profiles, including `python3`, `pytest`, `harbor`, and the exact `bash references/check-base-image.sh` workflow. If Harbor is installed outside the MCP server process `PATH`, configure the host with `CONTEXTPATCH_VALIDATION_PATHS` rather than asking the model to pass an executable path or environment override. If Harbor is unavailable in the current environment, do not claim a completed `harbor run`; verify task-local Oracle/verifier logic directly only as a documented fallback for that environment limitation.

Use `setup_profile_run` when a real project setup task needs a profile-owned command plan. It defaults to dry-run and requires `confirm: "run setup profile"` plus a clean worktree before executing an external setup command. The first supported profile is `node-capacitor-shell`, with typed actions for Capacitor dependency installation, the fixed `@capacitor/filesystem` plugin dependency, init, adding iOS/Android projects, sync, and iOS CocoaPods install. The profile uses pnpm when `pnpm-lock.yaml` is present and otherwise uses npm; it installs Capacitor runtime packages as dependencies and `@capacitor/cli` as a dev dependency. `ios_pod_install` applies only when the cwd contains a `Podfile`; Swift Package Manager based Capacitor projects do not need it. The caller never supplies raw commands, arbitrary package lists, or shell strings.

Use `native_build_run` for native build/test validation. It supports typed iOS actions (`ios_build`, `ios_test`) and Android actions (`android_assemble_debug`, `android_unit_test`), derives exact `xcodebuild` or repo-relative Gradle wrapper plans, defaults to dry-run, and refuses success if an executed build leaves Git source status changed. iOS builds may set repo-relative `derived_data_path` to make the built `.app` available to `ios_install_app`; use a gitignored/cache path so the source-status guard remains clean. `preflight_health` probes Xcode with `xcodebuild -version` plus `xcode-select -p`, so Claude Desktop can distinguish a usable Xcode install from missing Xcode or an unaccepted license before running a full build.

Use `native_device_run` for bounded simulator/emulator/device smoke actions. It supports listing, creating, booting, running a synced Capacitor iOS app on a target simulator, installing, launching, and reading timed log streams for iOS simulators, plus listing/install/launch/logcat actions for Android devices. Use `ios_read_logs` with optional `duration` in seconds such as `5` or `15`; it intentionally does not expose unbounded log streaming. Device-state-changing execution requires `confirm: "run native device"`; raw `xcrun`, `adb`, and package-manager exec access remains unsupported.

The `capability_manifest` includes compact examples for setup, native build, and native device actions. Claude Desktop should prefer those examples over guessing raw command syntax.

Use `git_commit_exact` for the narrow local-commit case that previously required leaving contextpatch entirely: the tool validates that `paths` exactly equals the repository's full dirty-path set, defaults to dry-run, requires `confirm: "commit exact paths"` when `dry_run` is false, stages only those paths, creates one local commit, and reports the commit hash. It still does not run fetch or push.

Use `git_commit_scoped` when the desired local commit is only a subset of dirty paths and unrelated dirty files should remain untouched. It validates that every requested path is dirty, requires the Git index to be clean before staging so pre-staged unrelated work cannot leak into the commit, defaults to dry-run, requires `confirm: "commit scoped paths"` when `dry_run` is false, stages only the requested paths, creates one local commit, and reports the remaining dirty paths. It still does not fetch or push.

Use `git_commit_prefix` when many dirty fixture or generated files live under known prefixes and enumerating each path manually would be noisy. The dry-run expands prefixes to the exact dirty path list and reports unrelated dirty paths that will remain. Execution requires a clean index and `confirm: "commit prefix paths"`, stages only the expanded paths, creates one local commit, and never fetches or pushes.

Commit messages should not include Claude, Anthropic, AI, assistant, or co-authored-by attribution unless the repository owner explicitly asks for that attribution for the specific commit. Treat authored code and commit messages as project-owned work by the configured repository user.

Use `git_restore_exact` to remove generated tracked dirty paths before an exact commit without exposing broad reset, checkout, clean, or stash behavior. It defaults to dry-run, requires `confirm: "restore exact paths"` for mutation, restores only explicit currently-dirty tracked paths from `HEAD`, and reports any remaining dirty paths.

Use `move_tracked` to rename one clean tracked regular file without broad `git mv` access. It defaults to dry-run, requires an absent untracked destination with an existing in-repository parent and a clean index, requires `confirm: "move tracked file"` for mutation, and verifies the destination remains tracked with the same SHA-256.

Use `delete_guarded` to remove one obsolete tracked regular file. Supply its current lowercase SHA-256, review the dry-run, then use `confirm: "delete tracked file"`; the tool refuses dirty, untracked, symlink, and hash-mismatched targets and leaves the deletion unstaged for review.

Use `delete_generated_prefix` for ignored/untracked generated output and empty directories under known prefixes when explicit file deletion would be too brittle. Review the dry-run `would_delete_files` and `would_delete_empty_or_ignored_dirs` lists first. Execution requires `confirm: "delete generated paths"` and refuses tracked paths instead of behaving like `git clean`.

Use `git_remote_check` and `git_push_exact` for the separate remote-publishing boundary. `git_remote_check` fetches one explicit remote branch and reports whether the remote is ahead without source changes. `git_push_exact` requires `confirm: "push exact commit"`, a clean worktree, current branch equal to the requested branch, `expected_head` equal to current `HEAD`, and no remote-ahead divergence after fetch; it pushes only `HEAD:refs/heads/<branch>` and never force-pushes.

Use `git_branch_prepare` for guarded branch setup from a remote base. It requires a clean worktree, fetches only `refs/heads/<base_branch>` into the matching remote-tracking ref, creates or switches to the requested local branch, requires explicit confirmation before resetting an existing branch, verifies the remote base is an ancestor, and can check required files such as split pipeline definitions.

Use `git_merge_readiness` for read-only PR or merge planning. It validates two refs, optionally fetches one explicit target branch, computes the merge base and ahead counts, and reports files changed on both sides as likely conflict candidates. It does not perform a merge, checkout, reset, stash, or source edit.

## Failure behavior

If a tool refuses an edit, Claude Desktop should receive a clear refusal reason. Refusal is a successful safety outcome, not a server failure.
