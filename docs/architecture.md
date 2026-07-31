# Architecture

`contextpatch` is split into three layers so safe edit behavior is independent from any single agent host.

The architecture supports one product goal: provide a reusable safe patch layer for AI coding agents without turning into a generic filesystem server.

## Layers

| Layer | Package | Responsibility |
| --- | --- | --- |
| Core | `core` | Filesystem, patch, replacement, policy, process-runner, setup-profile planning, native build/device policy, and Git guard logic |
| CLI | `cli` | Human-facing command-line UX |
| Server | `server` | Context-server protocol adapter and tool schema |

## Boundary rule

The core crate must not know about the server protocol. Server tools call core operations; they do not own edit semantics.

This keeps the edit engine reusable by the CLI, context server, editor integrations, and tests.

The safe edit engine is the product center. CLI and server crates are adapters.

## Crate ownership

### `core`

Owns:

- Path normalization and repository-root checks
- Atomic writes
- Shared SHA-256 validation
- Stable repository-specific scratch storage
- Cooperative repository and per-file mutation locking
- Durable cross-process mutation receipt storage
- Exact replacement semantics
- Patch validation and application semantics
- Diff generation semantics
- Git status inspection and tracked move behavior
- Guarded validation command policy and execution
- Direct-operation reply deadline policy
- Shared no-shell process execution machinery for guarded workflows
- Declarative setup-profile planning and profile-owned command derivation
- Typed native build/test command planning and source-status validation
- Typed native simulator/emulator/device command planning and confirmation gates
- Error types shared by CLI and server

Must not own:

- MCP/context-server transport
- JSON-RPC framing
- Claude Desktop configuration
- Human CLI argument parsing
- MCP tool schemas or setup-profile JSON adaptation

### `cli`

Owns:

- Kebab-case command names
- Terminal help text
- Exit codes
- Human-readable output
- Safe Claude Desktop configuration updates, cooperative locking, stale-read checks, backups, and wildcard tool-policy installation

Must call `core` for edit behavior instead of reimplementing safety logic.

### `server`

Owns:

- Protocol-facing snake_case tool names
- Tool schemas
- Request/response adaptation
- Server startup and client transport

Must call `core` for edit behavior instead of owning filesystem mutation logic.

Server setup and native tools are adapters only: they parse JSON into typed core params, call `core::setup`, `core::native_build`, or `core::native_device`, and format MCP responses. They must not derive package-manager, build, simulator, or device commands directly.

The server crate should keep `main.rs` small and move tool schemas, dispatch, handlers, and domain helpers into modules under `crates/server/src/tools/`. See [Server Refactor Plan](server-refactor-plan.md) for the planned split.

## Core feature organization

Core code is grouped by product capability rather than adapter:

| Module | Owns |
| --- | --- |
| `fs` | Repo-root-confined reads and create-only atomic writes |
| `fs::hash` / `fs::mutation_lock` / `fs::receipt` / `fs::scratch` | Shared digests, cooperative mutation serialization, durable mutation evidence, and stable outside-repository scratch storage |
| `patch` / `replace` | Anchored edit and diff semantics |
| `git` | Git status and guarded Git workflow helpers |
| `process` | Shared no-shell execution, reply deadlines, refusal guidance, timeout policy, redaction, and command capture |
| `setup` | Declarative setup profiles and typed command-plan derivation |
| `native_build` | Typed iOS/Android build and test command derivation plus source-status validation |
| `native_device` | Typed iOS simulator and Android device command derivation plus device-state confirmation policy |

`process` is infrastructure, not a public shell surface. `setup` may plan external mutators, but profile modules own the action vocabulary and command plans. `native_build` and `native_device` expose high-level actions, not arbitrary native command access.

## Dependency direction

```text
cli     -> core
server  -> core
core    -> standard library / focused implementation dependencies
```

`core` must not depend on `cli` or `server`.

## Testing strategy

Core behavior should be tested in the core crate first. CLI and server tests should verify argument/protocol mapping and should not duplicate every edit-engine test.

Repository-level integration tests should cover:

1. Successful exact replacement
2. Zero-match refusal
3. Multi-match refusal
4. Atomic write behavior where practical
5. Path traversal refusal
6. Dirty-repository guard behavior
7. Capability discovery and guarded command refusal behavior
8. Setup-profile dry-run planning and refusal behavior
9. Native build/device dry-run planning and refusal behavior
10. Stale complete-file SHA refusal in shared-worktree replacement
11. Durable mutation receipt settlement and interrupted-receipt recovery
12. Claude Desktop wildcard policy configuration, backup, preservation, and idempotence
13. Cooperative mutation-lock contention and deadline-worker saturation
14. Claude Desktop stale-read refusal and Windows executable detection
