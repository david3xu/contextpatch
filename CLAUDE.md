# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

`cargo` is installed at `~/.cargo/bin/cargo` but is **not on the PATH of non-interactive shells** here. Use the absolute path (or `export PATH="$HOME/.cargo/bin:$PATH"` first) or every command below fails with `command not found`.

```bash
~/.cargo/bin/cargo check --workspace
~/.cargo/bin/cargo build --workspace
~/.cargo/bin/cargo build --release -p server --bin contextpatch-server
~/.cargo/bin/cargo test --workspace
~/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings
~/.cargo/bin/cargo fmt --all -- --check
```

Single tests (integration tests are one binary per file; filter by test-fn name substring):

```bash
cargo test -p core replaces_exactly_one_match
cargo test -p cli stage1_cli_tools_work_together
cargo test -p server stage2_git_commit_exact_dry_run_and_commit_are_gated
cargo test -p server stage2_          # whole Stage 2 MCP suite
```

Smoke-run the binaries:

```bash
cargo run -p cli --bin contextpatch -- read-range README.md --start 1 --end 20
cargo run -p server --bin contextpatch-server -- --repo-root /path/to/repo [--tool-surface project|full]
```

There is no CI workflow in `.github/` — clippy/fmt/test are local gates only.

## Architecture

Three crates, one-way dependencies (`cli -> core`, `server -> core`; `core` never depends on either):

| Crate | Bin/lib | Owns | Must not own |
| --- | --- | --- | --- |
| `crates/core` (`contextpatch_core`) | lib | All product policy: path/root authority, atomic writes, exact replacement, diff/patch semantics, Git guards, no-shell process execution, deadlines, locks, receipts, setup/native command planning, shared errors | Protocol, JSON schemas, CLI parsing, Desktop config |
| `crates/cli` | `contextpatch` | kebab-case commands, help text, exit codes, terminal output, Claude Desktop config maintenance | Any edit or safety semantics |
| `crates/server` | `contextpatch-server` | line-oriented JSON-RPC-over-stdio MCP adapter, snake_case tool schemas, dispatch, request/response shaping | Any policy already owned by `core` |

Server handlers are adapters: parse JSON → typed core params → call core → format the MCP response. If a handler is deriving a command, resolving a path, or deciding a guard, that logic belongs in `core`.

### Repository authority (the central invariant)

Operations do **not** take a repository path. They take `core::git::root::RepositoryRoot`, which pairs a logical path with the authority that identifies the directory — a retained directory descriptor for a validated workspace selection, or the name itself for the configured `--repo-root`. The descriptor is private on purpose: a caller cannot bypass the authority and use the name when the descriptor is inconvenient.

- `core::fs::rooted` is the filesystem projection: descriptor-relative, `O_NOFOLLOW` primitives for metadata, read, hash, list, rename, unlink, recursive remove. Non-Unix **fails closed** rather than degrading to path resolution. Never reintroduce a name-resolving `std::fs` call on a repository file.
- `core::git::repository::GitRepository` is the Git projection and supplies the cwd for guarded subprocesses.
- `server::tools::dispatch::EffectiveRepository::root()` is the single place a call's authority is decided. Handlers receive it; they never resolve names themselves.
- Boundaries that need a stable *name* (mutation locks, the receipt journal, scratch identity) use `canonical_label`, never a path used to reach a file.

Most of the recent commit history is the migration of individual tools onto this model; a few handlers still take the logical path. Move them onto typed authority rather than adding new path-taking handlers.

### Request pipeline (`crates/server/src/tools/dispatch.rs`)

`handle_tool_call` → resolve the tool surface → `effective_repository` → `execute_tool` → per-name reply deadline (`core::process::deadline`, 30s read / 60s write / 120s Git, max 16 active workers) → cooperative per-repository mutation lock for mutating tools → `call_tool` match arm → bounded 900 KiB response envelope.

A deadline bounds the *reply*, not the work: the worker is abandoned, so expiry means the outcome is **unknown**. That is why receipts (`core::fs::receipt`, surfaced by `read_write_receipts`) exist for exact replacement, exact-hash overwrite, exact untracked deletion, and local commits.

Long calls run concurrently, so MCP replies may arrive out of request order — clients correlate by JSON-RPC id, and tests must too (`support::run_server` sorts by id; `run_server_sequential` does not).

### Tool surfaces

`--tool-surface full` (default) advertises every action as a direct MCP tool. `--tool-surface project` advertises one `project_execute` wrapper with a `describe` meta-action, reducing per-tool client approvals to one identity per project; the wrapper may also select an exact descendant Git worktree via `repository` before the deadline/lock/handler/receipt are chosen. Both surfaces run identical guards.

## Adding or changing an MCP tool

