# contextpatch

The safe patch layer for AI coding agents.

`contextpatch` is a Rust tool for safe, reviewable repository edits. It is designed for agent workflows where broad filesystem writes are too risky, whole-file rewrites are too unreliable, and every persistent change should be easy to audit.

The project principle is simple: **every write must be anchored, atomic, and reviewable**.

## Product thesis

AI agents should not get broad filesystem write power by default. They should get small, guarded, reviewable edit primitives.

`contextpatch` aims to be the safe patch layer between AI coding agents and a repository. It should be useful as a CLI, a local context server, and a reusable edit engine for future editor or agent integrations.

## Why this exists

AI desktop tools often expose generic filesystem writes. That is convenient, but it can fail poorly: large file rewrites, timeout-prone operations, accidental overwrites, and weak protection around concurrent repository changes.

`contextpatch` takes the opposite approach. It should prefer small guarded edit operations over broad writes.

## Product boundary

`contextpatch` is not a general filesystem server. It is not a shell runner. It is not a replacement for Git.

The project owns one narrow surface: **safe repository editing and validation primitives for agent clients**. Anything outside that boundary should be rejected unless it directly supports anchored edits, reviewable diffs, validation, or repository guardrails.

That boundary is intentional product strategy, not a temporary limitation.

## MVP scope

| Capability | Purpose |
| --- | --- |
| `read-range` | Read a bounded section of a file |
| `diff-preview` | Preview a proposed edit before writing |
| `replace-exact` | Replace text only when the expected anchor appears exactly once |
| `write-new-file` | Create a file only when it does not already exist |
| `status-guard` | Refuse when Git state is dirty before edits |
| `serve` | Run a local context-server interface for agent tools |

## First implementation order

1. `replace-exact` in `core`
2. `replace-exact` in `cli`
3. `read-range` in core and CLI
4. `write-new-file`
5. `diff-preview`
6. `status-guard`
7. Stage 1 server schemas and transport

The first useful milestone is a CLI command that can safely replace exactly one matched text span and refuse zero-match or multi-match edits. The full staged plan is in [docs/implementation-roadmap.md](docs/implementation-roadmap.md).

```bash
contextpatch read-range <path> --start <line> --end <line>
contextpatch diff-preview <path> --old <text> --new <text>
contextpatch replace-exact <path> --old <text> --new <text>
contextpatch write-new-file <path> --content <text>
contextpatch status-guard [path]
contextpatch configure-claude-desktop [--config <path>] [--dry-run] \
  [--tool-surface <project|full>]
```

The server supports two public MCP surfaces over the same guarded actions:

- `full` is the server default and advertises every action as a direct MCP tool.
- `project` advertises one stable `project_execute` wrapper for the fixed `--repo-root` trust
  boundary. Use `action: "describe"` to list actions or retrieve one action's exact schema, then
  execute one action per call with its original arguments. If `--repo-root` is a non-Git workspace,
  a wrapper-level `repository` may select a normalized workspace-relative path that is itself an
  exact descendant Git worktree root. Omitting it preserves the configured root. The wrapper derives
  that effective root before deadlines, mutation locks, handlers, logs, and receipts are selected.

The full surface exposes these direct actions; project mode keeps the same set behind
`project_execute`:

- `capability_manifest`
- `preflight_health`
- `read_range`
- `read_write_receipts`
- `diff_preview`
- `replace_exact`
- `bulk_replace_exact`
- `write_new_file`
- `write_new_file_base64`
- `write_existing_file_exact_hash`
- `file_info`
- `set_file_executable`
- `list_directory`
- `read_file_bytes`
- `artifact_write_text`
- `artifact_write_base64`
- `artifact_delete_exact`
- `bulk_write_new_files_base64`
- `create_directory`
- `status_guard`
- `run_guarded_command`
- `artifact_python_run`
- `task_image_python_run`
- `harbor_run_start`
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
- `git_commit_scoped`
- `git_commit_prefix`
- `git_stage_exact`
- `git_staged_scope_check`
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

Run `contextpatch configure-claude-desktop` to validate detected ContextPatch entries in the normal `claude_desktop_config.json` `mcpServers` map and default them to `--tool-surface project`. The command preserves repository roots, unrelated arguments, configuration, and user-authored policies, and removes only the exact legacy ContextPatch wildcard `toolPolicy: {"*":"allow"}` from targeted entries. It uses a cooperative lock and stale-read check and creates an exact backup before an update. `--dry-run` leaves the config unchanged and creates no backup; `--tool-surface full` restores the direct-tool surface.

