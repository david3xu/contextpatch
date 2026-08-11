# Tool Registry Plan

**Status: executed.** All four phases landed on `guarded-shell-script-list` between `b89dda2` and `ab6223c`. 489 tests passing, clippy clean under `-D warnings`, fmt clean. Both snapshots are byte-identical to the versions captured before the first migration, which is the evidence that fifty-four tools changed how every one of their facts is produced without the advertised surface moving once.

Three deviations from the plan as written, each recorded in the commit that made it:

- `project_execute` was excluded from the registry. It is the surface wrapper rather than an internal action: resolved before the repository is determined, advertised only on the project surface, classified as the widest reach of everything it dispatches.
- `EXPECTED_OPEN_WORLD_ACTIONS` was **not** derived from `reach`. Annotations are already computed from `reach`, so deriving the expectation would assert a tautology and discard the audit. It belongs in the guard category with the safety-contract prose.
- The registry scaffolding migrated one tool rather than landing empty. An empty table compiles and proves nothing about whether the schema merge, handler adaptation, deadline lookup, lock lookup, and both classifiers resolve through a descriptor.

A fourth staleness defect surfaced during the inventory and was fixed in `cde66bf`: `guidance::permitted_summary` told a refused `bash` caller that only the base-image script was permitted, three commits after the fixed list replaced it.

Collapsing per-tool fan-out in the MCP server.

Adding one tool currently touches 17 files and asserts that tool's identity independently in 8 places, with nothing checking those places agree. Three defects have already reached committed code through that gap. This plan closes it, and proves the closure by differential test rather than by inspection.

Inventories extracted from `guarded-shell-script-list` at `7de5b1d`: 55 registered tools, 482 tests passing, clippy and fmt clean.

## Why this, and why now

Three defects reached committed code on this branch. All three are the same class: a per-tool fact recorded by hand somewhere nobody thought to update.

| Site | Defect | Status |
| --- | --- | --- |
| `capability.rs` `typed_workflows` | Omitted `compose_stack_run` and `artifact_build_check_run` entirely | Fixed in `7de5b1d` |
| `capability.rs` `programs.bash` | Advertised only the base-image script, three commits after the fixed validation-script list replaced it | Fixed; now derived from the allowlist |
| `protocol/instructions.rs` | Client instructions name three `log_id` tools; there are five | Open |

The manifest is the worst possible landing place for this drift. `capability_manifest` exists so a client can tell a missing capability from a stale binary, so a tool it never mentions reads as a tool that does not exist. The agent report in `docs/gaps.md` was about exactly that failure mode, and its prescribed remedy — call `capability_manifest` before asserting a limit — would have answered wrongly here.

File size is not the cause, and splitting files is not the cure.

Tool count has plateaued: 53 to 55 over six days, against 24 added in one week in late July. There is no growth emergency. The case for doing this now is the defect rate, not the tool count.

## Coverage foundation

This plan is built from mechanical extraction, not from reading. Two inventories are the specification for everything below, and regenerating them is how each phase is verified.

### Inventory A — every tool against every classification axis

All 55 registered tools with the six per-tool facts currently asserted across six files. This table is the registry's target content: if the post-refactor extraction differs by one cell, something was lost.

