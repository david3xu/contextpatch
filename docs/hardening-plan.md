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

## Phase 0 — Azure companion MCP workflow (first implementation)

**This is the first implementation priority before the remaining open phases.** It addresses an
operator need without widening ContextPatch's product boundary: Claude Desktop needs direct,
structured access to Azure resource state, but ContextPatch remains a repository-confined edit and
validation engine. Do not add Azure control-plane APIs, generic `az`, arbitrary `npx`, or cloud
credentials to ContextPatch.

### 0.1 Configure a least-privilege Azure companion server (C35, open)

Repository deliverables are complete: `docs/azure-mcp-runbook.md` and the redacted template at
`docs/examples/claude-desktop-azure-mcp.json`. Host setup and live verification are not, so C35
stays open.

Exact blockers, in the order they must be cleared:

1. **No version is pinned, and the decision is deliberately deferred.** Provenance is now traced and
   recorded in the runbook, and one earlier finding in this plan is retracted. The claim that no
   general-availability release existed was wrong: it came from reading only the archived repository,
   whose versions stopped at `0.5.x`. Versioning continued in `microsoft/mcp` through a 1.0 stable
   release and then a 2.0 stable release. Azure MCP Server 2.0 is generally available as of
   10 April 2026, and `@azure/mcp` `2.0.2` is confirmed published, so a stable line does exist to
   pin. Two gaps still block installation, and the CVE is no longer one of them. `CVE-2026-32211` was
   verified and retracted as a blocker: MSRC records it as an Azure MCP Server information disclosure
   issue, CWE-306, CVSS 9.1 base and 7.9 temporal, released 2 April 2026, already fully mitigated by
   Microsoft with no customer action required, published under the cloud service CVE transparency
   programme. The GitHub advisory `GHSA-5w7p-v6h9-q8c5` lists no package and no affected or fixed
   version in any ecosystem, so `@azure/mcp` is not implicated at any version. The vector
   `AV:N/PR:N/UI:N` with `RL:O` points to a network-reachable service deployment rather than a local
   stdio process, which is not in scope for the planned rollout, though it would become relevant if a
   self-hosted remote deployment were ever considered. The claim that patching was pending came from
   a low-quality source that also misattributed the CVE to the Azure DevOps MCP server. What remains
   is that platform-version skew is unconfirmed for the stable line, since whether
   `@azure/mcp-darwin-arm64` publishes a matching `2.0.2` was not checked, and on Apple Silicon that
   package supplies the binary. The exact newest `Azure.Mcp.Server-` tag, its commit SHA, and the
   npm publish workflow are still unrecorded, because GitHub blocks automated access to the raw
   changelog and rejects the query-string release URL, so a browser is needed. The Microsoft Artifact
   Registry image is still not adopted; the standing condition that Microsoft identify a distribution
   as current and that its digest be pinnable is unchanged.
2. **Namespace tokens are unknown for any specific build.** The filtering table in the runbook is
   deliberately unfilled rather than guessed, and must be captured from the pinned binary's own
   help output.
3. **Host actions are outside ContextPatch's authority and remain operator-owned.** Installing the
   package, creating the service principal, assigning roles, signing in through the isolated
   `AZURE_CONFIG_DIR` profile, and editing Claude Desktop configuration cannot be performed from
   here. `npm install`, `npx`, and `az` are refused by policy, and writes outside the repository
   root are refused; those refusals are now pinned by test and must stay that way.
4. **None of the eight live smoke checks has run,** because no Azure MCP surface is connected.

One alternative should be evaluated before anything is installed. `microsoft/mcp` also catalogues
Azure Resource Manager MCP at `Azure/Azure-Resource-Manager-MCP`, a remote server at
`mcp.management.azure.com` offering Resource Graph queries and ARM deployment tooling. That covers
resource inspection, Resource Graph, and deployment inspection with no local install, no version to
pin, and no platform binary skew, though not Cost Management or the Speech tier, and its deployment
management tools mutate and would need excluding.

C35 is marked complete only when all eight checks in the runbook pass.

