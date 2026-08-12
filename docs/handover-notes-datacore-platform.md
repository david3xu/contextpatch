# Inherited handover note — datacore-platform

Not about this repository. These are commit-splitting notes from a session working on
`datacore-platform`, which arrived at the head of `docs/gaps.md` and were committed here by an
unscoped `git add -A`. They are kept because they have value to whoever wrote them, and moved
because a register of this repository's blockers should not open with another project's tree.

---

Yes, and for a reason beyond tidiness: **there is currently no checkpoint.** Roughly 60 files of coherent work sit uncommitted with no recovery point. A mistaken `checkout` or a revert loses all of it. That alone justifies committing before anything else.

Three more reasons: every gate is green right now, so this is a known-good state to capture; a large uncommitted tree sitting next to in-flight `clients/vscode/` work raises collision risk; and a fresh round starting from a clean baseline plus the handover brief will be more reliable than continuing to stack context.

## I can't do it

My git access is read-only: `status`, `diff`, `log`, `show`, `rev-parse`, `ls-tree`. No `add`, no `commit`. You'll need to run it.

## Do not use `git add -A`

Four things in the tree aren't part of this work:

```
 M .gitignore
 M CLAUDE.md
 M docs/working-notes/2026-08-06-lifecycle-and-memory-capability-review.md
?? clients/vscode/
```

The three `package.json` bumps to 0.1.75 are yours. My read is they belong with the demonstration bundle rather than with this feature work, but that's your call — I'm flagging it because you told me not to bundle a version bump into S6.5, and sweeping yours in would violate the spirit of that.

`.tmp/` is gitignored and needs no exclusion.

## Suggested split, three commits mapping to the three ADRs

**1. Decision records and plan docs.** The three ADRs, the design doc, the workload plan, `documentation-map.md`, `decisions/README.md`, `authority-policy.ts`. Gate: `./scripts/check-docs.sh` plus `./scripts/check-doc-commands.sh`.

**2. C2, Runtime-owned capability projection.** Schema, storage, publish and read endpoints, reader rewrite, Coordinator publish, `runtime-client`, contracts, tests. Gate: runtime typecheck, `audit drift`, `bun run test`.

**3. C1, dispatch intent pull.** Event type and schema, claim surface, spawn-attempt endpoint, poller, `worker-runtime` transport, failure-class vocabulary, Coordinator intent write, tests. Gate: same three plus the bridge and vocabulary lanes.

Generated artifacts must travel with their commit: `api-surface.md` and `_generated/platform-audit-events/coordination_dispatch_requested.json`, or drift fires.

Message convention: no attribution lines.

## One thing worth fixing before you commit

**ADR 0027's acceptance criterion 2 contains a false premise.** It says the re-sourcing must preserve "the existing operator remediation text." S6.5 established there was no existing remediation text — the failure class was a bare literal in two files. The criterion is satisfied by something better than it asked for, but committing an Accepted ADR with a wrong premise inside it is the same drift problem that cost us most of a pass this morning.

Small edit, five minutes, and it's the kind of thing that becomes archaeology if left. Say the word and I'll do it before you commit.

## What the new round should open with

Not code. The C1 demonstration, since it needs no Azure resources and is the only thing that can tell us whether any of this works. Then the three tally questions, which are a message rather than work. Then G1 and G3.

---
