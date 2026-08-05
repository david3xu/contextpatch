# Server Refactor Plan

`crates/server/src/main.rs` has grown beyond its intended role. The server crate should remain a protocol adapter, but `main.rs` currently mixes startup, JSON-RPC routing, MCP tool schemas, tool dispatch, handlers, validation profiles, Git helpers, fixture helpers, command logs, preflight probes, and shared argument parsing.

This is directory-structure and refactor debt, not a Rust limitation. The `crates/server/src/tools/*.rs` files mostly define tool-name constants today, so new tools still add most schema, dispatch, and implementation code to `main.rs`.

## Target structure

Keep protocol ownership in `server`, and keep edit/process semantics in `core`. Split server code by responsibility:

```text
crates/server/src/
  main.rs                     # parse args and call server::run
  server.rs                   # stdio loop and request routing
  protocol/                   # JSON-RPC/MCP protocol types
    metadata.rs               # server protocol metadata such as PROTOCOL_NAME
    response.rs               # JSON-RPC success/error response formatting
  tools/
    mod.rs                    # registry: tool list and dispatch table
    schema/                   # grouped MCP schema builders
      mod.rs                  # combines domain schema fragments in tool-list order
      files.rs                # file/edit/artifact tool schemas
      fixtures.rs             # fixture/base-image/manifest schemas
      git.rs                  # Git workflow schemas
    common.rs                 # arg parsing, path normalization, JSON helpers
    files.rs                  # read/diff/replace/write/create/hash/artifact/bulk handlers
    fixtures.rs               # fixture generator/base-image/manifest handlers
    process.rs                # guarded commands, logs, validation profiles
    git/                      # narrow Git workflows; not a raw Git passthrough
      mod.rs                  # Git domain exports
      names.rs                # public Git tool-name constants
      support.rs              # shared Git validation/status/ref helpers
      handlers/               # grouped workflow adapters
        commit.rs             # exact/scoped commit handlers
        restore.rs            # guarded restore/delete handlers
        sync.rs               # remote/branch/merge/push handlers
    github.rs                 # PR/fork handlers
    setup.rs                  # setup-profile adapter
    native.rs                 # native build/device adapters
    capability.rs             # capability_manifest and preflight_health
```

Each tool module should own its public tool names, schema fragments, handlers, and helpers that are local to that domain. Shared helpers should stay small and named by boundary rather than convenience.

Avoid one-file-per-tool modules when those files only contain constants or tiny wrappers. Group related tool names and adapters by domain so `tools/` stays balanced instead of becoming a flat directory with dozens of trivial files.

## Phased implementation

1. **Introduce routing types without behavior changes.** Add shared types such as `ToolContext`, `ToolResult`, and `ToolSpec`, then move dispatch mechanics into `tools/mod.rs` while old handlers remain callable.
2. **Move schema construction.** Extract `tool_definitions()` into grouped schema builders under `tools/schema/` and keep the tool-list test asserting the same public tool names.
3. **Extract common helpers.** Move JSON argument parsing, repo path normalization, and shared base64/SHA-256 helpers only when multiple modules need them. Preserve existing refusal text.
4. **Move file and fixture handlers first.** Extract read/diff/replace/write/create/artifact/hash/bulk handlers, then fixture generator, base-image check, and manifest verify/refresh handlers. These have focused tests and limited Git coupling.
5. **Move process and validation code.** Extract guarded command adapters, command-log helpers, validation profiles, and preflight process probes. Validation profiles should be data-style definitions so adding a profile does not touch routing.
6. **Move Git and GitHub workflows.** Extract Git helpers and commit/restore/remote/branch/merge/push handlers into a balanced `tools/git/` domain directory; extract PR/fork workflows into `tools/github.rs`. Keep the no-broad-Git boundary explicit.
7. **Move setup and native adapters.** Extract setup-profile, native-build, and native-device MCP adapters. These modules should only adapt JSON to typed `core` calls.
8. **Shrink `main.rs`.** Leave `main.rs` with only process entrypoint concerns, CLI argument parsing, and a call into the stdio server runner.

## Guardrails

- Prefer one extraction group per commit.
- Do not change MCP tool names, JSON shapes, confirmation literals, refusal text, or dry-run defaults unless a test is intentionally updated.
- Keep MCP schema and response formatting in `server`; do not move protocol details into `core`.
- Keep filesystem, Git guard, process guard, setup, and native policy in `core`; server modules only adapt JSON requests to typed core APIs.
- Treat `tools/git/support.rs` as transitional: helpers that encode reusable Git safety semantics should migrate to `core::git` when CLI or other adapters need them. MCP-only request shaping and response formatting should stay in `server`.
- The migration pattern for that move is settled: `core` returns the refusal *detail* only, and the server
  adapter supplies the `"<tool> refused: "` prefix. That keeps every public refusal string byte-identical
  while the policy relocates, and it is what makes each slice behavior-neutral and separately reviewable.
  Request validation moved first, under `core::git::validate`, followed by Git invocation and state
  queries under `core::git::state`.
- Keep the directory tree balanced. Do not move all code from `main.rs` into a crowded `tools/` root; split by domain and promote shared runtime/protocol code out of `tools/`.
- Update public docs only if public behavior changes. Pure module movement should not require tool-spec updates.

## Validation

Run focused validation after each extraction group:

```bash
cargo fmt --all -- --check
cargo test -p server stage1_mcp_tools_work_together
```

Run full validation before merging the completed refactor:

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p server --bin contextpatch-server
git diff --check
```

## Success criteria

- `crates/server/src/main.rs` is under 150 lines.
- No module exceeds roughly 800 lines without a single clear responsibility.
- Adding a new tool requires editing its domain module and registry, not the entrypoint.
- Existing server integration tests pass without public behavior changes.
