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

# MCP tool blockers — reported 2026-08-11

Reported by a Claude Desktop session running against this server. Each item was re-verified against source before being recorded here; the allowlist authority is `validate_command` in `crates/core/src/process/guarded_command.rs:109`.

## Verification summary

| Reported | Status | Evidence |
| --- | --- | --- |
| `bash` limited to `references/check-base-image.sh` | Confirmed | `guarded_command.rs:147`; test refuses `bash scripts/anything.sh` at `:268` |
| `bun test` allowlisted, so `bash` is transitively reachable | Confirmed | `:139` (`bun` → `run`/`test`) |
| `docker` unreachable from `run_guarded_command` | Confirmed | absent from `is_allowlisted_program` `:174`; test asserts refusal `:509` |
| 600s command cap | Confirmed | `DEFAULT_MAX_TIMEOUT_SECS` `:9`; `harbor run` alone gets 3600 `:10` |
| `psql` blocked | Confirmed | no occurrence in `core`/`server`; falls to `_ => false` `:152` |
| `set_file_executable` was available | Confirmed — retraction correct | exported in `crates/server/src/tools/mod.rs` |
| Git write tools were available | Confirmed — retraction correct | `git_stage_exact`, `git_commit_scoped`, `git_push_exact`, `git_restore_exact`, `git_branch_prepare` exported in `tools/mod.rs` |

## 1. `bash` for `scripts/*.sh`

Accurate as reported, and the porousness argument is correct: `validate_command` inspects only the direct program and argv, so a `bun test` child that calls `spawnSync("bash", …)` runs bash as a grandchild with no policy applied. The direct denial adds a detour, not isolation.

Worth noting this is not a newly discovered hole — it is the server's already-documented position. `docs/execution-threat-model.md` and the README both state that reviewed repository code run by an allowlisted program inherits the server user's permissions and network, and that npm-family scripts may start their own shell. That strengthens the ask rather than weakening it: the restriction is paying a real usability cost for isolation the threat model never claimed.

**The ask collides with a written contract line.** Both `README.md` and `.github/copilot-instructions.md` state that `run_guarded_command` "must not expose shell strings, arbitrary shell scripts, Docker, `pip`, arbitrary `python -m`, or package installation as generic execution." Granting `bash scripts/*.sh` is a safety-contract amendment, not an allowlist tweak, and per the documentation contract it needs `docs/safety-contract.md`, `README.md`, and the Copilot instructions changed in the same commit.

Two shapes, maintainer's call:

- **Narrow** — extend the existing exact-path pattern at `:147` to the five named gates (`check-docs.sh`, `check-doc-commands.sh`, `docs-audit.sh`, `check-endpoint-literals.sh`, `check-hosted-target-readiness.sh`). Keeps "no arbitrary shell scripts" literally true; costs an edit per new gate.
- **Glob** — allow `bash` for repo-relative `scripts/*.sh` under the existing argument-path confinement. Matches the trust level of `bun run`, which is already permitted and can do anything; requires retiring the contract sentence above rather than reinterpreting it.

## 2. `docker` and `docker compose`

Confirmed, with one correction to the framing: three typed docker paths exist, not one. `docker_image_inspect` (read-only), `image_cleanliness_check_run` (fixed `docker run --rm --network none --entrypoint find`), and `task_image_python_run`, which does build and run the task image (`crates/core/src/process/task_image.rs:301,338,380`). None of them accept generic arguments, so the blocker stands for stack-level proofs.

**Granting item 1 grants much of item 2 implicitly.** A `scripts/*.sh` gate that runs `docker compose up` reaches docker transitively by exactly the mechanism described in item 1. That may well be the intent, but it should be a decision rather than a discovery — a maintainer who grants item 1 believing item 2 remains blocked has been misled by the tool boundary.

## 3. The 600s cap — correct, but conditional

The cap is real and reported accurately. It is not the *next* binding constraint today, because docker is unreachable from `run_guarded_command` at all; 600s only starts binding once item 1 or item 2 is granted and a script can reach a docker build. Sequencing note, not a disagreement.

