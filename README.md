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

The first useful milestone is a CLI command that can safely replace exactly one matched text span and refuse zero-match or multi-match edits. The full staged plan is in `docs/implementation-roadmap.md`.

```bash
contextpatch read-range <path> --start <line> --end <line>
contextpatch diff-preview <path> --old <text> --new <text>
contextpatch replace-exact <path> --old <text> --new <text>
contextpatch write-new-file <path> --content <text>
contextpatch status-guard [path]
```

The MCP server exposes the same Stage 1 surface to local agent clients:

- `capability_manifest`
- `preflight_health`
- `read_range`
- `diff_preview`
- `replace_exact`
- `write_new_file`
- `status_guard`
- `run_guarded_command`
- `read_command_log`
- `validation_profile_run`
- `setup_profile_run`
- `native_build_run`
- `native_device_run`
- `git_commit_exact`
- `git_remote_check`
- `git_branch_prepare`
- `git_merge_readiness`
- `git_push_exact`

`run_guarded_command` is Stage 2 MCP-only validation support: it runs no shell, stays repo-root-confined, allows only selected validation-oriented programs/subcommands, drains stdout/stderr concurrently, applies a timeout, redacts probable secret values without hiding ordinary paths or docs, and reports command/cwd/exit-code/duration metadata.

`validation_profile_run` compresses common validation sequences into one guarded call and returns compact command summaries with log ids. `read_command_log` retrieves those redacted logs on demand so large outputs do not have to ride in the first JSON-RPC response.

`setup_profile_run` is a declarative setup-profile tool for real project scaffolding needs. It defaults to dry-run, can run typed server-owned actions such as the `node-capacitor-shell` profile only with `confirm: "run setup profile"` and a clean worktree, and does not expose arbitrary `npm install`, `pnpm add`, `npx`, package lists, or shell commands through `run_guarded_command`. The Capacitor profile derives npm or pnpm command plans from project lockfiles and installs `@capacitor/cli` as a dev dependency.

`native_build_run` and `native_device_run` cover native plugin/build workflows without exposing raw `xcodebuild`, `./gradlew`, `xcrun`, or `adb` authority. Agents select typed actions such as `ios_build`, `android_assemble_debug`, `ios_launch_app`, or `android_read_logcat`; contextpatch derives the exact command plan, defaults to dry-run, validates repo-relative artifacts, and requires `confirm: "run native device"` before executing device-state-changing actions.

`git_commit_exact` is a narrowly gated local Git checkpoint tool. It defaults to dry-run, requires the provided path list to exactly match the full dirty-path set, requires `confirm: "commit exact paths"` before mutation, stages only those paths, creates at most one local commit, and never fetches or pushes.

`git_remote_check` and `git_push_exact` keep remote publishing separate from local commit authority. `git_remote_check` fetches one explicit remote branch and reports whether `HEAD..remote/branch` is empty without changing source files. `git_push_exact` requires a clean worktree, matching current branch, expected HEAD hash, no remote-ahead divergence after fetch, `confirm: "push exact commit"`, and pushes only `HEAD:refs/heads/<branch>` without force.

`git_branch_prepare` handles the narrow branch-prep case for agent workflows: from a clean worktree it fetches one explicit remote base branch, creates or switches to the requested local branch, optionally resets an existing local branch only with `confirm: "reset branch from remote base"`, verifies the remote base is an ancestor, and checks required files. It does not expose broad checkout, rebase, merge, stash, clean, or arbitrary fetch.

`git_merge_readiness` fills the PR/merge planning gap without exposing `git merge` or `merge-tree`: it optionally fetches one explicit remote branch, resolves two validated refs, reports ahead counts, and returns files changed on both sides since the merge base as likely conflict candidates. It does not checkout, merge, reset, stash, or edit source files.

## Safety contract

1. Do not overwrite whole files by default.
2. Require anchors, hashes, or patch context for writes.
3. Write atomically through a temporary file plus rename.
4. Refuse ambiguous replacements.
5. Surface diffs before persistent changes when requested.
6. Never hide Git state from the caller.

See `docs/safety-contract.md` for the full contract.

## Current status

Stage 1 MVP is implemented across the core crate, CLI, and MCP server for `replace-exact`, `read-range`, `write-new-file`, `diff-preview`, and `status-guard`. Stage 2 MCP validation support now adds capability discovery, preflight health, allowlisted guarded command execution, exact Git workflows, dry-run setup-profile planning, and typed native build/device workflows for Claude Desktop. Code changes should keep the relevant Markdown file synchronized in the same commit.

## Repository layout

```text
crates/core/                   safe edit engine
crates/cli/                    human CLI
crates/server/                 context-server adapter
docs/                          public design and usage docs
tests/                         repo-level fixtures and integration tests
```

## Documentation contract

| File | Must change when |
| --- | --- |
| `docs/tool-spec.md` | A tool is added, removed, renamed, or its behavior changes |
| `docs/safety-contract.md` | A write rule, guard, or refusal policy changes |
| `docs/architecture.md` | Crate boundaries or ownership changes |
| `docs/claude-desktop.md` | Server install/config behavior changes |
| `docs/implementation-roadmap.md` | Stage scope, sequencing, or release criteria change |