The local ContextPatch MCP connection requires no authentication. Claude Desktop owns runtime tool authorization: ordinary MCP configuration and local Desktop Extension metadata cannot preapprove tools. Project mode reduces the normal persistent approval scope from one identity per direct tool to one stable `project_execute` identity per configured project; it cannot eliminate an approval Claude requires. These client-side approval choices never weaken ContextPatch's dry-run defaults, exact confirmations, path checks, Git-state gates, deadlines, locks, receipts, or refusals.

`run_guarded_command` is Stage 2 MCP-only validation support: it runs no shell, stays repo-root-confined, allows only selected validation-oriented programs/subcommands, drains stdout/stderr concurrently, applies a timeout, redacts probable secret values without hiding ordinary paths or docs, and reports command/cwd/exit-code/duration metadata. The default timeout is 120 seconds and the maximum is 600 seconds. It supports repo-relative Python scripts, `pytest`, and the exact `references/check-base-image.sh` or `references/check-base-image.sh task` base-image checks for project validation; it refuses direct `harbor run` in favor of `harbor_run_start`, along with Docker, `pip`, arbitrary `python -m`, arbitrary shell scripts, and generic package installation.

Operational support is discoverable through `capability_manifest`: it reports build provenance, a stable repository-specific scratch root, reply deadlines of 30 seconds for reads, 60 seconds for direct writes, and 120 seconds for Git and GitHub operations, plus transport and background-job limits. Git subprocesses use a shorter 90-second hard timeout so the child is terminated before the reply deadline; on Unix the process group also stops hooks, credential helpers, or other descendants holding inherited pipes. At most 16 deadline workers may remain active; later calls are refused before starting. Long tool calls no longer block independent requests, so MCP replies may arrive out of request order and clients must correlate them by JSON-RPC id. Guarded command data/output arguments may use `{scratch}` for byproducts outside the repository, but Python script selection stays repository-relative; use `artifact_python_run` for scratch-hosted code. ContextPatch mutations for one repository are cooperatively serialized across server processes, while SHA-guarded file edits also hold a per-target lock across verification and write. External programs that ignore those advisory locks remain outside the guarantee.

When a bounded mutation reply is interrupted, `read_write_receipts` reports durable before/after file digests or Git HEAD values for receipt-enabled exact replacements, exact-hash overwrites, untracked-file deletions, and local commit workflows. Outcomes distinguish `applied`, `refused`, `unknown` (state changed or became unreadable after an error), and `interrupted` (no settlement record). `replace_exact` also accepts an optional complete-file `expected_sha256` so shared-worktree callers can refuse a stale edit before mutation.

`file_info`, `list_directory`, and `read_file_bytes` provide bounded inspection for batched file metadata, depth-limited directory trees, symlink status, digests, line counts, and binary byte ranges without relying on ad hoc scripts. `set_file_executable` changes only Unix executable bits after a dry run and exact current hash/mode confirmation. `write_new_file_base64` creates binary repository files with the same create-only root guard as text file creation. `write_existing_file_exact_hash` covers whole-file synchronization only when the caller supplies the current lowercase SHA-256 digest and explicit confirmation. `bulk_write_new_files_base64` imports many create-only fixture files in one bounded call. `fixture_manifest_verify` and `fixture_manifest_refresh` enforce exact fixture file sets and SHA-256 digests against a manifest. `artifact_write_text` and `artifact_write_base64` create sidecar artifacts outside the repository tree for generators, trap checks, or local helper files that must not be committed. `artifact_delete_exact` removes one exact regular sidecar file only after a digest-reporting dry run, matching SHA-256, and explicit confirmation; it never removes parent directories or repository files. `artifact_python_run` executes a Python script from that sidecar artifact root with no shell and repo-root-confined cwd, so scratch analysis does not have to create temporary files in the repository. Confirmed `task_image_python_run` execution starts the fixed task-image build and repo-relative Python script asynchronously, returning a `log_id` while preserving the read-only mount, disabled network, dropped capabilities, bounded output, and no generic Docker arguments. `fixture_generator_run` handles the common Dynamo pattern of temporarily staging a repo-relative Python generator, running it with declared output paths/prefixes, verifying it did not touch undeclared files, and then letting the caller delete the temporary generator before commit.