The current workflow requires Azure state to be copied from a terminal into the agent conversation.
The first implementation should configure Microsoft's Azure MCP Server as a **separate** Claude
Desktop entry, rather than teaching ContextPatch to call Azure. Evaluate the official
`@azure/mcp` server first; add a separate Resource Graph or ARM deployment server only if a required
workflow is not available through that reviewed surface.

The capability and ownership boundary must be explicit:

| Need | ContextPatch | Azure companion MCP | Operator or workload repository |
| --- | --- | --- | --- |
| Repository edits, Bicep source changes, and local validation | Yes, through existing guarded file, Git, and command policies | No | Review and approve mutations |
| Guarded command execution | Only the supported forms of `git`, `cargo`, `bun`, `npm`, `pnpm`, `python3`, `pytest`, `rg`, and the exact approved shell check | No | Supply confirmations where required |
| Azure resource inventory, state, Resource Graph, cost evidence, and service-tier inspection | No | Read-only first implementation | Establish and revoke the scoped identity |
| `az bicep build`, ARM deployment, Speech SKU mutation, and other control-plane writes | No | Disabled in the first implementation | Run explicitly and retain raw output |
| Application sequencing such as handler migration before Redis adoption | Repository editing only | Inspect deployed state after it exists | Owned by the workload plan |

The detailed 27-item human/agent queue remains in the workload repositories rather than being copied
into this product hardening plan. Keep each workload item marked **H** (human/operator) or **A**
(agent), and preserve these corrected dependencies:

- migrate the Azure-calling handlers before adopting Redis, because nothing deployed currently
  consumes Redis;
- treat the App Service S1 and total-cost figures as live workload-plan estimates, not spare budget;
  the current plan records roughly $70 for App Service, $296 total, and a $400 ceiling while three
  services remain usage-billed;
- keep `resend` wiring and production PostHog verification as workload-repository audits that the
  agent can perform without Azure mutation authority.

The initial integration must cover these user needs:

- list resources and inspect current resource state without copy-paste;
- query Cost Management evidence used to verify the daily spending floor;
- confirm service configuration such as the Speech pricing tier;
- query and filter subscription resources through Resource Graph where useful;
- inspect ARM deployment state and evidence;
- keep repository-local Bicep edits under ContextPatch's existing guards.

The first rollout is read and verification oriented. Initial deployment remains an explicit operator
workflow (`az bicep build` followed by the reviewed deployment command) until mutation scope,
approval behavior, and evidence retention have their own design. Do not use a persistent
`npx -y ...@latest` command in Claude Desktop configuration: review a release, pin the tested
version, and record how it is upgraded and rolled back.

Authentication is a separate trust boundary. Do not attach the operator's subscription-wide Owner
identity to the server. Use a dedicated Entra identity with the narrowest useful scope:

- Reader or the corresponding read-only roles for resource inspection;
- Cost Management Reader only at the scope required for cost evidence;
- Contributor at the target resource group only if a later, separately approved mutation workflow
  genuinely needs it;
- no role-assignment permission and no credential material committed to this repository or embedded
  in Claude Desktop configuration.

Acceptance:

1. A documented Claude Desktop entry starts the pinned Azure MCP server and authenticates without
   interactive secrets in configuration.
2. The agent can list and inspect only the approved Azure scope, query cost evidence, and verify the
   Speech tier.
3. An out-of-scope resource query or mutation is denied by Azure authorization, not merely by prompt
   instructions.
4. ContextPatch's schemas, capability manifest, dependency graph, and command allowlist gain no Azure
   authority.
5. The runbook covers sign-in, credential rotation/revocation, version pinning, smoke checks,
   rollback, and removal.

Use the current Azure incident queue as the first end-to-end pilot, in this order:

1. The operator throttles Foundry immediately; the observed daily rate exceeds the intended monthly
   design cost.
2. Cost Management evidence confirms that the rate dropped. Until C35 is operational this remains a
   human copy-paste step; afterward it is the key read-only MCP acceptance case.
3. The operator determines whether the recorded high-cost event was a runaway loop and records the
   answer in the workload plan.
