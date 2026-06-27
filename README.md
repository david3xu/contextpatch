# contextpatch

Guarded patch editing for AI context servers.

`contextpatch` is a small Rust tool for safe, reviewable repository edits. It is designed for agent workflows where whole-file writes are too risky, too broad, or too unreliable.

The project principle is simple: **every write must be anchored, atomic, and reviewable**.

## Why this exists

AI desktop tools often expose generic filesystem writes. That is convenient, but it can fail poorly: large file rewrites, timeout-prone operations, accidental overwrites, and weak protection around concurrent repository changes.

`contextpatch` takes the opposite approach. It should prefer small guarded edit operations over broad writes.

## Product boundary

`contextpatch` is not a general filesystem server. It is not a shell runner. It is not a replacement for Git.

The project owns one narrow surface: **safe repository editing primitives for agent clients**. Anything outside that boundary should be rejected unless it directly supports anchored edits, reviewable diffs, or repository guardrails.

## MVP scope

| Capability | Purpose |
| --- | --- |
| `read-range` | Read a bounded section of a file |
| `diff-preview` | Preview a proposed edit before writing |
| `replace-exact` | Replace text only when the expected anchor appears exactly once |
| `apply-patch` | Apply unified patches with repository guardrails |
| `status` | Show clean/dirty Git state before edits |
| `serve` | Run a local context-server interface for agent tools |

## First implementation order

1. `replace-exact` in `contextpatch-core`
2. `replace-exact` in `contextpatch-cli`
3. `read-range` in core and CLI
4. `diff-preview`
5. `status`
6. `contextpatch-server` protocol transport

The first useful milestone is a CLI command that can safely replace exactly one matched text span and refuse zero-match or multi-match edits.

## Safety contract

1. Do not overwrite whole files by default.
2. Require anchors, hashes, or patch context for writes.
3. Write atomically through a temporary file plus rename.
4. Refuse ambiguous replacements.
5. Surface diffs before persistent changes when requested.
6. Never hide Git state from the caller.

See `docs/safety-contract.md` for the full contract.

## Current status

This repository is newly scaffolded as a Rust workspace. The docs now define the product contract before implementation. Code changes should keep the relevant Markdown file synchronized in the same commit.

## Repository layout

```text
crates/contextpatch-core/      safe edit engine
crates/contextpatch-cli/       human CLI
crates/contextpatch-server/    context-server adapter
docs/                          public design and usage docs
tests/                         repo-level fixtures and integration tests
```

## Documentation contract

| File | Must change when |
| --- | --- |
| `docs/tool-spec.md` | A tool is added, removed, renamed, or its behavior changes |
| `docs/safety-contract.md` | A write rule, guard, or refusal policy changes |
| `docs/architecture.md` | Crate boundaries or ownership changes |
| `docs/claude-desktop.md` | Server install/config behavior changes |
