# Tool Spec

Initial contextpatch tools are intentionally narrow.

This is deliberate: `contextpatch` is a safe patch layer for AI coding agents, not a general filesystem toolbox.

| Tool | Writes? | Guard |
| --- | --- | --- |
| `capability_manifest` | No | Reports exact supported capabilities and unsupported boundaries |
| `preflight_health` | No | Repository/tool readiness summary |
| `read_range` | No | Bounded path and line range |
| `diff_preview` | No | Proposed edit input |
| `replace_exact` | Yes | Old text must match exactly once |
| `apply_patch` | Yes | Unified patch context must apply cleanly |
| `insert_at_anchor` | Yes | Anchor must match exactly once |
| `move_tracked` | Yes | Source exists, destination absent, Git state visible |
| `status_guard` | No | Repository status inspection |
| `write_new_file` | Yes | Destination must not exist |
| `write_new_file_base64` | Yes | Destination must not exist; base64 decoded bytes with optional expected size |
| `create_directory` | Yes | Destination must not exist; optional explicit parent creation inside repo root |
| `run_guarded_command` | No source edits | Repo-root-confined, no-shell, allowlisted validation command |
| `read_command_log` | No | Reads captured guarded-command logs by opaque id |
| `validation_profile_run` | No source edits | Runs predefined allowlisted validation command sequences |
| `setup_profile_run` | External setup command | Dry-run default, clean-worktree and confirmation gates, profile-derived command plan, typed params only, no caller-supplied raw commands |
| `native_build_run` | External build/test command | Dry-run default, typed action params, source-status unchanged after execution, no raw native commands |
| `native_device_run` | External device command | Dry-run default, typed action params, device-state confirmation gate, no raw `xcrun` or `adb` commands |
| `git_commit_exact` | Git index + one local commit | Exact full dirty-path set, dry-run default, explicit confirmation, never pushes |
| `git_commit_scoped` | Git index + one local commit | Explicit dirty subset, clean-index gate, preserves unrelated dirty paths, never pushes |
| `git_restore_exact` | Git worktree/index restore | Explicit dirty tracked paths only, dry-run default, explicit confirmation, never resets the whole worktree |
| `git_remote_check` | Remote-tracking refs only | Fetches one explicit remote branch and reports HEAD/remote divergence without source edits |
| `git_branch_prepare` | Remote-tracking ref + local branch/worktree | Clean worktree, exact remote base fetch, validated branch, optional confirmed reset, required-file gates |
| `git_merge_readiness` | Optional remote-tracking ref update only | Validated refs, optional single-branch fetch, merge-base/ahead/diff-name analysis, no merge |
| `git_push_exact` | Remote branch update | Clean worktree, exact HEAD, no remote-ahead divergence, explicit confirmation, no force |
| `github_pr_run` | GitHub mutation for PR creation only | `gh`-backed auth/check/view plus confirmation-gated PR creation |
| `delete_guarded` | Yes | Expected hash/path confirmation |

## Naming

The public tool names use snake_case because they are protocol-facing. The CLI uses kebab-case commands.

## Tool contracts

### `capability_manifest`

Reports the server's capability contract in machine-readable JSON text.

Required inputs: none.

Rules:

- Must report file tools and process-execution availability honestly.
- Must identify the configured repository root.
- Must state unsupported operations, including arbitrary shell and destructive Git mutations.
- Must distinguish the narrow local `git_commit_exact`/`git_commit_scoped` checkpoints and guarded `git_remote_check`/`git_push_exact` workflow from unsupported broad Git/destructive workflows.
- Must report `git_branch_prepare` honestly when available and distinguish it from broad checkout/rebase/reset authority.
- Must report `git_merge_readiness` honestly when available and distinguish it from generic merge authority.
- Must report setup profiles honestly when available and distinguish declarative profile planning from generic package-manager or shell authority.
- Must include compact examples for typed setup, native build, and native device actions so clients can choose correct params without raw command knowledge.
- Must not mutate repository state.

### `preflight_health`

Reports whether the repository and local validation tools are ready for agent work.

Required inputs: none.

Rules:

- Must report repository cleanliness using the same Git guard semantics as `status_guard`.
- Must report whether guarded process execution is available.
- Must report local availability of expected validation tools without treating missing optional tools as server failure.
- Must report setup-profile prerequisites without running setup commands.
- Must not mutate repository state.

### `read_range`

Reads a bounded section of a UTF-8 text file.

Required inputs:

- `path`
- `start_line`
- `end_line`

CLI shape:

```bash
contextpatch read-range <path> --start <line> --end <line>
```

The CLI treats the current working directory as the repository root guard.

Rules:

- Line numbers are 1-based.
- The tool must refuse paths outside the repository root.
- The tool must return line numbers with the content.
- The tool must not read unbounded files by default.

### `diff_preview`

Returns a unified diff for a proposed edit without writing.

Required inputs:

- `path`
- `old`
- `new`

CLI shape:

```bash
contextpatch diff-preview <path> --old <text> --new <text>
```

The CLI treats the current working directory as the repository root guard.

Rules:

- The tool must not mutate files.
- The diff must be generated from the current file contents.
- The current implementation previews exact replacements; `old` must match exactly once.
- If the proposed edit cannot be validated, return a refusal reason instead of a diff.

### `replace_exact`

Replaces text only when the old text appears exactly once.

Required inputs:

- `path`
- `old`
- `new`

CLI shape:

```bash
contextpatch replace-exact <path> --old <text> --new <text>
```

The CLI treats the current working directory as the repository root guard.

Rules:

- Refuse if `old` is empty.
- Refuse if `old` appears zero times.
- Refuse if `old` appears more than once.
- Write atomically.
- Return the changed byte range or a concise edit summary.

### `apply_patch`

Applies a unified patch with context validation.

Required inputs:

- `patch`
- optional repository root guard

Rules:

- Patch context must apply cleanly.
- Paths must stay inside the repository root.
- Partial application must not leave persistent changes.
- The tool must report changed files.

### `insert_at_anchor`

Inserts content before or after an exact anchor.

Required inputs:

- `path`
- `anchor`
- `position`: `before` or `after`
- `content`

Rules:

- Refuse if the anchor is missing.
- Refuse if the anchor appears more than once.
- Write atomically.

### `move_tracked`

Moves or renames a file with repository guardrails.

Required inputs:

- `from`
- `to`

Rules:

- Refuse if source is missing.
- Refuse if destination exists.
- Prefer `git mv` when the source is tracked.
- Do not overwrite destination paths.

### `status_guard`

Reports repository readiness for edits and refuses when Git status is dirty.

Required inputs:

- optional `path`

CLI shape:

```bash
contextpatch status-guard [path]
```

The CLI treats the current working directory as the repository root guard. `contextpatch status` is an alias.

Rules:

- Return a clean summary when no Git changes are present.
- Refuse when the repository, or optional scoped path, has uncommitted changes.
- Include changed path summaries in the refusal.
- Refuse paths outside the repository root.
- Do not mutate repository state.

### `write_new_file`

Creates a new UTF-8 text file only when the destination does not exist.

Required inputs:

- `path`
- `content`

CLI shape:

```bash
contextpatch write-new-file <path> --content <text>
```

The CLI treats the current working directory as the repository root guard.

Rules:

- Refuse if the file already exists.
- Refuse parent traversal outside the repository root.
- Refuse missing parent directories.
- Write atomically.

### `write_new_file_base64`

Creates a new binary file from base64 only when the destination does not exist.

Required inputs:

- `path`
- `content_base64`

Optional inputs:

- `expected_bytes`: decoded byte-count guard

Rules:

- Refuse if the file already exists.
- Refuse parent traversal outside the repository root.
- Refuse missing parent directories.
- Refuse invalid base64 or decoded content larger than 20 MiB.
- When `expected_bytes` is provided, refuse if the decoded byte count does not match.
- Write atomically.

### `create_directory`

Creates a new directory only when the destination does not exist.

Required inputs:

- `path`

Optional inputs:

- `parents`: defaults to `false`; when `true`, create missing parent directories inside the repository root.

CLI shape:

```bash
contextpatch create-directory <path> [--parents]
```

The CLI treats the current working directory as the repository root guard.

Rules:

- Refuse if the target directory already exists.
- Refuse if any existing path component is a file.
- Refuse parent traversal outside the repository root.
- Refuse missing parent directories unless `parents` is true.
- With `parents: true`, create only the explicit path components needed for the requested directory path.

### `run_guarded_command`

Runs a bounded validation-oriented command without invoking a shell.

Required inputs:

- `program`: one of `git`, `cargo`, `bun`, `npm`, `pnpm`, `python`, `python3`, `pytest`, `harbor`, or `rg`
- `args`: command arguments; the first argument must be an allowlisted subcommand

Optional inputs:

- `cwd`: working directory relative to the configured repository root
- `timeout_secs`: timeout in seconds, from 1 to 600

Rules:

- The command must run without shell interpolation.
- The working directory must resolve inside the configured repository root.
- The executable must be an allowlisted program name, not a path.
- The subcommand must be allowlisted:
  - `git`: `status`, `diff`, `log`, `show`, `rev-parse`, `ls-tree`
  - `cargo`: `check`, `test`, `build`, `clippy`
  - `bun`: `run`, `test`
  - `npm`: `run`, `test`
  - `pnpm`: `run`, `test`
  - `python`/`python3`: a repo-relative `.py` script path as the first argument
  - `pytest`: validation invocation
  - `harbor`: `run`
  - `rg`: search invocation
- Arguments that directly reference paths outside the repository root must be refused.
- The tool must return command, cwd, allowlist rule, exit code, duration, stdout, and stderr.
- Output must redact probable secret values without masking ordinary path-shaped output, env-var names, or documentation prose, then truncate large streams.
- The tool must refuse arbitrary shell, environment inspection, destructive Git commands, package installation, Docker, and automatic commits.

### `read_command_log`

Reads a captured guarded-command log produced by `run_guarded_command` or `validation_profile_run`.

Required inputs:

- `log_id`: opaque id returned by a command-running tool

Optional inputs:

- `max_chars`: maximum characters to return, from 1 to 200000; defaults to 12000

Rules:

- `log_id` must be an opaque command-log id, not a path.
- The tool must only read from the contextpatch command-log directory.
- Logs contain the same redacted command output shape as guarded command responses.
- The tool must truncate large responses rather than returning unbounded JSON-RPC payloads.

### `validation_profile_run`

Runs a predefined sequence of allowlisted validation commands as one MCP call.

Required inputs:

- `profile`: one of `repo-basic`, `rust-workspace`, `datacore-vscode`, or `datacore-m6-vscode`

Optional inputs:

- `timeout_secs`: per-command timeout override, from 1 to 600
- `stop_on_failure`: stop after the first non-zero or timed-out command; defaults to true

Rules:

- Profiles must be explicit server-owned command lists, not user-supplied shell snippets.
- Each command must pass the same `run_guarded_command` allowlist, cwd, timeout, and redaction rules.
- The first response should be compact: per-command status, duration, timeout state, and log id.
- Full command output should be retrieved with `read_command_log` only when needed.
- Profiles must not commit, push, reset, checkout, clean, stash, or mutate product files.

### `setup_profile_run`

Plans or runs a predefined repository setup action from a typed profile.

This tool exists for real project setup cases that need package-manager or scaffold commands without turning `run_guarded_command` into a broad shell/package-manager gateway.

Required inputs:

- `profile`: currently `node-capacitor-shell`
- `action`: profile-owned action name

Optional inputs:

- `params`: typed action parameters
- `cwd`: working directory relative to the repository root
- `timeout_secs`: future execution timeout, from 1 to 600
- `dry_run`: defaults to `true`
- `confirm`: required mutation confirmation literal `run setup profile` when `dry_run` is `false`

Supported `node-capacitor-shell` actions:

- `install_capacitor_dependencies`
- `install_capacitor_filesystem`
- `cap_init` with `app_id`, `app_name`, and `web_dir`
- `cap_add_ios`
- `cap_add_android`
- `cap_sync` with optional `platform`: `ios`, `android`, or `all`
- `ios_pod_install`

