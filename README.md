# contextpatch

Guarded patch editing for AI context servers.

`contextpatch` is a small Rust tool for safe, reviewable repository edits. It is designed for agent workflows where whole-file writes are too risky, too broad, or too unreliable.

The project principle is simple: **every write must be anchored, atomic, and reviewable**.

## Why this exists

AI desktop tools often expose generic filesystem writes. That is convenient, but it can fail poorly: large file rewrites, timeout-prone operations, accidental overwrites, and weak protection around concurrent repository changes.

`contextpatch` takes the opposite approach. It should prefer small guarded edit operations over broad writes.

## MVP scope

| Capability | Purpose |
| --- | --- |
| `read-range` | Read a bounded section of a file |
| `diff-preview` | Preview a proposed edit before writing |
| `replace-exact` | Replace text only when the expected anchor appears exactly once |
| `apply-patch` | Apply unified patches with repository guardrails |
| `status` | Show clean/dirty Git state before edits |
| `serve` | Run a local context-server interface for agent tools |

## Safety contract

1. Do not overwrite whole files by default.
2. Require anchors, hashes, or patch context for writes.
3. Write atomically through a temporary file plus rename.
4. Refuse ambiguous replacements.
5. Surface diffs before persistent changes when requested.
6. Never hide Git state from the caller.

## Current status

This repository is newly scaffolded as a Rust workspace. The first implementation target is a local CLI with the same primitives that the server interface will expose.

## Repository layout

```text
crates/contextpatch-core/      safe edit engine
crates/contextpatch-cli/       human CLI
crates/contextpatch-server/    context-server adapter
docs/                          public design and usage docs
tests/                         repo-level fixtures and integration tests
```
