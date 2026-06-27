# contextpatch

The safe patch layer for AI coding agents.

`contextpatch` is a Rust tool for safe, reviewable repository edits. It is designed for agent workflows where broad filesystem writes are too risky, whole-file rewrites are too unreliable, and every persistent change should be easy to audit.

The project principle is simple: **every write must be anchored, atomic, and reviewable**.

## Product thesis

AI agents should not get broad filesystem write power by default. They should get small, guarded, reviewable edit primitives.

`contextpatch` aims to be the safe patch layer between AI coding agents and a repository. It should be useful as a CLI, a local context server, and a reusable edit engine for future editor or agent integrations.

## Why this exists

AI desktop tools often expose generic filesystem writes. That is convenient, but it can fail poorly: large file rewrites, timeout-prone operations, accidental overwrites, and weak protection around concurrent repository changes.

`contextpatch` takes the opposite approach. It should prefer small guarded edit operations over broad writes.

## Product boundary

`contextpatch` is not a general filesystem server. It is not a shell runner. It is not a replacement for Git.

The project owns one narrow surface: **safe repository editing primitives for agent clients**. Anything outside that boundary should be rejected unless it directly supports anchored edits, reviewable diffs, or repository guardrails.

That boundary is intentional product strategy, not a temporary limitation.

## MVP scope

| Capability | Purpose |
| --- | --- |
| `read-range` | Read a bounded section of a file |
| `diff-preview` | Preview a proposed edit before writing |
| `replace-exact` | Replace text only when the expected anchor appears exactly once |
| `write-new-file` | Create a file only when it does not already exist |
| `status` | Show clean/dirty Git state before edits |
| `serve` | Run a local context-server interface for agent tools |

## First implementation order

1. `replace-exact` in `core`
2. `replace-exact` in `cli`
3. `read-range` in core and CLI
4. `write-new-file`
5. `diff-preview`
6. `status`
7. Stage 1 server schemas and transport

The first useful milestone is a CLI command that can safely replace exactly one matched text span and refuse zero-match or multi-match edits. The full staged plan is in `docs/implementation-roadmap.md`.

```bash
contextpatch read-range <path> --start <line> --end <line>
contextpatch diff-preview <path> --old <text> --new <text>
contextpatch replace-exact <path> --old <text> --new <text>
contextpatch write-new-file <path> --content <text>
```

## Safety contract

1. Do not overwrite whole files by default.
2. Require anchors, hashes, or patch context for writes.
3. Write atomically through a temporary file plus rename.
4. Refuse ambiguous replacements.
5. Surface diffs before persistent changes when requested.
6. Never hide Git state from the caller.

See `docs/safety-contract.md` for the full contract.

## Current status

This repository is a new Rust workspace. The docs define the product contract, and Stage 1 implementation has started with `replace-exact`, `read-range`, `write-new-file`, and `diff-preview` in the core crate, CLI, and MCP server. Code changes should keep the relevant Markdown file synchronized in the same commit.

## Repository layout

```text
crates/core/                   safe edit engine
crates/cli/                    human CLI
crates/server/                 context-server adapter
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
| `docs/implementation-roadmap.md` | Stage scope, sequencing, or release criteria change |