Rules:

- The caller must not supply raw `program` or `args`.
- The profile must derive the exact planned command and expected changed-path classes.
- The profile may select npm or pnpm from repository lockfiles, but callers still must not choose arbitrary package-manager commands.
- The `node-capacitor-shell` dependency install action installs `@capacitor/core`, `@capacitor/ios`, and `@capacitor/android` as dependencies, and `@capacitor/cli` as a dev dependency.
- The `install_capacitor_filesystem` action installs only `@capacitor/filesystem`; callers must not supply arbitrary package names.
- `ios_pod_install` must refuse when the cwd has no `Podfile`; Swift Package Manager based Capacitor projects do not need CocoaPods.
- Action params must be typed and validated by the profile.
- The working directory must resolve inside the configured repository root.
- Dry-run output must clearly mark the plan as an external mutator and not claim atomic contextpatch writes.
- `dry_run=false` must require a clean worktree, exact confirmation, before/after status capture, changed-path reporting, and refusal for changed paths outside the profile's expected classes.
- The tool must not broaden the `run_guarded_command` validation allowlist.
- `pnpm` support for `node-capacitor-shell` remains profile-owned setup authority and must not make raw `pnpm add` available through `run_guarded_command`.

### `native_build_run`

Plans or runs a typed native build/test action without exposing raw native build tools.

Required inputs:

- `action`: one of `ios_build`, `ios_test`, `android_assemble_debug`, or `android_unit_test`
- `params`: typed action parameters

Optional inputs:

- `cwd`: working directory relative to the repository root
- `timeout_secs`: timeout from 1 to 600
- `dry_run`: defaults to `true`

iOS params:

- `workspace`: repository-relative `.xcworkspace` or `.xcodeproj`
- `scheme`
- optional `configuration`, default `Debug`
- optional `sdk`, default `iphonesimulator`
- optional `destination`
- optional `derived_data_path`: repository-relative Xcode DerivedData directory, useful when a later `ios_install_app` needs a repository-relative `.app` path

Android params:

- optional `gradlew`: repository-relative Gradle wrapper path, default `gradlew`

Rules:

- The caller must not supply raw `program` or `args`.
- Core must derive exact `xcodebuild` or Gradle wrapper command plans from typed params.
- `preflight_health` must probe Xcode with `xcodebuild -version`, not a generic `--version` flag, and should report `xcode-select -p` so callers can diagnose missing Xcode or unaccepted-license states.
- `derived_data_path`, when supplied, must be repository-relative and is passed as `-derivedDataPath`; use a gitignored/cache path so the source-status guard can still verify the build did not change tracked or visible source state.
- Android Gradle wrapper execution is allowed only through this native build policy and must resolve to a file inside the repository.
- The tool must not broaden the `run_guarded_command` validation allowlist or global executable-name validation.
- Dry-run must not execute native tools.
- Executed builds must compare Git source status before and after execution and refuse if source status changes.
- Native build output is external validator output; the tool must not claim atomic edit semantics for build products or caches.

### `native_device_run`

Plans or runs a typed native simulator/emulator/device action without exposing raw device tools.

Required inputs:

- `action`: one of `ios_list_simulators`, `ios_create_simulator`, `ios_boot_simulator`, `ios_cap_run`, `ios_install_app`, `ios_launch_app`, `ios_read_logs`, `android_list_devices`, `android_install_app`, `android_launch_app`, or `android_read_logcat`

Optional inputs:

- `params`: typed action parameters
- `cwd`: working directory relative to the repository root
- `timeout_secs`: timeout from 1 to 600
- `dry_run`: defaults to `true`
- `confirm`: required literal `run native device` when `dry_run` is `false` for device-state-changing actions

Params by action:

- `ios_create_simulator`: `name`, `device_type`, optional `runtime`
- `ios_cap_run`: `target`
- `ios_boot_simulator`: `device`
- `ios_read_logs`: `device`, optional `duration` in seconds from `1` to `30`
- `ios_install_app`: `device`, `app_path`
- `ios_launch_app`: `device`, `app_id`
- `android_list_devices`: optional `serial`
- `android_install_app`: optional `serial`, `apk_path`
- `android_launch_app`: optional `serial`, `app_id`
- `android_read_logcat`: optional `serial`, optional `lines`

