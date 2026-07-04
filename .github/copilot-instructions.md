# Repository instructions for Copilot

## Build, test, and lint

This is a Rust workspace with three packages: `core`, `cli`, and `server`.

```bash
cargo check --workspace
cargo build --workspace
cargo build --release -p server --bin contextpatch-server
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Run targeted tests by package and test name:

```bash
cargo test -p core replaces_exactly_one_match
cargo test -p cli stage1_cli_tools_work_together
cargo test -p server stage1_mcp_tools_work_together
cargo test -p server stage2_git_commit_exact_dry_run_and_commit_are_gated
```

Useful local commands:

```bash
cargo run -p cli --bin contextpatch -- read-range README.md --start 1 --end 20
cargo run -p server --bin contextpatch-server -- --repo-root /path/to/repo
```

## Architecture

`contextpatch` is a safe patch layer for AI coding agents. The project boundary is narrow: anchored, atomic, reviewable repository edits plus validation guardrails. It should not become a general filesystem server, shell runner, or broad Git automation tool.

Layering is intentional:

| Package | Owns | Must not own |
| --- | --- | --- |
| `core` | Path normalization, repo-root checks, atomic writes, exact replacement, patch/diff semantics, Git status/guard logic, guarded command execution, shared errors | MCP/context-server protocol, JSON-RPC, Claude Desktop config, CLI parsing |
| `cli` | Human-facing command names, help text, exit codes, terminal output | Edit semantics or duplicated safety logic |
| `server` | MCP stdio protocol adapter, snake_case tool schemas, request/response adaptation, Claude/client-facing server behavior | Filesystem mutation semantics already owned by `core` |

Dependency direction is one-way: `cli -> core`, `server -> core`; `core` must stay independent of `cli` and `server`.

The MCP server is line-oriented JSON-RPC over stdio in `crates/server/src/main.rs`. Tool name constants live in `crates/server/src/tools/*`; schema definitions and dispatch should use those constants instead of repeating literal names.

## Safety and behavior conventions

- Persistent writes must be anchored or otherwise guarded: exact old text, patch context, destination absence, hash/confirmation, or equivalent policy.
- Write operations should use the core atomic-write path and must refuse ambiguous or unsafe inputs rather than silently falling back.
- Path handling should resolve under the configured repository root. Existing-file operations use existing-file resolution; create-only operations verify the parent exists inside the root and refuse existing destinations.
- Refusals are expected safety outcomes. CLI commands print `<command> refused: ...` to stderr and return exit code `1`; parse/usage errors return `2`.
- Server tool refusals are returned as successful JSON-RPC tool responses with `"isError": true`, not as transport failures.
- Public protocol tool names are snake_case (`replace_exact`); CLI commands are kebab-case (`replace-exact`).
- `read_range` uses 1-based inclusive line numbers and returns numbered lines.
- `replace_exact` refuses empty old text, zero matches, and multi-matches; do not add broad whole-file rewrite behavior.
- `write_new_file` is create-only and must not overwrite.
- `diff_preview` previews exact replacement behavior without mutating the file.
- `apply_patch`, `insert_at_anchor`, `move_tracked`, and `delete_guarded` are documented roadmap/boundary tools unless implementation exists; keep docs and capability reporting honest when changing status.

## Guarded command and Git conventions

`run_guarded_command` is validation support, not a shell. It accepts a program plus argument array, runs with null stdin, adds `--no-pager`/`GIT_PAGER=cat` for Git, enforces repo-root-confined cwd, timeouts, redaction, truncation, and an allowlist.

Currently allowlisted command families are:

- `git`: `status`, `diff`, `log`, `show`, `rev-parse`, `ls-tree`
- `cargo`: `check`, `test`, `build`, `clippy`
- `bun`: `run`, `test`
- `npm`: `run`, `test`
- `rg`: search invocations

Local Git mutation is intentionally narrow. `git_commit_exact` defaults to dry-run, requires the provided path list to exactly match the full dirty-path set, requires `confirm: "commit exact paths"` for mutation, stages only those paths, creates one local commit, and never pushes. Remote publishing is split into `git_remote_check` and `git_push_exact`; push requires a clean worktree, matching current branch, expected HEAD, no remote-ahead divergence after fetch, `confirm: "push exact commit"`, and no force.

## Testing conventions

Core behavior should be tested in `crates/core` first. CLI and server tests should focus on argument/protocol mapping and integrated behavior rather than duplicating every core edit-engine case.

Existing integration tests create temporary Git repositories under `std::env::temp_dir()`, configure test-only Git identity, and invoke compiled binaries through `env!("CARGO_BIN_EXE_contextpatch")` or `env!("CARGO_BIN_EXE_contextpatch-server")`.

When changing refusal behavior, assert both the refusal text and that target files remain unchanged. When changing MCP behavior, assert the `isError` shape for expected refusals.

## Documentation contract

Keep docs synchronized with behavior changes:

| File | Update when |
| --- | --- |
| `docs/tool-spec.md` | A tool is added, removed, renamed, or behavior changes |
| `docs/safety-contract.md` | A write rule, guard, or refusal policy changes |
| `docs/architecture.md` | Crate boundaries or ownership change |
| `docs/claude-desktop.md` | Server install/config behavior changes |
| `docs/implementation-roadmap.md` | Stage scope, sequencing, or release criteria change |

