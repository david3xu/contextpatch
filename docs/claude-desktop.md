# Claude Desktop

`contextpatch` is intended to run as a local context server for desktop agent clients.

## Planned behavior

The server should expose safe edit and validation tools rather than generic filesystem writes or broad shell access. Claude Desktop or another client can request bounded reads, preview diffs, apply guarded edits, discover capabilities, run preflight health checks, and run allowlisted validation commands.

## Server boundary

The server should not expose broad filesystem write tools. In particular, the default server must not expose:

- generic `write_file`
- unrestricted delete
- recursive directory writes
- shell execution
- Git reset/checkout/stash/fetch/push
- Git merge or merge-tree as generic commands
- broad or automatic Git commits
- generic package-manager or scaffold execution
- generic native build, simulator, emulator, or device command execution

The expected agent workflow is:

1. Use `read_range` to inspect a bounded file section.
2. Use `diff_preview` before `replace_exact` when reviewing exact anchored edits.
3. Use `status_guard` before writes when a clean repository or clean target path is required.
4. Use `create_directory` for create-only directory creation, then `write_new_file` or `write_new_file_base64` for create-only repository file creation inside that directory.
5. Use `capability_manifest` and `preflight_health` to determine whether this server can support the current workflow.
6. Use `run_guarded_command` only for allowlisted validation commands such as `git status`, `git diff`, `cargo check`, project `bun run` checks, repo-relative Python scripts, `pytest`, `harbor run`, the exact `references/check-base-image.sh` check, or `rg` drift searches.
7. Use `git_commit_exact` only when the desired local commit path set is explicit and complete.
8. Use `git_commit_scoped` when the desired commit paths are explicit but unrelated dirty files should be preserved for later work. It requires a clean index before staging the requested paths.
9. When committing, use project-owned commit messages only. Do not add Claude, Anthropic, AI, assistant, or co-authored-by attribution unless the repository owner explicitly asks for it for that specific commit.
10. Use `artifact_write_text` or `artifact_write_base64` for generator, trap-check, or other sidecar files that must not enter the repository tree.
11. Use `bulk_write_new_files_base64` for bounded create-only fixture-tree imports when a generator cannot be run directly.
12. Use `fixture_generator_run` for repo-relative Python fixture generators that need declared output mutation; it supports temporary untracked generator scripts and verifies all changed paths stay within declared outputs.
13. Use `base_image_check_run` for the exact `references/check-base-image.sh` validation instead of asking for generic shell.
14. Use `delete_untracked_exact` only for explicit untracked regular files that would otherwise fail clean-worktree or no-extraneous-files gates.
15. Use `git_remote_list` to inspect whether `origin` and fork/upstream remotes are configured before publishing.
16. Use `git_remote_check` before publishing to fetch one explicit remote branch and inspect whether the remote is ahead.
17. Use `git_branch_prepare` to create or switch to a local branch from one explicit remote base branch after a clean-worktree check.
18. Use `git_merge_readiness` before PR/merge work when the question is whether two refs changed the same files since their merge base.
19. Use `git_push_exact` only after a clean local exact commit, matching branch, matching expected HEAD, no remote-ahead divergence, and explicit confirmation.
20. Use `setup_profile_run` for server-owned setup profiles instead of broad `npm install`, `pnpm add`, `npx`, or shell execution; keep `dry_run` enabled until the plan is reviewed.
21. Use `github_pr_run` for GitHub PR auth/status/check/view/create workflows instead of asking for arbitrary `gh` access. Keep PR creation in dry-run until the title, body, base, and head are reviewed.
22. Use `github_fork_prepare` to plan or run `gh repo fork` with explicit confirmation instead of arbitrary `gh repo` commands.
23. Use `native_build_run` for typed iOS/Android build and test actions instead of raw `xcodebuild` or `./gradlew`.
24. Use `native_device_run` for typed simulator/emulator/device smoke actions instead of raw `xcrun` or `adb`; keep `dry_run` enabled until the plan is reviewed.

## Build and configure Claude Desktop

Build the release server:

```bash
cargo build --release -p server --bin contextpatch-server
```

Then point Claude Desktop at the compiled binary. Use absolute paths because Claude Desktop does not inherit your shell's current directory:

```json
{
  "mcpServers": {
    "contextpatch": {
      "command": "/absolute/path/to/contextpatch/target/release/contextpatch-server",
      "args": [
        "--repo-root",
        "/absolute/path/to/repo"
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

Restart Claude Desktop after changing the config or rebuilding the server binary.

## Local development command

Before packaging, the server can be launched from the workspace with:

```bash
cargo run -p server --bin contextpatch-server -- --repo-root /path/to/repo
```

After installation, the intended command is:

```bash
contextpatch-server --repo-root /path/to/repo
```

The workspace package is named `server`; the installed binary remains `contextpatch-server` to avoid colliding with generic commands.

## Quick MCP smoke test

After restarting Claude Desktop, ask it to list available `contextpatch` tools. It should see:

- `read_range`
- `diff_preview`
- `replace_exact`
- `status_guard`
- `write_new_file`
- `write_new_file_base64`
- `artifact_write_text`
- `artifact_write_base64`
- `bulk_write_new_files_base64`
- `create_directory`
- `capability_manifest`
- `preflight_health`
- `run_guarded_command`
- `fixture_generator_run`
- `base_image_check_run`
- `read_command_log`
- `validation_profile_run`
- `setup_profile_run`
- `native_build_run`
- `native_device_run`
- `git_commit_exact`
- `git_commit_scoped`
- `git_restore_exact`
- `delete_untracked_exact`
- `git_remote_list`
- `git_remote_check`
- `git_branch_prepare`
- `git_merge_readiness`
- `git_push_exact`
- `github_pr_run`
- `github_fork_prepare`

If Claude Desktop lists fewer tools than this, the server-side build is not the issue: the rebuilt release binary advertises all thirty-one tools. Treat a partial list as a Claude Desktop session/configuration problem. Fully quit and restart Claude Desktop, confirm the MCP config points at the rebuilt binary:

```text
/Users/291928k/Developer/contextpatch/target/release/contextpatch-server
```

Then list contextpatch tools again. If the list is still partial, the chat is likely connected to an older binary/config entry or Claude Desktop cached a partial MCP tool list for that session; start a fresh chat after restart.

## Currently exposed tools

The current server exposes the implemented safe primitives:

- `read_range`
- `diff_preview`
- `replace_exact`
- `status_guard`
- `write_new_file`
- `write_new_file_base64`
- `artifact_write_text`
- `artifact_write_base64`
- `bulk_write_new_files_base64`
- `create_directory`
- `capability_manifest`
- `preflight_health`
- `run_guarded_command`
- `fixture_generator_run`
- `base_image_check_run`
- `read_command_log`
- `validation_profile_run`
- `setup_profile_run`
- `native_build_run`
- `native_device_run`
- `git_commit_exact`
- `git_commit_scoped`
- `git_restore_exact`
- `delete_untracked_exact`
- `git_remote_list`
- `git_remote_check`
- `git_branch_prepare`
- `git_merge_readiness`
- `git_push_exact`
- `github_pr_run`
- `github_fork_prepare`

Other documented boundary tools remain roadmap items until implemented.

`run_guarded_command` is not a shell. It accepts an executable name and argument array, runs from a repo-root-confined working directory, allows only documented validation-oriented programs/subcommands, drains stdout/stderr concurrently, times out, redacts probable secret values without hiding ordinary paths or docs, and returns command/cwd/exit-code/duration metadata. Package-manager script execution is limited to `npm run`/`npm test`, `pnpm run`/`pnpm test`, and `bun run`/`bun test`; install/add/exec-style package-manager commands remain outside this boundary. Project validation can also run repo-relative Python scripts (`python3 scripts/name.py`), `pytest`, `harbor run`, and the exact `references/check-base-image.sh` script through `base_image_check_run`; `pip`, Docker, arbitrary `python -m`, arbitrary shell scripts, and shell command strings remain refused.

Use `fixture_generator_run` for Dynamo-style fixture creation when the generator should live temporarily inside the repo. The tool runs a repo-relative `.py` script with `python3`, defaults to dry-run, requires `confirm: "run fixture generator"` for execution, allows only declared pre-existing dirty paths, and refuses if the generator leaves changes outside `expected_output_paths` or `expected_output_prefixes`. Afterward, delete the temporary generator with `delete_untracked_exact` before committing if it should not be submitted.

Use `bulk_write_new_files_base64` as a fallback for many binary/text fixture files when direct generator execution is unsuitable. It is create-only, repo-root-confined, bounded to 500 files and 20 MiB decoded total content, and can create missing parents only when `parents: true`.

Use `base_image_check_run` for `references/check-base-image.sh`. This is the only shell-script-shaped validation exception; it does not make arbitrary `bash` or shell snippets available.

Use `validation_profile_run` when a workflow has a named validation sequence, such as `repo-basic`, `rust-workspace`, `datacore-vscode`, or `datacore-m6-vscode`. It reduces MCP round trips by running the server-owned allowlisted commands in sequence and returning a compact summary plus `log_id` values. Use `read_command_log` only for logs that need inspection.

Use `setup_profile_run` when a real project setup task needs a profile-owned command plan. It defaults to dry-run and requires `confirm: "run setup profile"` plus a clean worktree before executing an external setup command. The first supported profile is `node-capacitor-shell`, with typed actions for Capacitor dependency installation, the fixed `@capacitor/filesystem` plugin dependency, init, adding iOS/Android projects, sync, and iOS CocoaPods install. The profile uses pnpm when `pnpm-lock.yaml` is present and otherwise uses npm; it installs Capacitor runtime packages as dependencies and `@capacitor/cli` as a dev dependency. `ios_pod_install` applies only when the cwd contains a `Podfile`; Swift Package Manager based Capacitor projects do not need it. The caller never supplies raw commands, arbitrary package lists, or shell strings.

Use `native_build_run` for native build/test validation. It supports typed iOS actions (`ios_build`, `ios_test`) and Android actions (`android_assemble_debug`, `android_unit_test`), derives exact `xcodebuild` or repo-relative Gradle wrapper plans, defaults to dry-run, and refuses success if an executed build leaves Git source status changed. iOS builds may set repo-relative `derived_data_path` to make the built `.app` available to `ios_install_app`; use a gitignored/cache path so the source-status guard remains clean. `preflight_health` probes Xcode with `xcodebuild -version` plus `xcode-select -p`, so Claude Desktop can distinguish a usable Xcode install from missing Xcode or an unaccepted license before running a full build.

Use `native_device_run` for bounded simulator/emulator/device smoke actions. It supports listing, creating, booting, running a synced Capacitor iOS app on a target simulator, installing, launching, and reading timed log streams for iOS simulators, plus listing/install/launch/logcat actions for Android devices. Use `ios_read_logs` with optional `duration` in seconds such as `5` or `15`; it intentionally does not expose unbounded log streaming. Device-state-changing execution requires `confirm: "run native device"`; raw `xcrun`, `adb`, and package-manager exec access remains unsupported.

The `capability_manifest` includes compact examples for setup, native build, and native device actions. Claude Desktop should prefer those examples over guessing raw command syntax.

Use `git_commit_exact` for the narrow local-commit case that previously required leaving contextpatch entirely: the tool validates that `paths` exactly equals the repository's full dirty-path set, defaults to dry-run, requires `confirm: "commit exact paths"` when `dry_run` is false, stages only those paths, creates one local commit, and reports the commit hash. It still does not run fetch or push.

Use `git_commit_scoped` when the desired local commit is only a subset of dirty paths and unrelated dirty files should remain untouched. It validates that every requested path is dirty, requires the Git index to be clean before staging so pre-staged unrelated work cannot leak into the commit, defaults to dry-run, requires `confirm: "commit scoped paths"` when `dry_run` is false, stages only the requested paths, creates one local commit, and reports the remaining dirty paths. It still does not fetch or push.

Commit messages should not include Claude, Anthropic, AI, assistant, or co-authored-by attribution unless the repository owner explicitly asks for that attribution for the specific commit. Treat authored code and commit messages as project-owned work by the configured repository user.

Use `git_restore_exact` to remove generated tracked dirty paths before an exact commit without exposing broad reset, checkout, clean, or stash behavior. It defaults to dry-run, requires `confirm: "restore exact paths"` for mutation, restores only explicit currently-dirty tracked paths from `HEAD`, and reports any remaining dirty paths.

Use `git_remote_check` and `git_push_exact` for the separate remote-publishing boundary. `git_remote_check` fetches one explicit remote branch and reports whether the remote is ahead without source changes. `git_push_exact` requires `confirm: "push exact commit"`, a clean worktree, current branch equal to the requested branch, `expected_head` equal to current `HEAD`, and no remote-ahead divergence after fetch; it pushes only `HEAD:refs/heads/<branch>` and never force-pushes.

Use `git_branch_prepare` for guarded branch setup from a remote base. It requires a clean worktree, fetches only `refs/heads/<base_branch>` into the matching remote-tracking ref, creates or switches to the requested local branch, requires explicit confirmation before resetting an existing branch, verifies the remote base is an ancestor, and can check required files such as split pipeline definitions.

Use `git_merge_readiness` for read-only PR or merge planning. It validates two refs, optionally fetches one explicit target branch, computes the merge base and ahead counts, and reports files changed on both sides as likely conflict candidates. It does not perform a merge, checkout, reset, stash, or source edit.

## Failure behavior

If a tool refuses an edit, Claude Desktop should receive a clear refusal reason. Refusal is a successful safety outcome, not a server failure.
