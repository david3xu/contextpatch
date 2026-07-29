# Repository instructions for Copilot

## Build, test, and lint

This is a Rust workspace with `core`, `cli`, and `server` packages.

```bash
cargo check --workspace
cargo build --workspace
cargo build --release -p server --bin contextpatch-server
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Run a single targeted test by package and test name:

```bash
cargo test -p core replaces_exactly_one_match
cargo test -p cli stage1_cli_tools_work_together
cargo test -p server stage1_mcp_tools_work_together
cargo test -p server stage2_git_commit_exact_dry_run_and_commit_are_gated
cargo test -p server stage2_git_branch_prepare
cargo test -p server stage2_git_merge_readiness
```

Useful local smoke commands:

```bash
cargo run -p cli --bin contextpatch -- read-range README.md --start 1 --end 20
cargo run -p server --bin contextpatch-server -- --repo-root /path/to/repo
```

## Architecture

`contextpatch` is a safe patch layer for AI coding agents. Keep the product boundary narrow: anchored, atomic, reviewable repository edits plus validation guardrails, not a general filesystem server, shell runner, or broad Git automation tool.

| Package | Owns | Must not own |
| --- | --- | --- |
| `core` | Path normalization, repo-root checks, atomic writes, exact replacement, patch/diff semantics, Git status/guard logic, guarded command execution, shared errors | MCP/JSON-RPC protocol, Claude Desktop config, CLI parsing |
| `cli` | Human-facing command names, help text, exit codes, terminal output | Edit semantics or duplicated safety logic |
| `server` | MCP stdio protocol adapter, snake_case tool schemas, request/response adaptation | Filesystem mutation semantics already owned by `core` |

Dependency direction is one-way: `cli -> core`, `server -> core`; `core` must stay independent of `cli` and `server`.

The Stage 1 edit tools (`read_range`, `diff_preview`, `replace_exact`, `write_new_file`, `status_guard`) exist across core, CLI, and MCP server. Stage 2A validation/Git workflow tools are MCP-facing: capability/preflight, guarded commands/logs, validation profiles, base64 file creation, hash-guarded existing-file writes, bounded bulk fixture import, fixture manifest verify/refresh, sidecar artifact creation, typed fixture-generator and base-image-check workflows, scoped/exact local commits, exact tracked restore, exact untracked cleanup, remote listing/checking, branch preparation, merge readiness, exact push, PR workflows, and fork preparation.

The MCP server is line-oriented JSON-RPC over stdio in `crates/server/src/main.rs`. Tool name constants live in `crates/server/src/tools/*`; schemas and dispatch should use those constants. CLI routing lives in `crates/cli/src/args.rs` and `crates/cli/src/commands/*`; edit behavior belongs in `crates/core/src/*`.

## Key conventions

- Persistent writes must be anchored or otherwise guarded: exact old text, patch context, destination absence, hash/confirmation, or equivalent policy.
- Write operations should use the core atomic-write path and refuse ambiguous or unsafe inputs rather than falling back to broad rewrites.
- Existing-file operations resolve under the repository root; create-only operations verify the parent exists inside the root and refuse existing destinations.
- CLI refusals print `<command> refused: ...` to stderr and return exit code `1`; parse/usage errors return `2`.
- MCP tool refusals are successful JSON-RPC tool responses with `"isError": true`, not transport failures.
- Public protocol tool names are snake_case (`replace_exact`); CLI commands are kebab-case (`replace-exact`).
- `read_range` uses 1-based inclusive line numbers and returns numbered lines.
- `replace_exact` refuses empty old text, zero matches, and multi-matches.
- `write_new_file` and `write_new_file_base64` are repo-root-confined create-only tools; `write_existing_file_exact_hash` is for whole-file synchronization only when the current SHA-256 is supplied; `bulk_write_new_files_base64` is a bounded multi-file create-only fixture import; `artifact_write_text` and `artifact_write_base64` are create-only sidecar artifact tools outside the repo tree; `diff_preview` previews exact replacement without mutating.
- `apply_patch`, `insert_at_anchor`, `move_tracked`, and `delete_guarded` are roadmap/boundary tools unless implementation exists; the CLI `apply-patch` command is currently only a not-implemented stub.
- `run_guarded_command` is validation support, not a shell: program plus args only, repo-root-confined cwd, timeout, redaction/truncation, null stdin, and allowlisted command families. It supports validation-oriented `git`, Rust, JS package-manager scripts, repo-relative Python scripts, `pytest`, `harbor run`, the exact `references/check-base-image.sh` check with optional `task` argument, and `rg`; it must not expose shell strings, arbitrary shell scripts, Docker, `pip`, arbitrary `python -m`, or package installation as generic execution.
- `fixture_generator_run` runs repo-relative Python fixture generators with dry-run/confirmation, allowed pre-existing dirty paths, and declared output path/prefix verification. Use it for temporary in-repo generators, then delete those helpers before commit if they are not submission artifacts. Use `fixture_manifest_verify` and `fixture_manifest_refresh` for exact fixture file-set and SHA-256 integrity.
- Local/remote Git mutation is intentionally narrow: `git_commit_exact` and `git_commit_scoped` are local-only commit tools; cleanup is split into `git_restore_exact` for explicit tracked paths and `delete_untracked_exact` for explicit untracked regular files; remote publishing is split across `git_remote_list`, `git_remote_check`, `git_branch_prepare`, `git_merge_readiness`, and `git_push_exact`.
- GitHub workflows are intentionally narrow: use `github_pr_run` for auth/view/check/create PR actions and `github_fork_prepare` for dry-run/confirmed fork preparation, not arbitrary `gh` passthrough.

## Testing and docs

Core behavior should be tested in `crates/core` first. CLI and server tests should focus on argument/protocol mapping and integrated behavior. Existing integration tests create temporary Git repositories under `std::env::temp_dir()` and invoke compiled binaries through `env!("CARGO_BIN_EXE_contextpatch")` or `env!("CARGO_BIN_EXE_contextpatch-server")`.

When changing refusal behavior, assert the refusal text and that target files remain unchanged. When changing MCP behavior, assert the `isError` shape for expected refusals.

Keep docs synchronized with behavior changes: update `docs/tool-spec.md` for tool changes, `docs/safety-contract.md` for write/guard/refusal policy changes, `docs/architecture.md` for crate-boundary changes, `docs/claude-desktop.md` for server install/config changes, and `docs/implementation-roadmap.md` for stage scope or release criteria changes.