1. Define `pub const NAME` in a module inside `crates/server/src/tools/<domain>.rs` (see the `pub mod name { pub const NAME: ... }` blocks in `files.rs`, `git/`, etc.). Schemas and dispatch must reuse that constant, never a string literal.
2. Add the JSON schema in the matching `crates/server/src/tools/schema/<domain>.rs` and register it in `schema/mod.rs::internal_tool_definitions`. Annotations come from the central helper; `openWorldHint` is derived in `schema/authority.rs`.
3. Add the dispatch arm in `tools/dispatch.rs::call_tool`, plus the deadline class and mutation-lock membership.
4. Put behavior in `core`. Add the test in `crates/core` first.
5. `crates/server/tests/stage1_mcp/protocol.rs` asserts `openWorldHint` against the documented execution authority — a new open-world or isolated action must be listed there, and classified in `schema/authority.rs`.
6. Update `docs/tool-spec.md` — both the summary table row and a `### \`tool_name\`` contract section. This is enforced by test, not convention.
7. Three tool-count assertions will fail until updated: `tests/stage1_mcp/files.rs` (`tools/list` length) and two in `tests/stage1_mcp/project.rs` (`action_count`, which is one higher because `describe` is dispatchable, and `action_definitions`).
8. Update the other matching docs in the same commit (see Documentation contract).

## Conventions

- Public MCP tool names are snake_case (`replace_exact`); CLI commands are kebab-case (`replace-exact`).
- CLI refusals print `<command> refused: ...` to stderr with exit code `1`; parse/usage errors exit `2`.
- MCP refusals are **successful** JSON-RPC responses with `"isError": true` — never transport errors.
- Every persistent write is anchored: exact old text, destination absence, current SHA-256 plus explicit `confirm: "<exact phrase>"`, or equivalent. Mutating tools default to `dry_run: true`.
- `ContextPatchError::invalid` marks caller-correctable refusals; `new` is for environment failures. They are behaviorally identical — the choice documents intent.
- `run_guarded_command` is validation support, not a shell: program + argv only, repo-root-confined cwd, allowlisted command families, timeout, redaction, null stdin. Keep Docker, `pip`, arbitrary `python -m`, package installation, and raw `xcodebuild`/Gradle/`xcrun`/`adb` out of it — those belong in typed `setup`/`native_build`/`native_device` plans.
- No-shell means this process never hands a string to an interpreter. It is **not** a sandbox: children inherit the environment and network. Only `task_image_python_run` and `image_cleanliness_check_run` are container-isolated with networking disabled. Do not let docs or schemas claim otherwise; `docs/execution-threat-model.md` is the audited statement.
- `apply_patch` and `insert_at_anchor` are Stage 2 boundary names only — the CLI `apply-patch` command is a not-implemented stub, and `policy::guard::require_clean_guard` is likewise unimplemented. Do not infer support from the names.

## Tests

Integration tests build and drive the real binaries: `env!("CARGO_BIN_EXE_contextpatch")` and `env!("CARGO_BIN_EXE_contextpatch-server")`, against temporary Git repositories created under `std::env::temp_dir()`. Server helpers live in `crates/server/tests/support/mod.rs` (`git_repo`, `run_server`, `run_server_project`, `run_server_sequential`, `run_server_with_env`).

Test names carry the stage prefix (`stage1_*`, `stage2_*`) and read as behavior claims. Core behavior is tested in `crates/core` inline `#[cfg(test)]` modules first; CLI/server tests cover argument and protocol mapping plus integrated workflows. When changing a refusal, assert both the refusal text and that the target file is unchanged; when changing MCP behavior, assert the `isError` shape.

## Documentation contract

Code changes must keep the relevant Markdown synchronized **in the same commit**:

| File | Must change when |
| --- | --- |
| `docs/tool-spec.md` | A tool is added, removed, renamed, or its behavior changes |
| `docs/safety-contract.md` | A write rule, guard, or refusal policy changes |
| `docs/architecture.md` | Crate boundaries or ownership changes |
| `docs/claude-desktop.md` | Server install/config behavior changes |
| `docs/implementation-roadmap.md` | Stage scope, sequencing, or release criteria change |
| `docs/execution-threat-model.md` | Execution authority or `openWorldHint` classification changes |
| `README.md` | The advertised action list or public claims change |

`docs/tool-spec.md` **is** enforced automatically: tests in `crates/server/src/tools/schema/mod.rs` assert that its summary table and its `### \`tool_name\`` contract headings match the registered tool set exactly, in both directions. Adding a tool without documenting it fails the build, as does documenting one that does not exist. The other files in this table are convention only.

## Product boundary

`contextpatch` is the safe patch layer between an AI agent and a repository: anchored, atomic, reviewable edits plus validation guardrails. It is not a general filesystem server, not a shell runner, not a Git replacement. Reject work outside that boundary unless it directly supports anchored edits, reviewable diffs, validation, or repository guardrails — the narrowness is the product, not a temporary limitation.
