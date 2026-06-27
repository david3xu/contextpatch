# Architecture

`contextpatch` is split into three layers so safe edit behavior is independent from any single agent host.

## Layers

| Layer | Crate | Responsibility |
| --- | --- | --- |
| Core | `contextpatch-core` | Filesystem, patch, replacement, policy, and Git guard logic |
| CLI | `contextpatch-cli` | Human-facing command-line UX |
| Server | `contextpatch-server` | Context-server protocol adapter and tool schema |

## Boundary rule

The core crate must not know about the server protocol. Server tools call core operations; they do not own edit semantics.

This keeps the edit engine reusable by the CLI, context server, editor integrations, and tests.