`ios_install_app` requires `app_path` to be repository-relative. For Xcode builds, pair this with `native_build_run`'s `derived_data_path`; for example, a build using `derived_data_path: ".contextpatch-derived-data"` can install `.contextpatch-derived-data/Build/Products/Debug-iphonesimulator/App.app`.

Rules:

- The caller must not supply raw `program` or `args`.
- Core must derive exact `xcrun simctl` or `adb` command plans from typed params.
- `ios_cap_run` must select `pnpm exec cap run ios --target <target> --no-sync` when `pnpm-lock.yaml` is present in cwd, otherwise `npm exec -- cap run ios --target <target> --no-sync`; it is for running already-synced Capacitor iOS projects, not broad package-manager execution.
- `ios_read_logs` must use `log stream --timeout <duration>` so simulator log reads terminate at the platform command level instead of relying on external command timeouts.
- Artifact paths such as `app_path` and `apk_path` must be repository-relative paths.
- Device ids, serials, and app ids must be single-line bounded values with only supported characters.
- Dry-run must not touch simulator, emulator, or device state.
- `dry_run=false` for device-state-changing actions must require exact confirmation `run native device`.
- Device operations must not mutate repository source and must not claim atomic edit semantics.

### `git_commit_exact`

Creates a local Git commit only when the caller provides the exact complete dirty-path set.

Required inputs:

- `paths`: non-empty list of repository-relative paths
- `subject`: single-line commit subject

Optional inputs:

- `body`: commit body/trailers
- `dry_run`: defaults to `true`
- `confirm`: required literal `commit exact paths` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `commit exact paths`.
- `paths` must exactly match the repository's full dirty-path set from Git status, including untracked files. If any dirty path is missing or any extra path is supplied, the tool must refuse.
- Paths must be normalized repository-relative paths and must not use path traversal, absolute paths, NUL bytes, or Git pathspec metacharacters.
- Rename/copy status entries are refused until a dedicated tracked-move workflow exists.
- The tool may run `git add -- <paths>` and one local `git commit`; it must not fetch, pull, push, reset, checkout, stash, clean, or modify remotes.
- Commit subjects and bodies should remain project-owned and must not add Claude, Anthropic, AI, assistant, or co-authored-by attribution unless explicitly requested by the repository owner for that commit.
- The tool must verify that the staged path set exactly matches `paths` before committing.
- On success, the tool returns the commit hash, short hash, committed paths, and post-commit short status.
- Commit failure after staging must be reported explicitly; it must not pretend the commit succeeded.

### `git_commit_scoped`

Creates a local Git commit from an explicit subset of dirty paths while preserving unrelated dirty paths.

Required inputs:

- `paths`: non-empty list of repository-relative dirty paths to include in the commit
- `subject`: single-line commit subject

Optional inputs:

- `body`: commit body/trailers
- `dry_run`: defaults to `true`
- `confirm`: required literal `commit scoped paths` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `commit scoped paths`.
- Every requested path must currently be dirty. Unrelated dirty paths may exist and must remain uncommitted.
- The Git index must be clean before mutation so pre-staged unrelated paths cannot be accidentally included.
- Paths must be normalized repository-relative paths and must not use path traversal, absolute paths, NUL bytes, or Git pathspec metacharacters.
- Rename/copy status entries are refused until a dedicated tracked-move workflow exists.
- The tool may run `git add -- <paths>` and one local `git commit`; it must not fetch, pull, push, reset, checkout, stash, clean, or modify remotes.
- Commit subjects and bodies should remain project-owned and must not add Claude, Anthropic, AI, assistant, or co-authored-by attribution unless explicitly requested by the repository owner for that commit.
- The tool must verify that the staged path set exactly matches `paths` before committing.
- On success, the tool returns the commit hash, short hash, committed paths, and remaining dirty paths.
- Commit failure after staging must be reported explicitly; it must not pretend the commit succeeded.

