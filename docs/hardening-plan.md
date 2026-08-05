# Hardening Plan

This plan collects every concern raised by a user-side and implementation-level audit of
`contextpatch`. It is rebased onto commit `e0f56b0`, which absorbed the earlier audit fixes together
with the repository's own hardening work.

Ordering follows dependency and daily impact rather than severity alone, because several fixes are
cheap only after an earlier one lands. Each item carries an identifier so the
[concern index](#concern-index) can show that nothing was dropped.

## Principles

These four rules explain most of the sequencing below.

1. **Every enforcement claim an agent reads must be derived from the code that enforces it.** A
   restated claim is a claim that can drift, and an agent trusts the claim to decide what is safe.
2. **One name, one implementation.** Two helpers with the same name and different semantics are a
   defect, not a style preference.
3. **Variance belongs in parameters, not in tool count.** A new tool is justified by a new
   capability, never by a new way of selecting inputs.
4. **A guard that can silently degrade is worse than a guard that fails.** Fallbacks in guard paths
   must refuse rather than continue with weaker protection.

## Status

| ID | Concern | State |
| --- | --- | --- |
| C01 | In-place replacement discarded the target's permission bits | Complete |
| C02 | Configured root was never validated at startup | Complete |
| C03 | Critical modules and the whole server test tree were untracked | Complete |
| C04 | Build stamp could not be pinned to a dirty tree | Complete |
| C05 | Installed binary predates the landed fixes | **Unverified** |

C01 preserves the target's permission bits across an in-place replacement, reading the mode from the
held descriptor and applying it to the temporary file before the rename. Three named constants
replaced inline literals, including a retry count that had been restated in prose inside its own
failure message.

C02 resolves the configured repository root once at startup, canonicalizing it and verifying it is a
directory. It deliberately does **not** require a Git worktree root, because a plain directory
holding descendant worktrees is a supported configuration reached through the wrapper `repository`
selector. Requiring a worktree root belongs to the tools that need one, which is Phase 3.1.

C03 is satisfied by `e0f56b0`. The previously untracked filesystem modules, the task-image process
module, and the entire server test tree now have history.

C04 adds a deterministic fingerprint of the uncommitted delta. See
[Phase 1.2](#12-make-the-build-stamp-pinnable-complete) for the derivation and its guarantees.

C05 stays open and is deliberately **not** claimed as verified. The running server still reports an
earlier build stamp, so no assertion is made here about what is installed. Verification requires a
release rebuild, a reinstall, and a fresh `capability_manifest` read whose `git_sha` and
`git_dirty_fingerprint` match the tree under test.

## Phase 1 — Recover history and pin provenance

### 1.1 Commit the untracked implementation (complete)

Satisfied by `e0f56b0`. Retained here because the failure mode is worth keeping on record: the
filesystem module declaration was tracked and modified while three of the modules it declared were
untracked, so committing only the tracked modifications would have produced a tree that does not
build. `git_commit_exact` is the correct tool for that shape, because it requires the provided path
list to match the complete dirty set and therefore refuses an incomplete split. The scoped and prefix
variants permit a subset and would have produced the broken commit.

### 1.2 Make the build stamp pinnable (complete)

`capability_manifest` reported `git_sha` plus a `git_dirty` boolean. When the tree is dirty the sha
still matches `HEAD`, so the manifest's own advice to compare sha against the checked-out commit
returned a false all-clear and the running server could not be matched to a known source state.

The build stamp now carries `git_dirty_fingerprint`, derived as follows:

- `git status --porcelain=v1 -z --untracked-files=all` enumerates the uncommitted paths, including
  untracked files. Rename and copy records are paired with their origin record so both endpoints
  contribute, rather than the origin's leading characters being misread as a status code.
- `git hash-object` supplies a content digest per path, batched so a large dirty set neither spawns
  one process per file nor overruns the platform argument limit. A path that no longer exists is
  recorded with an absent marker, distinct from the digest of an empty file, so deleting a file and
  truncating it do not collapse to the same stamp.
- Entries are sorted and rendered as a canonical manifest of path, status, and content digest, then
  combined into a fixed-width value.

Three properties are deliberate. **Determinism**: sorting makes the result independent of Git's
reporting order. **No content exposure**: the module never reads a file. Digests are computed by Git
and supplied by the caller, so contents cannot reach the stamp, a build log, or a failure message,
and the rendered stamp is fixed-width so it cannot leak how much is uncommitted. **No new build
dependency**: the combining step is self-contained, honoring the build script's existing constraint
against external crates.

The logic lives in a module the crate compiles normally and the build script includes directly,
because a build script cannot depend on the crate it builds. The shipped stamp and the tested logic
are therefore the same code. Inner doc comments are avoided in that file for the same reason: the
inclusion splices it mid-script, where they are not legal.

The stamp reads `clean` for a clean worktree and `unknown` when provenance could not be determined,
so a consumer never branches on absence and a clean build stays distinguishable from an unknown one.
The manifest note and the operator-facing staleness guidance were both corrected, because both
previously told the reader that a matching sha was sufficient.

### 1.3 Rebuild and reinstall

Open, and tracked as C05. Until this lands, `capability_manifest` describes a binary older than the
source tree.

## Phase 2 — Daily edit ergonomics (complete)

The highest-frequency friction points for routine use, all independent of the structural work that
follows. All four concerns landed one per commit, each with focused tests and documentation
synchronized in the same commit. The subsections below are retained as the record of what was
delivered and why.

### 2.1 Allow multiple hunks per file (C06)

`bulk_replace_exact` rejects two entries naming the same file as a duplicate path or filesystem
alias. The most common real edit shape, several hunks in one file, is therefore the one shape the
batch tool refuses, forcing sequential calls that cannot be validated or applied as a set.

Accept multiple hunks per file. Validate them as non-overlapping byte ranges against a single
captured snapshot, then apply one atomic replacement per file. Keep the existing refusal for
*cross-file* duplicates and hard-link or case aliases; that check is correct and must not be relaxed
to implement this.

### 2.2 Return the resulting digest from mutating file responses (C07)

`replace_exact` reports byte offsets and total size but not the resulting SHA-256, so a caller making
sequential edits cannot chain `expected_sha256` without an extra `file_info` round trip per hunk.
That is the guard most worth keeping on a shared worktree, and the cost of an extra call is exactly
what discourages using it.

Return the post-write digest from `replace_exact`, `write_existing_file_exact_hash`,
`write_new_file`, `write_new_file_base64`, and the bulk variants.

### 2.3 Record receipts for validation refusals (C08)

A `bulk_replace_exact` refusal during validation leaves no entry in the receipt journal, so
`read_write_receipts` cannot distinguish "refused before any write" from "never attempted". The
single-file path already records a refused outcome; the bulk path should match it.

### 2.4 Advertise the cheap discovery paths (C09)

The `project_execute` input schema documents only `action`. It never mentions that `describe` accepts
a name to return one schema, so the natural first call returns every full schema and pays that cost
in every session. Two further projections are equally undiscoverable: `capability_manifest` accepts
`names_only`, and `preflight_health` accepts a minimal response mode while still reporting the
repository root, which is the fastest way to confirm which repository a server owns.

Document all three in the wrapper description, and include `describe` in the reported action name
list so a client enumerating actions can discover the meta action at all.

## Phase 3 — Consolidate the Git trust boundary

`docs/server-refactor-plan.md` already designates the server-side Git support module as transitional,
with helpers that encode reusable Git safety semantics expected to migrate to `core::git`. The
duplicate helpers below are that unfinished migration, not a new defect.

This phase is ordered so that policy is *decided* before it is *moved*, and moved before it is
*deduplicated*. Consolidating helpers first would bake in whichever semantics happened to survive.

### 3.1 Define per-tool worktree requirements (C12) — complete

Each Git action now declares its policy explicitly, so the classification is auditable rather than
implied by which helper a handler happened to call. Actions that name paths keep the resolving-only
policy, because path normalization already bounds them and a descendant root stays useful. Actions that
name no paths, `git_branch_prepare` and `git_push_exact`, require the configured root to be exactly a
Git worktree root; the strict check is delegated to `core` so no second implementation was grown.

This does not close C11: the two canonicalize-only helpers that share the name `canonical_repo_root`
still exist and are removed in 3.3.

Decide explicitly, per tool, whether an exact worktree root is required, and state it in the schema.
Today the same configured root can be accepted by the status path and refused by the mutation path.

The path-taking tools already fail closed, because path normalization rejects parent components,
absolute paths, and pathspec metacharacters. The tools that take no paths are the exposure:
`git_branch_prepare` and `git_push_exact` act on the enclosing repository, so branch switching and
pushing are not bounded by the configured root. Write this decision down before moving any code, so
the migration has a specification to preserve.

### 3.2 Migrate Git policy into `core` (C10)

The refactor plan's guardrail keeps filesystem, Git, process, setup, and native policy in `core`,
with server modules adapting JSON to typed core calls. Tracked move and guarded delete follow that
rule; the commit, restore, and sync handlers carry real policy. Move that policy into `core::git` and
leave request shaping and response formatting in `server`. This is a module move and must carry no
behavior change.

### 3.3 Consolidate the repository-root helpers (C11)

| Location | Behavior | Call sites |
| --- | --- | --- |
| `core::git::guarded_path` | canonicalize, verify directory, require the resolved top level to equal the root | 3 |
| `core::git::status` | canonicalize only | 3 |
| `server::tools::git::support` | canonicalize only | ~20 |

Three functions share one name and two semantics. The strong version backs three call sites; the
canonicalize-only versions back roughly twenty, including every commit, stage, restore, remote,
branch, and push workflow. Because startup now resolves the root once (C02), both weak versions are
pass-throughs, so removal is mechanical. Keep one definition, in `core`.

### 3.4 Rebuild the workspace selector on descriptors (C13)

The selector is otherwise sound: it rejects parent and current-directory components, absolute paths,
trailing and doubled separators, backslashes, control characters, drive prefixes, and the Git
administrative directory; requires the re-joined path to equal the input; rejects a symlink at every
component; and then requires an exact worktree root. It cannot escape the workspace boundary.

It reaches that result by path, however, inspecting and then reopening by path later. That is the
race-prone pattern the rest of the module avoids by design with descriptor-relative opens and
revalidation. Walk the selector with descriptors and hand the resulting directory descriptor forward,
so the checked object and the used object are the same object.

## Phase 4 — Derive the capability contract

This phase removes the root cause of the duplicated-literal class. **It must complete before Phase
5.3 restructures any handler module**, because collapsing or moving handlers while the contract is
hand-maintained means editing five files per change and silently accepting drift.

### 4.1 One source for each confirmation phrase (C14)

Every confirmation phrase exists at least twice with no linkage: once as a comparison literal and
again as prose inside a schema description, plus a third copy in the tool specification. Add a
confirmation constant beside each tool's existing name constant, and build every schema description
from that constant rather than restating it.

### 4.2 Generate the capability manifest (C15)

The capability module is roughly forty-four thousand bytes serving two tools, because it
hand-maintains the entire contract as inline JSON. Derive it from the schema registry and from the
guard implementations instead.

### 4.3 Remove duplicated enforcement prose (C16)

Strings describing what the code enforces are hand-copied across five files: the capability module
itself, one schema module, the README, the safety contract, and the native background implementation
note. None is derived from the enforcing code, so drift is silent and the manifest can misreport what
is actually guaranteed. Public docs should reference the generated contract rather than restate it.

### 4.4 Add drift tests (C17)

Assert, for every action, that the confirmation phrase quoted in its schema description equals its
confirmation constant, and that every capability claim is generated rather than literal. Acceptance:
changing a phrase or a claim in one place fails the test.

## Phase 5 — Tool surface consolidation (blocked, design first)

**Do not execute this phase.** Merging tools changes the public surface that clients bind to, and no
merge should land before the consequences are written down and agreed.

### 5.0 Required deliverable: versioned API proposal (C33)

Write a versioned proposal covering, at minimum:

- **Compatibility.** Which existing action names remain callable, and for how long.
- **Deprecation.** How a superseded action announces itself, whether it warns in-band, and what a
   client sees during the overlap window.
- **MCP annotations.** How merged tools present `readOnlyHint`, `destructiveHint`, `idempotentHint`,
   and `openWorldHint` when one tool spans both a preview and a mutation.
- **Approval behavior.** Whether a merged tool changes when a host prompts for approval. A tool that
   was read-only becoming conditionally destructive is an approval-surface change, not a refactor.
- **Schema implications.** How mutually exclusive parameters are expressed, since a permissive object
   schema moves validation from the schema to runtime and weakens what a client can check ahead of
   the call.
- **Versioning.** How the surface version is reported and how a client detects which shape it is
   talking to.

Only after that proposal is accepted should any of the following be implemented.

### 5.1 Candidate merges (C19)

| Merge | Rationale |
| --- | --- |
| The three commit tools into one | They differ only in how the path set is selected. Three near-identical confirmation phrases also invite sending the wrong one |
| `diff_preview` into `replace_exact` with `dry_run` | `diff_preview` is a dry-run replacement, and `replace_exact` is the only mutating tool with no `dry_run`, so one problem is solved two ways |
| `fixture_manifest_verify` into `fixture_manifest_refresh` with `dry_run` | Same shape |
| Text and base64 write pairs | Payload encoding is a parameter, not a tool |
| Single into bulk for replacement and file creation | The bulk form is the general case; the single form is a one-element batch |
| The three cleanup tools into one | They differ only by selector |
| The two Docker wrappers | Use the typed-action idiom already used for native builds |

The surface currently expresses variance four ways: separate tools per variant, a `dry_run` boolean,
a typed action with an untyped parameter object, and the wrapper's string dispatch over all of the
above. The proposal should choose one idiom and say why.

### 5.2 Candidate splits (C20)

- The GitHub workflow tool carries nine actions and roughly fifteen optional parameters whose
  validity depends entirely on the chosen action, which the schema cannot express, so all validation
  is deferred to runtime.
- The native device tool carries eleven actions across two platforms with an untyped parameter
  object, which is an untyped hole in an otherwise strongly typed surface.

Both are candidates for per-action schemas, and both are in scope for 5.0.

### 5.3 Rebalance the handler modules (C21)

Gated on Phase 4. Schemas are already centralized and the Git domain was split into a balanced
directory; the remaining handler modules were not, and one of them shrinks on its own once the
contract is generated. Pure module movement, no behavior change.

## Phase 6 — Write-path durability and residue

**Each item is its own commit.** These are independent, and durability changes should be reviewable
in isolation rather than bundled.

### 6.1 Flush the parent directory after rename (C22)

Both write paths synchronize the temporary file and then rename without flushing the parent
directory, so a rename can be lost on power loss while a receipt records the write as applied. In the
guarded path the parent descriptor is already held, so this is a single call.

### 6.2 Retire the duplicate write implementation (C23)

The standalone atomic write module is presented in the README as the project's safety primitive but
backs only the receipt journal; real repository writes go through the guarded descriptor path. The
two implementations have already diverged in their temporary-name format, and one omits the target
name so concurrent writes to different files in one directory contend on the first attempt. Fold the
receipt writer onto the guarded path or reduce the module to a documented internal helper, and derive
the temporary-name format from one named constant.

### 6.3 Make the scratch root fail instead of degrade (C24)

The scratch root canonicalizes the repository root but falls back to the raw path when
canonicalization fails. Because the mutation lock key derives from that value, the fallback can
silently place the lock elsewhere and drop cross-process mutual exclusion while operations continue
to report success. Startup resolution (C02) makes this unreachable from the server, but the function
is reachable from other callers and should refuse.

### 6.4 Name the remaining inline literals (C25)

The lock subdirectory name sits directly below two named constants that follow the intended pattern;
the scratch fallback leaf is inline; and the meta action name is a bare literal while every real
action has a name constant.

### 6.5 Remove dead code (C26)

The mutation lock guard captures an identity field that is never compared after acquisition, and the
standalone atomic write wraps its body in a closure that adds no cleanup and returns immediately.

## Phase 7 — Operator configuration

**Not product code.** These are host and toolchain configuration items and must not be mixed into
commits that change the crates. They are listed here because several produce errors that look like
tool defects.

### 7.1 Make each entry's identity match its root (C27)

The unqualified entry resolves to a different repository than its name suggests, which is the entry
an agent reaches for first. Another entry is named for a specific task but rooted at a workspace
directory. Rename entries so identity and root agree.

### 7.2 Handle workspace-rooted entries explicitly (C28)

A workspace root that is not itself a worktree makes every Git tool fail until a `repository`
selector is supplied. Either point the entry at the intended worktree or document that the selector
is required on every call.

### 7.3 Restore or remove the unresponsive entry (C29)

One configured entry did not answer a minimal readiness probe within four minutes. An entry that
cannot answer a health probe should be repaired or removed rather than left to time out.

### 7.4 Reject a repeated root argument (C30)

The tool-surface argument refuses a second occurrence; the repository-root argument silently takes
the last value. Add the same guard.

### 7.5 Extend the toolchain allowlist and validation profile (C31, C32)

The Rust validation profile runs only the workspace test command, although linting is already
permitted for guarded execution. Add linting and a formatting check to the profile.

Separately, **the formatting command is not currently allowlisted**, so the required gate below
cannot be run in full through the guarded surface. Add it to the permitted subcommands, or the
formatting check must be run outside this tooling and the gate is only partially enforceable.

## Guardrails

- **Never combine a behavior change with a module move.** If a move reveals a needed behavior change,
  land the move first with identical behavior, then change behavior in its own commit.
- Preserve refusal text, confirmation literals, dry-run defaults, and JSON response shapes unless a
  phase explicitly scopes the change, and update the corresponding test in the same commit.
- One concern per commit in Phase 2, one item per commit in Phase 6.
- No new inline literals. A value used twice becomes a named constant, and a value quoted in a
  description is formatted from that constant.
- Never widen a guard to make a refactor easier. If a guard blocks a change, the change is wrong or
  the guard needs its own reviewed commit.
- Keep filesystem, Git, process, setup, and native policy in `core`. Server modules adapt JSON
  requests to typed core calls.
- Keep the relevant Markdown synchronized in the same commit, per the documentation contract.

## Validation

Run the full gate after every phase:

```sh
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The formatting command is currently outside the guarded allowlist (C32); until that is resolved it
must be run directly, and the gate is only partially enforceable through this tooling.

Phase-specific checks:

- After Phase 1.2, confirm two builds from different dirty trees at the same commit report different
  stamps, and that a clean tree reports the clean marker.
- After Phase 1.3, confirm the reported stamp matches the tree under test. Until then C05 stays
  unverified and no claim should be made about the installed binary.
- After any write-path change, verify mode preservation directly: create a file, set it executable,
  replace its content, and confirm the executable bit survives. No existing test covers this,
  because the repository contains no executable tracked files.
- After Phase 3.3, confirm only one repository-root helper remains.
- After Phase 4, confirm the drift tests fail when a confirmation phrase or capability claim is
  changed in only one location.

## Concern index

| ID | Concern | Phase | State |
| --- | --- | --- | --- |
| C01 | Replacement discarded the target's permission bits | 1 | Complete |
| C02 | Configured root was never validated at startup | 1 | Complete |
| C03 | Critical modules and the server test tree were untracked | 1.1 | Complete |
| C04 | Build stamp could not be pinned to a dirty tree | 1.2 | Complete |
| C05 | Installed binary predates the landed fixes | 1.3 | Unverified |
| C06 | Multiple hunks in one file are refused by the batch tool | 2.1 | Complete |
| C07 | Mutating responses omit the resulting digest | 2.2 | Complete |
| C08 | Batch validation refusals leave no receipt | 2.3 | Complete |
| C09 | Cheap discovery projections are undocumented; meta action unlisted | 2.4 | Complete |
| C10 | Git policy lives in the server crate against the stated boundary | 3.2 | Open |
| C11 | Three helpers share one name with two semantics | 3.3 | Open |
| C12 | Worktree-root requirement is inconsistent across tools | 3.1 | Complete |
| C13 | Workspace selector validates by path, not by descriptor | 3.4 | Open |
| C14 | Confirmation phrases duplicated between handler and schema | 4.1 | Open |
| C15 | Capability contract is hand-maintained inline JSON | 4.2 | Open |
| C16 | Enforcement prose duplicated across five files | 4.3 | Open |
| C17 | No test detects contract drift | 4.4 | Open |
| C18 | Four competing idioms for expressing variance | 5.0 | Blocked |
| C19 | Seven mergeable tool groups | 5.1 | Blocked |
| C20 | Two tools are under-specified for their action space | 5.2 | Blocked |
| C21 | Handler modules are unbalanced | 5.3 | Gated on Phase 4 |
| C22 | Neither write path flushes the parent directory | 6.1 | Open |
| C23 | Two divergent write implementations and name formats | 6.2 | Open |
| C24 | Lock key derivation degrades silently | 6.3 | Open |
| C25 | Remaining inline literals beside named constants | 6.4 | Open |
| C26 | Dead lock identity field and redundant closure | 6.5 | Open |
| C27 | Entry identities do not match their roots | 7.1 | Open |
| C28 | Workspace-rooted entry needs a selector on every call | 7.2 | Open |
| C29 | One configured entry is unresponsive | 7.3 | Open |
| C30 | Repeated root argument is silently accepted | 7.4 | Open |
| C31 | Validation profile omits linting | 7.5 | Open |
| C32 | Formatting command is not allowlisted, so the gate is partial | 7.5 | Open |
| C33 | Surface consolidation has no versioned design proposal | 5.0 | Open |