4. The operator runs `az bicep build`, preserves the raw output, and reviews the uncompiled modules,
   especially the `Microsoft.Cdn` and `Microsoft.ApiManagement` API versions that were selected from
   memory.

### 0.2 Correct the command-allowlist threat model (C36, complete)

The audit is recorded in `docs/execution-threat-model.md`, and the corrected claims are reconciled
across `README.md`, `docs/safety-contract.md`, `docs/architecture.md`, `docs/tool-spec.md`,
`docs/azure-workload-position.md`, and the `run_guarded_command` schema description.

What the audit changed, beyond wording:

- `openWorldHint` was a blanket `false` for every advertised action, which was a false public
  capability claim. It is now derived per action from one `RemoteReach` classification in
  `crates/server/src/tools/schema/authority.rs`, built from the existing action-name constants.
  Sixteen actions are open-world; `task_image_python_run` and `image_cleanliness_check_run` stay
  closed-world because they run with networking disabled. A protocol test pins the inventory.
- `git_merge_readiness` is both read-only and open-world, because it may fetch to report state.
  That was previously invisible.
- `pytest` was gated by `=> true`, so its arguments were entirely unrestricted. It now refuses the
  plugin-loading option `-p` in both short forms, and pytest children receive
  `PYTEST_DISABLE_PLUGIN_AUTOLOAD=1` with `PYTEST_ADDOPTS` and `PYTEST_PLUGINS` removed. The
  environment is not otherwise cleared, because supported builds need inherited toolchain
  variables. Repository-local `conftest.py` collection is deliberately retained and tested.
- Argument path confinement was found to be stronger than assumed: absolute paths and `..`
  traversal are refused before spawn, in `validate_common_command_shape`. That claim survived the
  audit intact and is now pinned by test rather than assumed.

Refusal tests added for `az`, `npx`, npm/pnpm/bun installation forms, arbitrary `python -m` and
`-c`, generic shell programs and shell strings, pytest plugin-loading options, pytest paths outside
the repository, and a program supplied as a path. Every prohibition listed in item 2 below is now
covered by a test rather than by prose alone.

The original requirements, retained for the record:

The executable name allowlist is not a sandbox. Repository-relative Python programs and reviewed
`bun`, `npm`, or `pnpm` scripts can execute general code with the server process's operating-system
permissions. The allowlist narrows entry points and arguments; it does not by itself prove
hermeticity, network isolation, or harmlessness.

Before the Azure companion is presented as a safe contrast with ContextPatch:

1. Inventory the exact authority of every allowed command family, including environment, network,
   filesystem, subprocess, lifecycle-script, and credential access.
2. Keep package installation, arbitrary `npx`, generic `az`, arbitrary `python -m`, shell strings,
   and unreviewed scripts outside `run_guarded_command`.
3. Reconcile the README, capability manifest, safety contract, and Azure-position document so none
   claims that executable-name allowlisting alone prevents arbitrary code or outbound access.
4. State which guarantees actually come from repository-relative selection, retained root
   authority, null stdin, bounded arguments and output, timeouts, process-group termination,
   confirmation gates, and explicit command-family policy.
5. Add drift tests for the refused command forms and for every public claim that depends on process
   isolation.

This audit must not turn ContextPatch into the Azure transport. Its purpose is to make both trust
boundaries explicit: ContextPatch executes narrowly selected local tooling, while the companion
Azure MCP server independently holds scoped cloud authority.

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

C11 is closed by 3.3: the canonicalize-only duplicates are gone and one resolver remains.

Decide explicitly, per tool, whether an exact worktree root is required, and state it in the schema.
Today the same configured root can be accepted by the status path and refused by the mutation path.

The path-taking tools already fail closed, because path normalization rejects parent components,
absolute paths, and pathspec metacharacters. The tools that take no paths are the exposure:
`git_branch_prepare` and `git_push_exact` act on the enclosing repository, so branch switching and
pushing are not bounded by the configured root. Write this decision down before moving any code, so
the migration has a specification to preserve.

### 3.2 Migrate Git policy into `core` (C10) — complete