`validation_profile_run`, confirmed `task_image_python_run`, and `harbor_run_start` start asynchronously and immediately return stable log ids. They share a two-background-job limit; poll each id on the same server with `read_command_log` instead of restarting work. For `dynamo-harbor-task`, the terminal profile log reads per-trial rewards from Harbor's authoritative `result.json` before using rendered output as a compatibility fallback, then appends oracle rewards, nop rewards, determinism checks, and a single pass/fail field. Harbor terminal logs include lifecycle status plus bounded reward, trial-path, and verifier evidence. A job left active across a server restart reports `unknown` rather than being silently restarted. `read_command_log` retrieves redacted logs on demand, with offset paging, so large outputs do not have to ride in the first JSON-RPC response. Hosts can add validation executables that are not on the server `PATH` through `CONTEXTPATCH_VALIDATION_PATHS`; `preflight_health` reports resolved validation-tool paths in its bounded `full` mode and also supports `compact` and `minimal` projections. Every tool response has a 900 KiB serialized envelope below the client's 1 MiB ceiling. `image_cleanliness_check_run` covers the narrow Docker image-cleanliness check by planning or running only `docker run --rm --network none --entrypoint find <image> / -name <filename>`, defaulting to `solve.sh`. `docker_image_inspect` adds only the read-only `docker image inspect <image>` workflow and keeps generic Docker arguments out of scope.

`setup_profile_run` is a declarative setup-profile tool for real project scaffolding needs. It defaults to dry-run, can run typed server-owned actions such as the `node-capacitor-shell` profile only with `confirm: "run setup profile"` and a clean worktree, and does not expose arbitrary `npm install`, `pnpm add`, `npx`, package lists, or shell commands through `run_guarded_command`. The Capacitor profile derives npm or pnpm command plans from project lockfiles and installs `@capacitor/cli` as a dev dependency.

`native_build_run` and `native_device_run` cover native plugin/build workflows without exposing raw `xcodebuild`, `./gradlew`, `xcrun`, or `adb` authority. Agents select typed actions such as `ios_build`, `android_assemble_debug`, `ios_launch_app`, or `android_read_logcat`; contextpatch derives the exact command plan, defaults to dry-run, validates repo-relative artifacts, and requires `confirm: "run native device"` before executing device-state-changing actions.

`git_stage_exact` stages explicit dirty paths without committing, preserving the split between "track this file" and "create a validated checkpoint." `git_staged_scope_check` is the read-only audit partner: it reports whether currently staged paths are limited to allowed exact paths/prefixes and whether required paths are staged. `git_commit_exact` is a narrowly gated local Git checkpoint tool. It defaults to dry-run, requires the provided path list to exactly match the full dirty-path set, requires `confirm: "commit exact paths"` before mutation, stages only those paths, creates at most one local commit, and never fetches or pushes. `git_commit_scoped` covers the related case where unrelated dirty files must be preserved; it requires a clean index and stages only the explicit dirty subset. `git_commit_prefix` is the practical large-fixture variant: it expands explicit prefixes to the exact dirty paths underneath them, reports the expanded set in dry-run, then commits only that expanded set with `confirm: "commit prefix paths"`.

`git_restore_exact`, `delete_untracked_exact`, and `delete_generated_prefix` cover generated-noise cleanup without exposing broad reset/checkout/clean behavior. Exact tools operate only on explicit requested paths. Prefix cleanup expands explicit prefixes only to ignored/untracked files and empty directories, refuses tracked paths, and defaults to dry-run/confirmation-gated mutation.

`move_tracked` and `delete_guarded` cover the tracked-file operations that cleanup tools intentionally refuse. A move requires one clean tracked regular file, an absent untracked destination with an existing in-repository parent, a clean index, dry-run review, and `confirm: "move tracked file"`; it uses `git mv` and verifies content-hash preservation. A deletion requires the current lowercase SHA-256 of one clean tracked regular file, defaults to dry-run, and requires `confirm: "delete tracked file"`; it removes only the worktree file and leaves a reviewable unstaged tracked deletion.

`git_remote_list`, `git_remote_check`, and `git_push_exact` keep remote publishing separate from local commit authority. `git_remote_list` parses `git remote -v` without mutation. `git_remote_check` fetches one explicit remote branch and reports whether `HEAD..remote/branch` is empty without changing source files. `git_push_exact` requires a clean worktree, matching current branch, expected HEAD hash, no remote-ahead divergence after fetch, `confirm: "push exact commit"`, and pushes only `HEAD:refs/heads/<branch>` without force.

