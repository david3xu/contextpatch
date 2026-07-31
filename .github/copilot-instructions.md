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
cargo test -p server stage2_setup_profile_run_plans_capacitor_shell_without_mutating
cargo test -p server stage2_native_build_run_plans_builds_without_raw_commands
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
| `core` | Path normalization, repo-root checks, atomic writes, exact replacement, patch/diff semantics, Git guards, no-shell process execution, setup-profile planning, typed native build/device policy, shared errors | MCP/JSON-RPC protocol, Claude Desktop config, CLI parsing, protocol schemas |
| `cli` | Human-facing command names, help text, exit codes, terminal output | Edit semantics or duplicated safety logic |
| `server` | MCP stdio protocol adapter, snake_case tool schemas, request/response adaptation | Edit, setup, process, Git, or native command policy already owned by `core` |

Dependency direction is one-way: `cli -> core`, `server -> core`; `core` must stay independent of `cli` and `server`.

The Stage 1 edit tools (`read_range`, `diff_preview`, `replace_exact`, `write_new_file`, `create_directory`, `status_guard`) exist across core, CLI, and MCP server. Stage 2A is primarily MCP-facing and adds bounded inspection, guarded validation/logging, binary and sidecar artifact workflows, fixture integrity/generation, declarative setup, typed native build/device actions, narrow local/remote Git operations, and typed GitHub PR/fork workflows.

The MCP server is line-oriented JSON-RPC over stdio. `crates/server/src/main.rs` is only the entrypoint; transport/protocol code lives in `server.rs` and `protocol.rs`, while schemas, dispatch, and handlers live under `crates/server/src/tools/`. Tool name constants are defined with their tool modules and must be reused by schemas and dispatch. CLI routing lives in `crates/cli/src/args.rs` and `crates/cli/src/commands/*`; product policy and behavior belong in `crates/core/src/*`.

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
- `apply_patch`, `insert_at_anchor`, `move_tracked`, and `delete_guarded` remain Stage 2 boundary tools, not advertised MCP capabilities. Some core modules are placeholders, and the CLI `apply-patch` command is still a not-implemented stub; do not infer support from names in the source or tool spec.
- `run_guarded_command` is validation support, not a shell: program plus args only, repo-root-confined cwd, timeout, redaction/truncation, null stdin, and allowlisted command families. It supports validation-oriented `git`, Rust, JS package-manager scripts, repo-relative Python scripts, `pytest`, `harbor run`, the exact `references/check-base-image.sh` check with optional `task` argument, and `rg`; it must not expose shell strings, arbitrary shell scripts, Docker, `pip`, arbitrary `python -m`, or package installation as generic execution.
- Setup and native workflows are typed adapters: derive command plans in `core::setup`, `core::native_build`, or `core::native_device`; server handlers only parse params, call core, and format MCP responses. Keep package installation out of `run_guarded_command` and raw `xcodebuild`, Gradle, `xcrun`, and `adb` authority out of public tools.
- `fixture_generator_run` runs repo-relative Python fixture generators with dry-run/confirmation, allowed pre-existing dirty paths, and declared output path/prefix verification. Use it for temporary in-repo generators, then delete those helpers before commit if they are not submission artifacts. Use `fixture_manifest_verify` and `fixture_manifest_refresh` for exact fixture file-set and SHA-256 integrity.
- Local/remote Git mutation is intentionally narrow: staging, exact/scoped/prefix commits, tracked restore, and exact/prefix-generated cleanup are separate tools with dry-run, clean-index/worktree, path-expansion, and confirmation gates. Remote publishing stays split across remote listing/checking, branch preparation, merge readiness, and exact push; never collapse these into broad Git authority.
- GitHub workflows are intentionally narrow: use `github_pr_run` for auth, targeted fork/upstream PR reads, locally filtered sticky comments, full-SHA-scoped run discovery, structured checks, redacted/bounded Actions run/job evidence, confirmation-gated failed-job reruns, and confirmation-gated PR creation; use `github_fork_prepare` for dry-run/confirmed fork preparation, not arbitrary `gh` passthrough.
- Every advertised MCP tool must include stable annotations via the central schema helper. Annotations are client permission hints only and never replace internal dry-run, confirmation, path, hash, repository-state, or timeout guards.

## Testing and docs

Core behavior should be tested in `crates/core` first. CLI and server tests should focus on argument/protocol mapping and integrated behavior. Existing integration tests create temporary Git repositories under `std::env::temp_dir()` and invoke compiled binaries through `env!("CARGO_BIN_EXE_contextpatch")` or `env!("CARGO_BIN_EXE_contextpatch-server")`.

When changing refusal behavior, assert the refusal text and that target files remain unchanged. When changing MCP behavior, assert the `isError` shape for expected refusals.

Keep docs synchronized with behavior changes: update `docs/tool-spec.md` for tool changes, `docs/safety-contract.md` for write/guard/refusal policy changes, `docs/architecture.md` for crate-boundary changes, `docs/claude-desktop.md` for server install/config changes, and `docs/implementation-roadmap.md` for stage scope or release criteria changes.