The migration pattern held across all five slices: `core` returns the refusal detail only and the server
adapter supplies the `"<tool> refused: "` prefix, so every public refusal string stayed byte-identical
while policy relocated. Server-side regressions pin those assembled strings in each slice, which makes a
detail reworded in `core` a test failure rather than a silent public change.

Three refusal prefixes are not the usual shape and are preserved as data returned to the adapter rather
than strings built in `core`: `refused after staging`, `refused after commit`, and the restore and delete
postconditions. Two more were never tool names at all, `git operation` and `git workflow`.

Slices, each its own behavior-neutral commit:

1. **Request validation — done.** `core::git::validate` owns path confinement, pathspec rejection, prefix
   normalization, prefix boundary matching, and commit subject/body limits. The server functions are thin
   adapters. This slice covers the guard that bounds every path-taking action, which is the same reason
   those actions do not need an exact worktree root under 3.1.
2. **Git-executing state guards — done.** `core::git::state` owns Git invocation and everything derived
   from what Git reports: the no-shell argv, working directory, shared subprocess timeout, timeout and
   truncation refusals, exit-code handling, porcelain and NUL path parsing, status/untracked/ignored/
   cached path queries, head with its unborn-repository case, current branch, local branch existence,
   revision counts, remote existence, and ref resolution. The bounded-output refusal is the reason this
   is policy rather than plumbing: a truncated `git status` reads as a shorter list of dirty paths, and a
   tool acting on it would act on a set it never saw.

   Fifteen core tests exercise these against real temporary repositories, covering the unborn head, a
   detached head, modified versus untracked versus ignored classification, staged-only reporting, branch
   presence and absence, and the rename entry that is refused rather than half-interpreted. The
   truncation test moved with its subject. A server regression pins the assembled prefixes, including the
   two that were never tool names, `git operation` and `git workflow`, which are the easiest to lose in a
   move.

   Removing the inlined implementations left several server pass-throughs with no callers; they were
   deleted rather than kept alive behind an allow attribute.
3. **`restore` planning — done.** `core::git::restore` owns planning, guarded execution, and
   postcondition checks for the three restore and delete actions: the dirty and tracked requirements for
   a restore, the untracked and regular-file requirements for an exact delete, prefix expansion over
   untracked and ignored candidates, empty-directory collection, the expansion bound, and the deletion
   itself. Plan and apply are separate functions so the adapter still holds its mutation lock and opens
   its receipt around the mutation alone.

   Two postcondition refusals are addressed as `refused after restore` and `refused after delete` rather
   than the usual shape, so the offending set is returned as data and worded by the adapter. Limits,
   confirmations, and dry-run defaults stay ahead of normalization in the adapter, which keeps unchanged
   the order in which a caller learns a request is malformed.

   Fourteen core tests run against real temporary repositories. Two of them record findings rather than
   assumptions: porcelain reports the untracked *file* rather than its enclosing directory, so a prefix
   over a mixed directory deletes the generated sibling and leaves tracked history untouched; and Git only
   collapses a directory in an ignored listing when it holds nothing tracked, which means the
   tracked-path refusal inside that walk guards a race rather than a routine case. That is now stated at
   the function rather than covered by a test that cannot reach it.
4. **`commit` planning — done.** `core::git::commit` owns the three selection shapes and their differing
   claims. `exact` asserts the caller knows the entire dirty set, so an inexact list is a stale view and is
   refused rather than narrowed, and it needs no clean-index gate because the exact set already accounts
   for everything dirty. `scoped` and `prefix` claim only a subset, so they additionally require a clean
   index to prove no unrelated staged work is swept in. All three verify after staging that the index holds
   exactly what was planned, and `exact` additionally re-checks that the dirty set has not moved. Staging,
   committing, and verification are separate functions because each refusal is addressed differently, as
   `refused`, `refused after staging`, or `refused after commit`, and because the adapter opens its Git
   receipt around the mutation.

   Thirteen core tests cover the shapes against real temporary repositories, including that an exact commit
   tolerates a pre-staged index while a partial commit does not, that a scoped commit leaves unrelated
   dirty paths behind, that exactly one commit is produced, that a staging request produces none, and that
   planning stages and commits nothing. Server regressions pin the assembled prefixes and the adapter-side
   limits, confirmations, and message validation ordering.

   One pre-existing defect was preserved rather than fixed: in the exact-set mismatch refusal the
   `expected_paths` and `actual_dirty_paths` labels are swapped relative to the sets they print. Correcting
   them would change public refusal text, which this behavior-neutral migration must not do. It is now
   recorded as C34 and marked at the code.