The async lane is not the escape hatch it looks like. `validation_profile_run`, `task_image_python_run`, and `harbor_run_start` return a `log_id` immediately and so are exempt from reply deadlines (`run_guarded_command` is deliberately absent from `deadline_for`, `crates/server/src/tools/dispatch.rs:399`), but `validation_profile_run` accepts only five server-defined profile names — `repo-basic`, `rust-workspace`, `datacore-vscode`, `datacore-m6-vscode`, `dynamo-harbor-task` (`crates/server/src/tools/process.rs:1066`), with their command lists owned by the server at `:1027`. An arbitrary `product-health.sh` cannot be handed to it.

So a long docker-driven health run needs one of: a raised cap on the synchronous path, or a new named profile in the async lane. The second fits the existing design better and keeps the reply-deadline guarantee intact.

## 4. `psql`

Confirmed blocked, deferred by the reporter as the other developer's lane. No action requested.

---

# Blocker status — updated 2026-08-11, branch `guarded-shell-script-list`

Six commits, pushed, unmerged. The report above is kept as the evidence record and not rewritten.

**The line numbers cited above are pre-change and have moved.** They are evidence of what was measured at the time, not current citations. `validate_command` is still the allowlist authority.

| Item | Then | Now |
| --- | --- | --- |
| 1. `bash` for gates | Blocked | **Resolved** in the narrow shape — `26cce7a` |
| 2. Docker stack proofs | Blocked | **Resolved** — `compose_stack_run`, `2c79ce2` |
| 2b. Artifact lane | Blocked | **Resolved** — `artifact_build_check_run`, `20bc287` |
| 3. 600s cap | Predicted next constraint | **Closed without changing the limit** |
| 4. `psql` | Blocked | Unchanged, still deferred |
| B1. Compose file mapping | — | **New, blocking** |
| B2. Nothing exercised for real | — | **New, blocking** |
| B3. Branch unmerged | — | **New** |

## 1. Resolved, but in the narrow shape rather than the glob

The ask was `bash scripts/*.sh`. That was refused as stated and granted as an exact enumeration, because safety-contract clause 19 requires a shell exception to be "fixed to a specific repository script... not arbitrary shell authority." A glob over a directory is exactly the authority the clause refuses: it would execute any file later written into that directory, putting the executable set under whoever can write there rather than under review.

All five named gates work. The gates are argument-free, so a caller cannot reach a listed script's own option surface. Clause 19 was restated to say *enumeration* explicitly and to name the glob as refused. Adding a gate is a one-line change to `ALLOWED_SHELL_SCRIPTS`.

The porousness argument was accepted and recorded rather than argued with: the denial never provided isolation, because `bun test` can already spawn bash as a grandchild. What the enumeration still buys is that the *directly launchable* set is fixed by review.

### Correction to the prediction in item 2 above

The earlier note said granting item 1 would implicitly grant much of item 2, and a later summary went further and said the enumerated, argument-free gates do **not** transitively unlock Docker. That second claim was too strong and is withdrawn.

Argument-freeness prevents a caller from *steering* a listed script. It does nothing about what the script's own body does. If `scripts/docs-audit.sh` internally runs `docker compose up`, Docker runs. The accurate statement is narrower: Docker reachability now depends on the reviewed contents of five named scripts rather than on anything a caller can choose or on any file that appears in `scripts/` later. That is a real difference from a glob, and it is not the same as unreachable.

## 2. Resolved, both halves, without allowlisting Docker

`docker` is still absent from `validate_command` and should stay that way. Both new tools follow the established pattern: argv derived in `core`, executed through `run_bounded_command`, which is legitimate precisely because the caller supplies no Docker arguments.

`compose_stack_run` — named stack proofs, dry-run by default, `confirm: "run compose stack"`, async `log_id`. Each action pinned to one compose file. Teardown is scoped to the server's own Compose project name, so it cannot stop a stack an operator started by hand from the same file, and runs after every outcome with `teardown_clean` reported separately.

