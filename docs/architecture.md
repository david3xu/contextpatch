# Architecture

`contextpatch` is split into three layers so safe edit behavior is independent from any single agent host.

The architecture supports one product goal: provide a reusable safe patch layer for AI coding agents without turning into a generic filesystem server.

## Layers

| Layer | Package | Responsibility |
| --- | --- | --- |
| Core | `core` | Filesystem, patch, replacement, policy, and Git guard logic |
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
- Exact replacement semantics
- Patch validation and application semantics
- Diff generation semantics
- Git status inspection and tracked move behavior
- Error types shared by CLI and server

Must not own:

- MCP/context-server transport
- JSON-RPC framing
- Claude Desktop configuration
- Human CLI argument parsing

### `cli`

Owns:

- Kebab-case command names
- Terminal help text
- Exit codes
- Human-readable output

Must call `core` for edit behavior instead of reimplementing safety logic.

### `server`

Owns:

- Protocol-facing snake_case tool names
- Tool schemas
- Request/response adaptation
- Server startup and client transport

Must call `core` for edit behavior instead of owning filesystem mutation logic.

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