5. **`sync` planning — done.** `core::git::sync` owns remote listing and parsing, the single-branch fetch
   refspec, the fetch-did-not-disturb-the-worktree guard, clean-worktree gates, ancestry, divergence
   counts in both directions, changed-file overlap between two refs, expected-head validation, the
   required-file gates, and the branch action selection.

   Branch action selection is the one place this slice improved rather than only relocated. The dry-run and
   execution paths previously derived the command sequence separately, so they could drift; they now share
   one derivation, with the two reported name sets preserved because a plan says what it *would* do while a
   result says what it *did*. The reset case still distinguishes being already on the branch, since forcing
   a ref while it is checked out would leave the worktree behind.

   Fourteen core tests run against a bare remote with a clone wired to it. Bare on purpose: pushing to a
   non-bare repository's checked-out branch is refused by Git itself, which would mask the guards under
   test. Two server regressions pin the assembled prefixes and the confirmations that still precede any
   Git call. All five sync integration tests pass unchanged.

With this slice C10 is complete. `tools/git/support.rs` is down from roughly twenty-six thousand bytes to
twelve, and what remains there is request shaping and refusal addressing rather than policy. Eleven more
pass-throughs lost their callers during this slice and were deleted.

JSON parsing, MCP schemas, receipts, and response formatting stay in `server` throughout. C11 helper
deduplication and C13 descriptor-based selection remain out of scope for this phase.

The refactor plan's guardrail keeps filesystem, Git, process, setup, and native policy in `core`,
with server modules adapting JSON to typed core calls. Tracked move and guarded delete follow that
rule; the commit, restore, and sync handlers carry real policy. Move that policy into `core::git` and
leave request shaping and response formatting in `server`. This is a module move and must carry no
behavior change.

### 3.3 Consolidate the repository-root helpers (C11) — complete

Three functions previously shared one name and two semantics: a strong version backing three call sites,
and canonicalize-only copies in `core::git::status` and in the server layer backing roughly twenty,
including every commit, stage, restore, remote, branch, and push workflow.

There is now one canonicalization, `core::git::resolve_repo_root`. Both duplicates are gone.
`exact_worktree_root` is deliberately kept separate and now *builds on* the shared resolver, so the two
cannot drift on **how** a root is resolved while differing only in what each additionally demands. That is
the property the duplication was hiding: the weak copies were not merely redundant, they were free to
diverge from the strong one on resolution itself.

The consolidation is behavior-neutral because the shared resolver returns the raw I/O error rather than a
worded refusal. The two surfaces above it word failure differently and always have: `core` names the path,
the MCP adapter reports a shorter phrase and its tool prefix. Sharing resolution without sharing wording is
what let this land without changing a single refusal string. `core::git::status` keeps its phrasing in a
two-line wrapper; the server keeps its own in `resolved_repo_root`, which adds only the prefix.

Core tests cover the resolver directly, including that a symlinked root resolves to its target, that a
subdirectory of a worktree resolves successfully but is refused by the strict resolver, and that the strict
resolver reports the shared resolution failure. A server regression pins the MCP wording and asserts it is
*not* core's, which is precisely the drift a shared resolver could otherwise introduce.

C13 remains separate and untouched.

### 3.4 Rebuild the workspace selector on descriptors (C13) — in progress

**Slice 1 — done.** The selector walk is descriptor-relative. Each component is opened with `openat` from
the descriptor the previous step returned, using `O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`, so the parent
chain cannot be substituted mid-walk and no full path is ever handed to the kernel after the first open.
Because `ELOOP` and `ENOTDIR` do not portably distinguish a symlink from a plain file, a failed open is
classified with `fstatat` against the same parent descriptor, which keeps all four existing refusal strings
exact.