### `git_restore_exact`

Restores explicit dirty tracked paths from `HEAD` without exposing broad checkout, reset, clean, or stash operations. This is for generated-noise cleanup before exact commits.

Required inputs:

- `paths`: non-empty list of repository-relative dirty tracked paths

Optional inputs:

- `dry_run`: defaults to `true`
- `confirm`: required literal `restore exact paths` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `restore exact paths`.
- Every requested path must currently be dirty, tracked by Git, and normalized under the repository root.
- The tool may restore a subset of dirty paths, but it must not touch any path not explicitly listed.
- The tool must refuse untracked paths because restoring from `HEAD` cannot safely recreate/delete untracked files.
- The tool may run only `git restore --staged --worktree -- <paths>` plus read-only Git status/tracking checks.
- On success, the requested paths must no longer be dirty, and the response must report any remaining dirty paths.

### `git_remote_check`

Fetches one explicit remote branch and reports whether the local `HEAD` is behind that remote branch.

Required inputs:

- `branch`: branch name to check

Optional inputs:

- `remote`: remote name; defaults to `origin`

Rules:

- The tool may run only `git fetch <remote> <branch>` plus read-only Git queries.
- The tool must reject malformed remote or branch names.
- The tool must not modify source files or the Git index. It may update remote-tracking refs as the explicit purpose of the tool.
- The response must include `head`, `remote_head`, `remote_ref`, `head_to_remote_empty`, `remote_ahead_count`, and `local_ahead_count`.
- The tool must report whether source status changed during the fetch and refuse if source status changes.

### `git_branch_prepare`

Prepares and switches to one local branch from one explicit remote base branch.

Required inputs:

- `base_branch`: remote branch name to fetch and prepare from
- `branch`: local branch name to create or switch to

Optional inputs:

- `remote`: remote name; defaults to `origin`
- `required_files`: repository-relative files that must exist after preparation
- `reset_existing`: defaults to `false`
- `confirm`: required literal `reset branch from remote base` when `reset_existing` is `true`

Rules:

- The tool must require a clean worktree before fetching or switching branches.
- The tool may fetch only `refs/heads/<base_branch>:refs/remotes/<remote>/<base_branch>`.
- Remote, base branch, local branch, and required-file paths must be validated before Git mutation.
- If the local branch does not exist, the tool may create it from the fetched remote-tracking ref and switch to it.
- If the local branch exists and `reset_existing` is false, the fetched remote-tracking ref must already be an ancestor of that branch before switching.
- If the local branch exists and `reset_existing` is true, the tool must require `confirm: "reset branch from remote base"` before recreating that branch from the fetched remote-tracking ref.
- Required files must be checked against the target ref before switching/resetting and checked again in the worktree after preparation.
- The response must include the action taken, previous branch, current branch, remote ref, remote commit, prepared head, ancestor check, required-file results, and post-preparation status.
- The tool must not expose generic checkout, rebase, merge, merge-tree, stash, clean, arbitrary reset, arbitrary fetch, push, or multi-branch operations.

### `git_merge_readiness`

Performs read-only merge/PR readiness analysis between two validated refs.

Required inputs:

- `base_ref`: base ref, remote-tracking ref, `HEAD`, or commit hash
- `target_ref`: target ref, remote-tracking ref, `HEAD`, or commit hash

Optional inputs:

- `fetch`: defaults to `false`
- `remote`: remote name used only when `fetch` is true; defaults to `origin`
- `target_branch`: remote branch to fetch when `fetch` is true; inferred from `target_ref` when `target_ref` starts with `<remote>/` or `refs/remotes/<remote>/`

Rules:

- The tool must validate refs before invoking Git and reject ref syntax such as whitespace, pathspec metacharacters, revision operators, or ref traversal.
- If `fetch` is true, the tool may fetch only `refs/heads/<target_branch>:refs/remotes/<remote>/<target_branch>` for the validated remote and branch.
- The tool must refuse if source status changes during fetch.
- The tool may run read-only `merge-base`, `rev-list --count`, and `diff --name-only -z` queries after ref resolution.
- The response must include resolved commits, merge base, ahead counts, changed-file counts, `changed_on_both_sides`, and `likely_conflict_candidates`.
- `likely_conflict_candidates` is a conservative file-level intersection, not a guarantee that Git merge content conflicts exist.
- The tool must not checkout, merge, run merge-tree as a generic command, reset, stash, clean, edit source files, stage files, commit, or push.