| Tool | Handler | Schema | Deadline | Lock | Reach | Read-only |
| --- | --- | --- | --- | --- | --- | --- |
| `artifact_build_check_run` | process | process | none | — | exec-code | — |
| `artifact_delete_exact` | files | files | write | Y | local | — |
| `artifact_python_run` | process | process | none | — | exec-code | — |
| `artifact_write_base64` | files | files | write | Y | local | — |
| `artifact_write_text` | files | files | write | Y | local | — |
| `base_image_check_run` | fixtures | fixtures | none | — | exec-code | — |
| `bulk_replace_exact` | files | files | write | Y | local | — |
| `bulk_write_new_files_base64` | files | files | write | Y | local | — |
| `capability_manifest` | capability | capability | read | — | local | Y |
| `compose_stack_run` | process | process | none | — | exec-code | — |
| `create_directory` | files | files | write | Y | local | — |
| `delete_generated_prefix` | git/names | git | git | Y | local | — |
| `delete_guarded` | git/names | git | git | Y | local | — |
| `delete_untracked_exact` | git/names | git | git | Y | local | — |
| `diff_preview` | files | files | read | — | local | Y |
| `docker_image_inspect` | process | process | none | — | local | — |
| `file_info` | files | files | read | — | local | Y |
| `fixture_generator_run` | fixtures | fixtures | none | Y | exec-code | — |
| `fixture_manifest_refresh` | fixtures | fixtures | write | Y | local | — |
| `fixture_manifest_verify` | fixtures | fixtures | read | — | local | Y |
| `git_branch_prepare` | git/names | git | git | Y | remote | — |
| `git_commit_exact` | git/names | git | git | Y | local | — |
| `git_commit_prefix` | git/names | git | git | Y | local | — |
| `git_commit_scoped` | git/names | git | git | Y | local | — |
| `git_merge_readiness` | git/names | git | git | Y | remote | Y |
| `git_push_exact` | git/names | git | git | Y | remote | — |
| `git_remote_check` | git/names | git | git | Y | remote | — |
| `git_remote_list` | git/names | git | git | — | local | Y |
| `git_restore_exact` | git/names | git | git | Y | local | — |
| `git_stage_exact` | git/names | git | git | Y | local | — |
| `git_staged_scope_check` | git/names | git | git | — | local | Y |
| `github_fork_prepare` | github | github | git | Y | remote | — |
| `github_pr_run` | github | github | git | — | remote | — |
| `harbor_run_start` | process | process | none | — | exec-code | — |
| `image_cleanliness_check_run` | process | process | none | — | isolated | — |
| `list_directory` | files | files | read | — | local | Y |
| `move_tracked` | git/names | git | git | Y | local | — |
| `native_build_run` | native | native | none | Y | exec-code | — |
| `native_device_run` | native | native | none | Y | exec-code | — |
| `preflight_health` | capability | capability | read | — | local | Y |
| `project_execute` | project | project | none | — | wrapper (see note) | — |
| `read_command_log` | process | process | read | — | local | Y |
| `read_file_bytes` | files | files | read | — | local | Y |
| `read_range` | files | files | read | — | local | Y |
| `read_write_receipts` | files | files | read | — | local | Y |
| `replace_exact` | files | files | write | Y | local | — |
| `run_guarded_command` | process | process | none | — | exec-code | — |
| `set_file_executable` | files | files | write | Y | local | — |
| `setup_profile_run` | setup | setup | none | Y | exec-code | — |
| `status_guard` | files | files | read | — | local | Y |
| `task_image_python_run` | process | process | none | — | isolated | — |
| `validation_profile_run` | process | process | none | — | exec-code | — |
| `write_existing_file_exact_hash` | files | files | write | Y | local | — |
| `write_new_file` | files | files | write | Y | local | — |
| `write_new_file_base64` | files | files | write | Y | local | — |

Totals — deadline: git 17, none 15, write 12, read 11. Mutation-lock members: 30. Read-only: 14. Every registered tool has a schema entry.

`project_execute` is special-cased to `WrapperDispatch` ahead of the group checks in `authority.rs`, so it appears in none of the three classifier functions.

**Two latent inconsistencies the matrix exposes.** `git_merge_readiness` is simultaneously read-only and a mutation-lock holder. `fixture_generator_run`, `native_build_run`, `native_device_run`, and `setup_profile_run` hold the mutation lock with no reply deadline. Both are probably deliberate — a fetch touches `.git`, and slow mutators should not be deadline-bounded — but neither is stated anywhere. The registry makes each an explicit reviewable field rather than an emergent accident.

### Inventory B — every site that enumerates tool identity

48 files name three or more tools. They are not equivalent, and treating them as one bucket is how a migration goes wrong.

| Disposition | Sites | Action |
| --- | --- | --- |
| Migrate | `schema/*.rs` (11), `dispatch.rs`, `schema/authority.rs`, `tools/mod.rs` | Becomes the registry; source of all six axes |
| Derive | `tests/files.rs`, `tests/project.rs` (×2), `tests/protocol.rs`, `capability.rs`, `README.md` | Counts and lists computed from the registry, not typed |
| Guard | `tool-spec.md`, `safety-contract.md`, `execution-threat-model.md`, `claude-desktop.md`, `copilot-instructions.md`, `protocol/instructions.rs`, `core/process/guidance.rs` | Prose stays hand-written; add existence tests |
| Leave | `hardening-plan.md`, `implementation-roadmap.md`, `project-tool-surface-plan.md`, `native-background-implementation.md`, `gaps.md` | Historical records; rewriting destroys evidence |

## Phase 1 — Close the open defects

Done first and separately, so the registry migration is a pure refactor with no behavioural change mixed in.