The walk now returns a typed `SelectedRepository` that retains the final directory descriptor rather than
discarding it. Dispatch binds that selection for the whole call, so the validated directory stays open
while the tool runs, and `revalidate` proves the resolved path still names the anchored directory
immediately before the path is handed to `execute_tool`. Selection also revalidates once at construction,
so a swap during canonicalization or the worktree check is caught before the selection is handed out.
Non-Unix fails closed: without descriptor-relative opens the walk cannot be made safe, so the selector is
refused outright rather than silently falling back to paths.

Three adversarial tests cover the window: renaming the validated directory aside and moving a different
repository into its place, replacing it with a symlink pointing outside the workspace, and unlinking it
altogether. Each asserts the refusal and that neither the replacement nor the outside directory was
touched.

**Slice 2 — done.** Path confinement and the filesystem primitives beneath the Git handlers are now
descriptor-relative. An opaque `RepositoryRoot` carries a logical path plus path-or-descriptor authority,
keeping the descriptor private so a caller cannot reach past the authority and use the name whenever the
descriptor is inconvenient. `GitRepository` is its Git projection; `crate::fs::rooted` is its filesystem
projection.

Confinement walks components from the retained descriptor and inspects the final component with `fstatat`
rather than opening it, because a path that does not exist yet is a legitimate create target. The
filesystem primitives — type and metadata checks, no-follow regular-file opening and hashing, required-file
validation, rename, unlink, recursive removal, and directory listing — all take the root and ask it how to
reach the entry.

One behavior difference is deliberate and asserted in both directions. A path-backed root resolves by name
and therefore follows an in-tree symlink; an anchored root refuses a symlink at any component, because
following one would mean resolving a name again. Anchored roots are stricter, matching the selector walk
and the guarded file path. The path-backed branch is retained rather than unified away so configured roots
and existing callers keep their exact refusals.

**Slice 3 — done.** All five mixed filesystem/Git handlers are migrated end to end: `git_branch_prepare`,
`delete_untracked_exact`, `delete_generated_prefix`, `move_tracked`, and `delete_guarded`. Each derives its
Git subprocess from the `GitRepository` projection and every filesystem check and mutation from the
descriptor-relative primitives, both taken from the same root authority, so no operation is split between
anchored Git and logical-path filesystem access. The exact-worktree-root requirement is applied without
re-resolving an anchored selection, which was already verified as a worktree root when it was selected.

Project-selection tests exercise dispatch rather than core helpers: tracked moves, guarded deletions, exact
untracked cleanup, generated-prefix cleanup, and branch required-file checks each act on the selected
repository only, with siblings holding identically named files and identically named ignored trees, and
with an outside repository asserted bit-for-bit unchanged. Each pair of assertions runs in both directions
so none can pass by accident of ordering.

**Slice 4 — done. Uniform no-follow authority.** The two authorities no longer differ. An anchored root
lends the descriptor it retains; a path-backed root has one opened for it with `O_DIRECTORY | O_NOFOLLOW`,
and that single open is the only name resolution. Everything after it is descriptor-relative under either
authority, so a configured root is exactly as safe as a selected one.

This supersedes the deliberate asymmetry recorded in slice 2, and the change is user-visible rather than
internal. A configured root no longer accepts an in-tree symlink that it previously followed and allowed,
and a configured root whose own *name* has become a symlink is refused rather than followed to whatever
replaced it. That second case previously succeeded and returned the replacement's content. The tests that
asserted the old asymmetry are replaced by tests asserting the new uniformity in both directions.

Inspection is deliberately not folded in with access. `fstatat` with `AT_SYMLINK_NOFOLLOW` still reports a
symlink as a symlink, so `file_info` and `list_directory` keep describing what is present. What no longer
happens anywhere is *following* one.