`artifact_build_check_run` — `docker build` then a run of the built image. Unlike the compose tool the Dockerfile is caller-named, deliberately: there is no fixed set of artifacts the way there is a fixed set of stack proofs, and `task_image_python_run` already accepts a caller-named repository script on the same reasoning. Paths validate through `fs::rooted`, so traversal and symlinked components refuse before Docker is involved. Smoke arguments sit after the image name and so are the container command by construction; the tag is unique per plan; the image is always removed.

Both are `openWorldHint: true`. The network split in the artifact tool is the substantive choice: the build has the network, the import smoke is pinned `--network none` so a dead export cannot be masked by a successful download.

One statement was added to the threat model that had been implied but never written: `docker build` hands work to the Docker daemon, which conventionally runs as root and is not namespaced from the host the way the task image is. A Dockerfile's authority is at least the server user's.

## 3. Closed without raising the cap

The 600-second cap is unchanged and was not the binding constraint. It belongs to `run_guarded_command`, which neither new tool passes through. Both use the async background lane, which already carries per-command timeouts to 3600s and is exempt from reply deadlines. The correct fix was a typed async tool, not a larger number.

The two-job background cap now covers Compose and artifact work as well. Its refusal message previously named only three job kinds, which made a saturation refusal actively misleading; fixed in `1dff697`.

## 4. `psql` — unchanged

Still absent from the allowlist, still falls through to `_ => false`. No work done, still reads as the other developer's lane.

## B1. The compose action-to-file mapping is unconfirmed — blocking

`compose_stack_run` refuses **every** action until `STACK_ACTIONS` matches the real layout. The report named six proofs but not their compose paths, so the mapping uses `compose/<action>.yml`. That was pinned visibly rather than guessed silently: a path that is not a regular file refuses by name before Docker is invoked, so a wrong pin fails loudly with the exact string to correct.

This is the cheapest open unblock. One answer, six lines.

## B2. Nothing has been exercised against the real repository — blocking

Every claim above is verified by tests: 481 passing, clippy clean under `-D warnings`, `cargo fmt --check` now clean (it had been failing at ~110 pre-existing sites, cleaned in `4750246`). None of it has run against the platform repo. No gate has been launched through the real server and no stack has been started.

Two hazards on first use. The binary must be rebuilt — `cargo build --release -p server --bin contextpatch-server` — and Claude Desktop restarted, or `capability_manifest` reports the new tools as missing off a stale binary, which is the exact false negative `BUILD_GIT_SHA` exists to disambiguate. And B1 will surface on the first `compose_stack_run` call.

## B3. Branch unmerged

`guarded-shell-script-list` is pushed to `origin` and not merged. PR or direct merge is the maintainer's call.

## Method note

The reporter's closing commitment — enumerate with `capability_manifest` before asserting a limit — held up: every item in the original report verified against source. The two errors in this round were both mine and both in the same class as the one that commitment addresses. I asserted there was no docs-sync test when `schema/mod.rs` enforces `tool-spec.md` in both directions, and I overstated the transitive-Docker claim corrected under item 1. Both were fixed where they were recorded rather than only here.

## Retractions, and one finding underneath them

Both self-corrections are accurate — `set_file_executable` and the five Git write tools were available throughout.

The Git one is worth keeping rather than filing as simple error, because **both beliefs were true of different surfaces**: `run_guarded_command`'s `git` allowlist genuinely is read-only — `status`, `diff`, `log`, `show`, `rev-parse`, `ls-tree` (`guarded_command.rs:134`) — while write authority lives in separate typed tools that never appear in that allowlist. An agent that probes Git through guarded commands and receives a refusal will correctly conclude Git is read-only and be wrong about the server. That is a discoverability gap in the tool surface, not just a lapse in the reporter's memory, and it is the one item here that could be fixed by a refusal-message change: `guidance.rs` already produces per-program refusal suffixes and could point `git commit`/`git add` at `git_commit_scoped` and `git_stage_exact`.

**This supersedes the "My git access is read-only" claim in the note above** — that entry was written under the same misreading.

Reporter's own conclusion, endorsed: call `capability_manifest` before asserting a limit. One turn, and it is authoritative.