- Update `protocol/instructions.rs` to name all five `log_id` tools, or better, phrase it by category so it cannot fall behind again.
- Add an existence test: every tool name appearing in `protocol/instructions.rs` and `core/process/guidance.rs` must be a registered tool. `guidance.rs` names 28 tools as refusal alternatives and nothing currently checks they exist.

Exit: full suite green; no tool name anywhere in `crates/` that is not registered.

## Phase 2 — Build the registry

One descriptor per tool, carrying every fact currently spread across six files.

```rust
struct ToolDescriptor {
    name: &'static str,
    schema: fn() -> Value,
    handler: fn(EffectiveRepository, &Map<String, Value>) -> Result<String, String>,
    deadline: Option<Deadline>,   // read | write | git | none
    reach: RemoteReach,           // local | remote | isolated | exec-code | wrapper
    read_only: bool,
    serializes_mutation: bool,
}
```

The safety property is that migration is incremental. The registry is added alongside the existing dispatch, not in place of it. Each tool moves individually, and an old `match` arm is deleted only when its descriptor is live. At every commit the suite is green and both paths agree.

Order, easiest and most isolated first so the pattern is settled before it meets the hard cases: `capability` (2) → `setup`/`native`/`project` (4) → `fixtures` (4) → `github` (2) → `git` (15, already well factored) → `files` (18) → `process` (10, most heterogeneous, most informed by then).

Exit: `dispatch.rs` holds a table lookup rather than 58 arms; `deadline_for`, `serializes_repository_mutation`, and the three `authority.rs` classifier functions are gone, their content now fields.

## Phase 3 — Derive what is currently typed

- The three hardcoded counts (`54`, `55`, `55`) become registry-derived. They currently fail as `left: 54, right: 53`, which says nothing about what is wrong.
- `EXPECTED_OPEN_WORLD_ACTIONS` in `tests/stage1_mcp/protocol.rs` becomes a derivation from `reach`, keeping the isolated-versus-networked assertion that already caught a real misclassification.
- The README's 54-bullet action list becomes generated, or is replaced by a pointer to `tools/list`.

Exit: adding a tool requires no test edit at all.

## Phase 4 — Split the two oversized modules

Follow the `git/` precedent, already proven in this codebase: `names.rs` plus `handlers/*.rs` plus `support.rs`. It carries the most tools and has the best organisation.

| Module | Tools | Lines | Files | Split into |
| --- | --- | --- | --- | --- |
| `git/` (reference) | 15 | 2,020 | 8 | already done |
| `process.rs` | 10 | 1,695 | 1 | jobs, logs, guarded, containers |
| `files.rs` | 18 | 1,215 | 1 | read, write, artifacts |

`process.rs` is the least cohesive file in the crate: background-job machinery, the command-log store, guarded execution, and five container tools in one file.

This comes last on purpose. The registry changes what belongs in these files, so splitting first means splitting twice.

## How coverage is proven

The requirement is that nothing is lost. Inspection cannot establish that across 55 tools and 8 axes, so three mechanical checks do it instead.

**Golden snapshot, captured before any change.** Serialize all 54 `tools/list` definitions — names, schemas, annotations, descriptions — to a checked-in fixture. Every phase asserts byte-identical output. A refactor that changes one description or drops one annotation fails immediately and names the tool.

**Matrix regeneration as a test.** The extraction that produced Inventory A becomes a checked-in test comparing the registry against the committed matrix. This is what makes full coverage a property rather than a claim: the six axes cannot silently diverge, because one table is the only source and the test compares it against a frozen expectation.

**Existence closure.** Every tool name mentioned anywhere in `crates/` — dispatch, manifest, guidance, instructions, tests — must resolve to a registered tool, and every registered tool must appear in `tools/list`, `docs/tool-spec.md`, and the capability manifest. Three of these guards exist already; the rest are cheap.

## Scope boundaries

Not in scope. The `core` crate is not touched: its large files are cohesive, roughly 40% inline tests, and show none of this coupling. The 2,576-line `tests/stage1_mcp/project.rs` stays as it is, because size costs little in a test file. Safety-contract and threat-model prose stays hand-written, because it records guarantees rather than names, and generating it would strip the reasoning that makes it worth having.

Risk. The registry touches the dispatch path for every tool, which is the highest-traffic code in the server. That is why migration is per-tool with both paths live, and why the golden snapshot is captured first. The realistic failure is a silently changed schema description, which the snapshot catches by construction.
