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
```

The MCP server exposes the safe tool surface to local agent clients:

- `capability_manifest`
- `preflight_health`
- `read_range`
- `diff_preview`
- `replace_exact`
- `write_new_file`
- `write_new_file_base64`
- `write_existing_file_exact_hash`
- `file_info`
- `list_directory`
- `read_file_bytes`
- `artifact_write_text`
- `artifact_write_base64`
- `bulk_write_new_files_base64`
- `create_directory`
- `status_guard`
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
- `git_commit_scoped`
- `git_commit_prefix`
- `git_stage_exact`
- `git_staged_scope_check`
- `git_restore_exact`
- `delete_untracked_exact`
- `delete_generated_prefix`
- `git_remote_list`
- `git_remote_check`
- `git_branch_prepare`
- `git_merge_readiness`
- `git_push_exact`
- `github_pr_run`
- `github_fork_prepare`

Every advertised MCP tool includes stable tool annotations so clients can offer persistent "always allow" approval after the first permission decision. These annotations do not bypass server-side guardrails: mutation-capable tools still keep their dry-run defaults, exact confirmation strings, path checks, clean-index/worktree gates, and refusal behavior.

`run_guarded_command` is Stage 2 MCP-only validation support: it runs no shell, stays repo-root-confined, allows only selected validation-oriented programs/subcommands, drains stdout/stderr concurrently, applies a timeout, redacts probable secret values without hiding ordinary paths or docs, and reports command/cwd/exit-code/duration metadata. It supports repo-relative Python scripts, `pytest`, `harbor run`, and the exact `references/check-base-image.sh` or `references/check-base-image.sh task` base-image checks for project validation; it still refuses Docker, `pip`, arbitrary `python -m`, arbitrary shell scripts, and generic package installation.

`file_info`, `list_directory`, and `read_file_bytes` provide bounded inspection for file digests, line counts, directory entries, symlink status, and binary byte ranges without relying on ad hoc scripts. `write_new_file_base64` creates binary repository files with the same create-only root guard as text file creation. `write_existing_file_exact_hash` covers whole-file synchronization only when the caller supplies the current lowercase SHA-256 digest and explicit confirmation. `bulk_write_new_files_base64` imports many create-only fixture files in one bounded call. `fixture_manifest_verify` and `fixture_manifest_refresh` enforce exact fixture file sets and SHA-256 digests against a manifest. `artifact_write_text` and `artifact_write_base64` create sidecar artifacts outside the repository tree for generators, trap checks, or local helper files that must not be committed. `artifact_python_run` executes a Python script from that sidecar artifact root with no shell and repo-root-confined cwd, so scratch analysis does not have to create temporary files in the repository. `fixture_generator_run` handles the common Dynamo pattern of temporarily staging a repo-relative Python generator, running it with declared output paths/prefixes, verifying it did not touch undeclared files, and then letting the caller delete the temporary generator before commit.

`validation_profile_run` compresses common validation sequences into one guarded call and returns compact command summaries with log ids. For `dynamo-harbor-task`, it also appends a structured Harbor summary with oracle rewards, nop rewards, determinism checks, and a single pass/fail field. `read_command_log` retrieves redacted logs on demand, with offset paging, so large outputs do not have to ride in the first JSON-RPC response. Hosts can add validation executables that are not on the server `PATH` through `CONTEXTPATCH_VALIDATION_PATHS`; `preflight_health` reports resolved validation-tool paths. `image_cleanliness_check_run` covers the narrow Docker image-cleanliness check by planning or running only `docker run --rm --network none --entrypoint find <image> / -name <filename>`, defaulting to `solve.sh`. `docker_image_inspect` adds only the read-only `docker image inspect <image>` workflow and keeps generic Docker arguments out of scope.

`setup_profile_run` is a declarative setup-profile tool for real project scaffolding needs. It defaults to dry-run, can run typed server-owned actions such as the `node-capacitor-shell` profile only with `confirm: "run setup profile"` and a clean worktree, and does not expose arbitrary `npm install`, `pnpm add`, `npx`, package lists, or shell commands through `run_guarded_command`. The Capacitor profile derives npm or pnpm command plans from project lockfiles and installs `@capacitor/cli` as a dev dependency.

`native_build_run` and `native_device_run` cover native plugin/build workflows without exposing raw `xcodebuild`, `./gradlew`, `xcrun`, or `adb` authority. Agents select typed actions such as `ios_build`, `android_assemble_debug`, `ios_launch_app`, or `android_read_logcat`; contextpatch derives the exact command plan, defaults to dry-run, validates repo-relative artifacts, and requires `confirm: "run native device"` before executing device-state-changing actions.

`git_stage_exact` stages explicit dirty paths without committing, preserving the split between "track this file" and "create a validated checkpoint." `git_staged_scope_check` is the read-only audit partner: it reports whether currently staged paths are limited to allowed exact paths/prefixes and whether required paths are staged. `git_commit_exact` is a narrowly gated local Git checkpoint tool. It defaults to dry-run, requires the provided path list to exactly match the full dirty-path set, requires `confirm: "commit exact paths"` before mutation, stages only those paths, creates at most one local commit, and never fetches or pushes. `git_commit_scoped` covers the related case where unrelated dirty files must be preserved; it requires a clean index and stages only the explicit dirty subset. `git_commit_prefix` is the practical large-fixture variant: it expands explicit prefixes to the exact dirty paths underneath them, reports the expanded set in dry-run, then commits only that expanded set with `confirm: "commit prefix paths"`.

`git_restore_exact`, `delete_untracked_exact`, and `delete_generated_prefix` cover generated-noise cleanup without exposing broad reset/checkout/clean behavior. Exact tools operate only on explicit requested paths. Prefix cleanup expands explicit prefixes only to ignored/untracked files and empty directories, refuses tracked paths, and defaults to dry-run/confirmation-gated mutation.

`git_remote_list`, `git_remote_check`, and `git_push_exact` keep remote publishing separate from local commit authority. `git_remote_list` parses `git remote -v` without mutation. `git_remote_check` fetches one explicit remote branch and reports whether `HEAD..remote/branch` is empty without changing source files. `git_push_exact` requires a clean worktree, matching current branch, expected HEAD hash, no remote-ahead divergence after fetch, `confirm: "push exact commit"`, and pushes only `HEAD:refs/heads/<branch>` without force.

`git_branch_prepare` handles the narrow branch-prep case for agent workflows: from a clean worktree it fetches one explicit remote base branch, creates or switches to the requested local branch, optionally resets an existing local branch only with `confirm: "reset branch from remote base"`, verifies the remote base is an ancestor, and checks required files. It does not expose broad checkout, rebase, merge, stash, clean, or arbitrary fetch.

`git_merge_readiness` fills the PR/merge planning gap without exposing `git merge` or `merge-tree`: it optionally fetches one explicit remote branch, resolves two validated refs, reports ahead counts, and returns files changed on both sides since the merge base as likely conflict candidates. It does not checkout, merge, reset, stash, or edit source files.

`github_pr_run` and `github_fork_prepare` cover GitHub review workflow gaps without arbitrary `gh` access. PR creation and fork preparation default to dry-run and require exact confirmation before mutating GitHub or remotes.

## Safety contract

1. Do not overwrite whole files by default.
2. Require anchors, hashes, or patch context for writes.
3. Write atomically through a temporary file plus rename.
4. Refuse ambiguous replacements.
5. Surface diffs before persistent changes when requested.
6. Never hide Git state from the caller.

See [docs/safety-contract.md](docs/safety-contract.md) for the full contract.

## Current status

Stage 1 MVP is implemented across the core crate, CLI, and MCP server for `replace-exact`, `read-range`, `write-new-file`, `diff-preview`, and `status-guard`. Stage 2 MCP validation support now adds capability discovery, preflight health, allowlisted guarded command execution with host-configured validation paths, bounded file/directory/binary inspection, binary/sidecar/bulk fixture writes, artifact-root Python scratch execution, hash-guarded existing-file synchronization, fixture manifest verification/refresh, typed fixture generator/base-image/image-cleanliness/Docker-inspect workflows, exact/stage/scoped/prefix Git workflows, generated-prefix cleanup, GitHub PR/fork workflows, dry-run setup-profile planning, and typed native build/device workflows for Claude Desktop. Code changes should keep the relevant Markdown file synchronized in the same commit.

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