### `git_push_exact`

Pushes exactly the current branch `HEAD` to the matching branch on an explicit remote.

Required inputs:

- `remote`
- `branch`
- `expected_head`
- `confirm`: literal `push exact commit`

Rules:

- The tool must require a clean worktree before pushing.
- The current branch must exactly match `branch`.
- The current `HEAD` must match `expected_head`.
- The tool must fetch `<remote> <branch>` immediately before the push.
- The remote-tracking ref must not be ahead of `HEAD`, and it must be an ancestor of `HEAD`; divergent or non-fast-forward states must be refused.
- The push refspec must be exactly `HEAD:refs/heads/<branch>` for the requested branch.
- The tool must not use force push, delete refs, push tags, push multiple branches, or push a different local branch.
- On success, the response must include the pushed commit hash, branch, remote, previous remote head, refspec, and post-push status.

### `github_pr_run`

Runs narrow GitHub pull-request workflows through `gh` without exposing arbitrary `gh` command execution.

Required inputs:

- `action`: one of `auth_status`, `pr_view`, `pr_checks`, or `pr_create`

Action-specific inputs:

- `pr_view` and `pr_checks`: `number`
- `pr_create`: `base`, `head`, `title`, `body`

Optional inputs:

- `draft`: create a draft PR when `action` is `pr_create`
- `dry_run`: defaults to true for `pr_create`
- `confirm`: literal `create pull request` when `pr_create` uses `dry_run: false`

Rules:

- The tool must use the GitHub CLI as `gh` with explicit argv, no shell.
- Read-only actions may execute directly.
- `pr_create` must default to dry-run and return the command plan without mutating GitHub.
- `pr_create` with `dry_run: false` must require the exact confirmation literal.
- The tool must not expose arbitrary `gh` subcommands, repository deletion, workflow dispatch, release publishing, secret access, or forceful Git operations.

### Latency instrumentation

Future Stage 2B command and workflow tools should expose optional timing metadata. The metadata should be diagnostic, not part of the safety decision itself.

Recommended fields:

- `request_received_to_dispatch_ms`
- `argument_validation_ms`
- `allowlist_validation_ms`
- `child_spawn_ms`
- `child_runtime_ms`
- `stdout_stderr_drain_ms`
- `redaction_truncation_ms`
- `log_write_ms`
- `response_bytes`
- `total_tool_ms`

Rules:

- Timing metadata must not include command output, arguments classified as secret values, or environment values.
- Timings should be monotonic-duration measurements, not wall-clock timestamps.
- Tools should keep compact summaries as the default and use `read_command_log` for large output details.
- The instrumentation goal is p50/p95/p99 diagnosis across long sessions, especially to distinguish MCP transport overhead from process/output handling.

### `delete_guarded`

Deletes a file only with explicit confirmation.

Required inputs:

- `path`
- expected file hash or equivalent exact confirmation

Rules:

- Refuse directories.
- Refuse missing confirmation.
- Refuse hash mismatch.
- Report the deleted path.

## MVP implementation subset

Stage 1 ships:

1. `replace_exact`
2. `read_range`
3. `write_new_file`
4. `write_new_file_base64`
5. `create_directory`
6. `diff_preview`
7. `status_guard`
8. `capability_manifest`
9. `preflight_health`
10. `run_guarded_command`
11. `read_command_log`
12. `validation_profile_run`
13. `git_commit_exact`
14. `git_commit_scoped`
15. `git_remote_check`
16. `git_branch_prepare`
17. `git_merge_readiness`
18. `git_push_exact`
19. `github_pr_run`
20. `setup_profile_run`
21. `native_build_run`
22. `native_device_run`

The remaining tools stay documented as planned Stage 2 boundaries until implemented. See `docs/implementation-roadmap.md`.