`git_branch_prepare` handles the narrow branch-prep case for agent workflows. It defaults to a non-fetching dry-run that validates current state and reports exact Git argv; `dry_run: false` fetches one explicit remote base branch from a clean worktree, creates or switches to the requested local branch, optionally resets an existing local branch only with `confirm: "reset branch from remote base"`, verifies the remote base is an ancestor, and checks required files. It does not expose broad checkout, rebase, merge, stash, clean, or arbitrary fetch.

`git_merge_readiness` fills the PR/merge planning gap without exposing `git merge` or `merge-tree`: it optionally fetches one explicit remote branch, resolves two validated refs, reports ahead counts, and returns files changed on both sides since the merge base as likely conflict candidates. It does not checkout, merge, reset, stash, or edit source files.

`github_pr_run` and `github_fork_prepare` cover GitHub review workflow gaps without arbitrary `gh` access. PR/check/run reads can target a validated `OWNER/REPO`, so a fork checkout can inspect upstream evidence. PR details expose the exact head SHA; bounded Actions runs can then be discovered for that commit, sticky findings can be retrieved through a local comment-body filter, and individual workflow runs or redacted, bounded job logs can be read by database id. Job logs default to a tail projection so terminal failures survive response bounding, with an explicit head projection when setup output is needed. Failed jobs and their dependencies can be rerun through one explicit run id with dry-run and confirmation gates. PR creation and fork preparation also default to dry-run and require exact confirmation before mutating GitHub or remotes.

## Safety contract

1. Do not overwrite whole files by default.
2. Require anchors, hashes, or patch context for writes.
3. Write atomically through a temporary file plus rename.
4. Refuse ambiguous replacements.
5. Surface diffs before persistent changes when requested.
6. Never hide Git state from the caller.

See [docs/safety-contract.md](docs/safety-contract.md) for the full contract.

## Current status

Stage 1 MVP is implemented across the core crate, CLI, and MCP server for `replace-exact`, `read-range`, `write-new-file`, `diff-preview`, and `status-guard`. Stage 2 MCP validation support now adds capability discovery and build provenance, concurrent bounded dispatch for every ID-bearing tool call, asynchronous task-image/Harbor/profile jobs, bounded reply deadlines with hard Git child-process termination, durable mutation receipts, stable scratch paths, allowlisted guarded command execution, structured Harbor evidence, bounded batch file and recursive directory inspection, guarded executable-bit changes, binary/sidecar/bulk fixture writes, exact sidecar cleanup, host and task-image Python execution, hash-guarded existing-file synchronization, fixture manifest verification/refresh, typed fixture generator/base-image/image-cleanliness/Docker-inspect workflows, exact/stage/scoped/prefix Git workflows, guarded tracked-file moves and deletions, generated-prefix cleanup, GitHub PR/fork workflows, dry-run setup-profile planning, and typed native build/device workflows for Claude Desktop. Guarded repository file access currently requires Unix descriptor-relative path support and refuses rather than using race-prone path fallbacks elsewhere. The CLI safely maintains ordinary Claude Desktop ContextPatch entries but does not claim authority over Claude's runtime tool approvals. Code changes should keep the relevant Markdown file synchronized in the same commit.

## Repository layout

```text
crates/core/                   safe edit engine
crates/cli/                    human CLI
crates/server/                 context-server adapter
docs/                          public design and usage docs
tests/                         repo-level fixtures and integration tests
```

## Documentation contract

Public docs:

- [README](README.md)
- [Tool specification](docs/tool-spec.md)
- [Safety contract](docs/safety-contract.md)
- [Architecture](docs/architecture.md)
- [Claude Desktop usage](docs/claude-desktop.md)
- [Implementation roadmap](docs/implementation-roadmap.md)
- [Server refactor plan](docs/server-refactor-plan.md)
- [Native background implementation](docs/native-background-implementation.md)
- [Copilot repository instructions](.github/copilot-instructions.md)

| File | Must change when |
| --- | --- |
| [docs/tool-spec.md](docs/tool-spec.md) | A tool is added, removed, renamed, or its behavior changes |
| [docs/safety-contract.md](docs/safety-contract.md) | A write rule, guard, or refusal policy changes |
| [docs/architecture.md](docs/architecture.md) | Crate boundaries or ownership changes |
| [docs/claude-desktop.md](docs/claude-desktop.md) | Server install/config behavior changes |
| [docs/implementation-roadmap.md](docs/implementation-roadmap.md) | Stage scope, sequencing, or release criteria change |
