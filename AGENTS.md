# Repository operating rules

The non-obvious rules for working in this repository, kept here because rediscovering them from the
source costs more than reading them. Named neutrally rather than for one vendor's convention: the
rules are the project's, and which agent reads them is machine configuration rather than repository
content. Point your tooling at this file if it does not find it by name.

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

Most of the recent commit history is the migration of individual tools onto this model. A few sites still read `logical_path()`, all deliberately and none for access: two report `cwd` back to the caller, and one uses it as receipt identity, which is path-derived by design. The exact set is asserted by `the_files_that_read_a_logical_path_are_the_known_ones` in `dispatch.rs`, so read it there rather than from a list here — line numbers in prose go stale and that test does not. Adding a reader for *access* would be a regression; adding one for reporting needs the same justification these carry.

### Module size

There is no enforced line limit, and any figure quoted as one has been fitted to a measurement rather than chosen. What matters is whether a file holds more than one concern.

`registry.rs` is the largest file under `tools/` and is deliberately exempt: it is a declarative table of one entry per tool, growing linearly with the tool count. No figure is quoted, because it drifts by roughly a dozen lines per tool and a stale number here would argue against the very point this section makes. That is not the coupling the module split was addressing, and breaking it up would scatter the single source of truth it exists to be. `harbor.rs` and `capability.rs` are larger still and are genuine candidates, the latter because its manifest prose grows with every tool.

Split when a file holds unrelated concerns — which is what `process.rs` did, with job machinery, a log store, guarded execution, and container tools in one place — not when it crosses a number.

### The tool registry (`crates/server/src/tools/registry.rs`)

One `ToolDescriptor` per tool carries all six per-tool facts. `dispatch.rs` is a table lookup, `deadline_for` and `serializes_repository_mutation` are one-line field reads, and `schema/authority.rs` classifies from the descriptor. Before this existed those facts lived in five files with nothing checking they agreed, which produced four separate staleness defects in one week.

The predicate that would have caught all of them, and that has since caught two more: **a site asserting which tools have a property is a defect risk; a site naming one tool inside its own code path is not.** Every instance was a claim about a set — `typed_workflows`, `programs.bash`, the client-instruction log-id list, `guidance::permitted_summary`, `background_jobs.tools`, and the isolation sentence in `run_guarded_command`'s description. Each was true when written and went stale when the set changed. Apply the predicate to any new list: derive it from the registry or the allowlist that defines it, or assert it whole in a test that fails when it moves. A count or a membership list written by hand is the defect, not the omission from it.

`project_execute` is deliberately *not* in the registry. It is the surface wrapper, not an internal action: resolved in `handle_tool_call` before the repository is determined, advertised only on the project surface, and classified as the widest reach of everything it dispatches.

Two snapshots in `crates/server/tests/fixtures/` guard the whole surface: `tools-surface.json` (every advertised definition) and `tool-matrix.tsv` (every tool against the five behavioural axes, plus whether it starts a background job).

Regeneration is operator-only. `CONTEXTPATCH_UPDATE_FIXTURES=1 cargo test -p server` rewrites both, and `run_guarded_command` has no environment parameter, so it cannot be run through the guarded surface at all. That is deliberate rather than a gap. A deliberate move is patched from the comparison's own output: the failure prints the recorded and the current line verbatim, so the fixture is edited to what the test says it should be rather than reconstructed. Any change small enough to review by eye is small enough to patch that way, and a diff too large to patch by hand is a diff too large to review — which is a signal to look harder at the change, not a reason to grant an environment.

Do not route around this by spawning `cargo` from a Python artifact with a modified environment. That is the `rg --pre` escape wearing a different name, and C37 closed it by construction.

### Testing a dry-run-then-confirm capability

**A capability whose contract is dry run and then confirm needs at least one test that confirms.** The plan path and the execution path share no code, so a green plan proves nothing about execution.

This is the same defect the rest of this file catalogues, a fact asserted in one place and never checked against the thing it describes, but relocated from documentation into tests, where it was least visible and cost the most. Committing a rename had five passing parser tests and could not execute: `git add` aborted on the rename source, and once that was fixed the staged set reported one side against an expected set holding both. Both failures land after the plan returns ok. Neither was reachable from a test that only planned.

So for anything with a `dry_run` argument, the test that matters is the one that passes `dry_run: false` and its confirmation phrase, then asserts the world changed: the commit exists, the file moved, the worktree is clean. Assertions about what the plan *said* are worth having and are not evidence that the plan is achievable.

### Request pipeline (`crates/server/src/tools/dispatch.rs`)

`handle_tool_call` → resolve the tool surface → `effective_repository` → `execute_tool` → per-name reply deadline (`core::process::deadline`, 30s read / 60s write / 120s Git, max 16 active workers) → cooperative per-repository mutation lock for mutating tools → `call_tool` match arm → bounded 900 KiB response envelope.

A deadline bounds the *reply*, not the work: the worker is abandoned, so expiry means the outcome is **unknown**. That is why receipts (`core::fs::receipt`, surfaced by `read_write_receipts`) exist for exact replacement, exact-hash overwrite, exact untracked deletion, and local commits.

Long calls run concurrently, so MCP replies may arrive out of request order — clients correlate by JSON-RPC id, and tests must too (`support::run_server` sorts by id; `run_server_sequential` does not).

### Tool surfaces

`--tool-surface full` (default) advertises every action as a direct MCP tool. `--tool-surface project` advertises one `project_execute` wrapper with a `describe` meta-action, reducing per-tool client approvals to one identity per project; the wrapper may also select an exact descendant Git worktree via `repository` before the deadline/lock/handler/receipt are chosen. Both surfaces run identical guards.

## Adding or changing an MCP tool

Every per-tool fact lives on one `ToolDescriptor` in `crates/server/src/tools/registry.rs`: name, schema, handler, deadline class, mutation-lock membership, advertised reach, and read-only status. There is no second place to remember.

1. Define `pub const NAME` in a module inside `crates/server/src/tools/<domain>.rs` (the `pub mod name { pub const NAME: ... }` blocks). The registry reuses that constant, never a string literal.
2. Write the schema as `pub(crate) fn <name>_definition() -> Value` in `crates/server/src/tools/schema/<domain>.rs` and re-export it from `schema/mod.rs`. Annotations are added centrally from the descriptor's `reach` and `read_only`, so a tool cannot advertise an authority it is not classified under.
3. Write the handler in `tools/<domain>.rs`, then add one `ToolDescriptor` to `registry.rs`. Handlers keep whatever argument shape suits them; the table adapts them with a non-capturing closure.
4. Put behavior in `core`. Add the test in `crates/core` first.
5. Update `docs/tool-spec.md` — both the summary table row and a `### \`tool_name\`` contract section. Enforced by test in both directions.
6. Regenerate the recorded surface: `CONTEXTPATCH_UPDATE_FIXTURES=1 cargo test -p server`. The diff to `tests/fixtures/tools-surface.json` and `tool-matrix.tsv` is the review artifact — it shows exactly what the advertised contract gained. A regeneration you did not intend is a bug.
7. If the action is open-world or isolated, add it to `EXPECTED_OPEN_WORLD_ACTIONS` or `EXPECTED_ISOLATED_ACTIONS` in `tests/stage1_mcp/protocol.rs`. That list is deliberately hand-maintained: it audits the classification rather than restating it.
8. Update the other matching docs in the same commit (see Documentation contract).

No test carries a tool count, and the README does not list tools. Both used to, and both went stale.

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