**Slice 5 — done. The repository-file surface.** Every repository-rooted core primitive takes
`RepositoryRoot`: reads, ranges, metadata, directory listing, replacement, diff preview, text and base64
creation, exact-hash overwrite, bulk planning and apply, executable mode, and directory creation.
`guarded_file` walks from a descriptor supplied by `crate::fs::rooted` rather than opening the root by name,
and `GuardedRegularFile` retains its own root descriptor so revalidation re-walks from the authority that
opened the file. `create_directory` inspects with `fstatat`, creates with `mkdirat`, and descends with
`openat`. `fs/path.rs` was deleted rather than migrated: it had no callers and implemented the
canonicalize-and-compare form this phase removes.

Fifteen actions now receive `EffectiveRepository::root()` from dispatch: `read_range`,
`read_write_receipts`, `diff_preview`, `replace_exact`, `bulk_replace_exact`, `status_guard`, `file_info`,
`list_directory`, `read_file_bytes`, `set_file_executable`, `write_new_file`, `write_new_file_base64`,
`write_existing_file_exact_hash`, `bulk_write_new_files_base64`, and `create_directory`. Every non-artifact
`resolved_repo_root` call is gone, because resolving a name to decide where to work is precisely what an
anchored selection must not do.

The boundaries that legitimately need a name ask for one through `canonical_label`, which resolves a
configured root once and returns an anchored selection's logical path untouched. Locks, receipts, and
scratch identity use it; nothing else does.

Five grouped dispatch tests cover the surface, each running in both directions: reads and listing, writes
and replacement and mode and hash-gated overwrite, bulk atomicity including a validation refusal that
leaves every target unchanged, symlink refusals at intermediate and leaf components across five action
families, and a replaced root that is refused rather than followed. Siblings carry identically named files
and outside repositories are asserted unchanged.

**Remaining unanchored uses.** C13 stays open only for the boundaries listed here, each deferred
deliberately rather than pending:

1. **Repository and file mutation locks**, keyed by the canonical path label. Lock identity must be stable
   across aliases and across processes, which a descriptor cannot provide.
2. **Receipt journal and scratch identity**, derived from the canonical path label for the same reason.
3. **Sidecar artifact operations** — `artifact_write_text`, `artifact_write_base64`,
   `artifact_delete_exact`, and `artifact_python_run` — which resolve a sidecar root rather than the
   repository root and keep `resolved_repo_root`.
4. **Process, fixture, GitHub, capability, and native handlers**, none of which take root authority.
5. **`exact_worktree_root` for path-backed roots**, which canonicalizes and compares by name. Correct for a
   configured root reached only by name, and skipped entirely for an anchored selection.
6. **The Git status scope pathspec**, normalized against the canonical label. It is a pathspec handed to
   Git rather than a path used for access, but it is recorded here rather than left implicit.

Across the migrated surface, `logical_path()` is read only through `canonical_label`, and only for the
label boundaries above. No handler reads it to gain access to a file.

The selector is otherwise sound: it rejects parent and current-directory components, absolute paths,
trailing and doubled separators, backslashes, control characters, drive prefixes, and the Git
administrative directory; requires the re-joined path to equal the input; rejects a symlink at every
component; and then requires an exact worktree root. It cannot escape the workspace boundary.

That result is now reached by descriptor rather than by name: the selector walks with descriptor-relative
opens and hands the resulting directory descriptor forward, so within the Git surface the checked object
and the used object are the same object. The boundaries listed above are where that is not yet true.

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
| C10 | Git policy lives in the server crate against the stated boundary | 3.2 | Complete |
| C11 | Three helpers share one name with two semantics | 3.3 | Complete |
| C12 | Worktree-root requirement is inconsistent across tools | 3.1 | Complete |
| C13 | Workspace selector validates by path, not by descriptor | 3.4 | In progress |
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
| C34 | Exact-commit mismatch refusal swaps its `expected_paths` and `actual_dirty_paths` labels | unscheduled | Open |
| C35 | Azure verification requires copy-paste and lacks a scoped companion MCP workflow | 0.1 | Open — repository deliverables complete, host setup and live checks outstanding |
| C36 | The command allowlist is described more strongly than its execution authority supports | 0.2 | Complete |
