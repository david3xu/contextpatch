# Safety Contract

`contextpatch` exists to make agent edits small, anchored, atomic, and reviewable.

The product position is that AI coding agents should receive guarded edit primitives, not broad filesystem write power. This contract protects that position.

This document is normative. If implementation behavior conflicts with this file, the implementation is wrong unless this file is intentionally updated in the same change.

## Write rules

1. Do not overwrite whole files by default.
2. Require exact anchors, hashes, patch context, or destination-absence checks for persistent writes.
3. Refuse ambiguous replacements.
4. Write through a temporary file and atomic rename where supported.
5. Expose Git state before guarded edits.
6. Prefer previewable diffs over hidden mutation.
7. Never provide an unrestricted shell or recursive delete primitive.
8. If validation command execution is exposed, it must be no-shell, repo-root-confined, allowlisted, timeout-bound, and auditable.
9. If local Git commit support is exposed, it must be an explicitly confirmed exact-path checkpoint, not broad Git authority.
10. If remote Git support is exposed, it must be split into explicit remote-check, branch-preparation, merge-readiness, and exact-push tools, not added to generic command execution.
11. If setup support is exposed, it must be declarative and profile-owned: callers choose a profile/action with typed params, never raw package-manager or shell commands.
12. If native build or device support is exposed, it must be declarative and action-owned: callers choose typed build/device actions, never raw `xcodebuild`, Gradle, `xcrun`, or `adb` commands.

## Required refusal cases

Write tools must refuse the operation when:

1. The target path is outside the configured repository root.
2. The target path points to a directory when a file is required.
3. The expected anchor or old text is missing.
4. The expected anchor or old text appears more than once.
5. The destination already exists for create-only writes.
6. A delete request lacks the expected file hash or equivalent confirmation.
7. Repository status violates the requested guard policy.

Refusals must return a clear reason. They must not pretend success.

## Atomic write expectation

Persistent file writes should use this pattern:

1. Read the current file state.
2. Validate anchors, hashes, patch context, or destination absence.
3. Build the complete new file contents in memory.
4. Write to a temporary file in the same directory.
5. Flush and rename the temporary file over an existing target, or publish a create-only target with an atomic no-overwrite operation.

If the platform cannot provide the expected atomic behavior, the operation must report that limitation.

## Git guard expectation

Git state is a guardrail, not a hidden side effect. Tools may inspect Git state, may run read-only Git validation commands, and may use Git for tracked moves. The only allowed commit workflow is `git_commit_exact`: it defaults to dry-run, requires an exact complete dirty-path set, requires explicit confirmation before mutation, stages only those paths, creates at most one local commit, and never pushes.

Remote Git authority is intentionally split. `git_remote_check` may fetch exactly one explicit remote branch and report local/remote divergence without changing source files. `git_branch_prepare` may fetch exactly one explicit remote base branch, create or switch to one validated local branch from that base, and reset an existing local branch only when the worktree is clean and `confirm: "reset branch from remote base"` is supplied. `git_merge_readiness` may optionally fetch one explicit remote branch, then run read-only merge-base, rev-count, and diff-name queries to identify files changed on both sides of two validated refs. `git_push_exact` may push only the current `HEAD` to the matching named remote branch after verifying a clean worktree, current branch match, expected HEAD hash, no remote-ahead divergence, and explicit `confirm: "push exact commit"`. Tools must not expose broad reset, checkout, merge, merge-tree through generic command execution, rebase, stash, clean, force-push, push tags, push multiple refs, delete refs, or discard user work outside those explicit branch-prep reset guards.

## Default-deny tools

The server should not expose generic `write_file`, unrestricted `delete`, recursive directory writes, or shell execution as default tools.

Default-deny is a trust feature. Adding a broad write primitive would change the product, not merely expand the API.

## Guarded validation command expectation

`run_guarded_command` is validation support, not a shell. It must:

1. Accept a program name plus argument array, never a shell command string.
2. Resolve the working directory inside the configured repository root.
3. Allow only documented validation-oriented programs and subcommands.
4. Time out rather than running indefinitely.
5. Return command, cwd, allowlist rule, exit code, timeout state, duration, stdout, and stderr.
6. Drain stdout and stderr concurrently so child processes cannot deadlock on full pipes.
7. Redact probable secret values without masking ordinary path-shaped output, env-var names, or documentation prose, then truncate large output.
8. Refuse destructive Git operations and ungated commits.
9. Prefer predefined validation profiles for repeated workflows, but require every profile command to pass the same no-shell allowlist and timeout rules.
10. Store only redacted command logs, address them by opaque ids, and read them back through `read_command_log` rather than exposing arbitrary paths.
11. Add latency instrumentation with monotonic durations and response sizes so performance work is evidence-based, while never recording secrets, environment values, or unredacted command output in timing metadata.

## Setup profile expectation

`setup_profile_run` is a narrow external-mutator planning surface for real project setup workflows. It must not become a generic package-manager runner.

Rules:

1. Accept only server-owned `profile` and `action` names plus typed params.
2. Refuse caller-supplied raw `program`, `args`, shell strings, environment overrides, or arbitrary package lists.
3. Derive the exact command plan inside `core::setup`.
4. Resolve `cwd` inside the configured repository root.
5. Default to dry-run and clearly label planned commands as external mutators, not atomic contextpatch writes.
6. Keep setup/package-manager commands out of `run_guarded_command`; validation and setup are separate trust boundaries.
7. Package-manager selection may be derived by a profile from lockfiles, such as using pnpm for `pnpm-lock.yaml`, but callers must not pass arbitrary package-manager commands.
8. Refuse mutation until the implementation records clean-worktree preconditions, exact confirmation, before/after Git status, changed paths, and expected changed-path classes.
9. Keep profile modules universal and reusable; do not add app-specific setup logic to `contextpatch`.

## Native build and device expectation

Native build and device tools support real plugin workflows without broad shell or native-tool authority.

Rules:

1. Keep `xcodebuild`, repo-relative `gradlew`, `xcrun`, `adb`, `swift`, `kotlinc`, and CocoaPods workflows out of `run_guarded_command`.
2. Accept only server-owned action names and typed params.
3. Derive exact command plans inside `core::native_build`, `core::native_device`, or setup profiles.
4. Default native build and device tools to dry-run.
5. Treat native builds as external validators: they may write build caches, but must not leave Git source status changed.
6. Resolve Android Gradle wrappers as repository-relative executables for native build policy only; do not weaken global executable-name validation.
7. Validate app ids, device ids, simulator/device serials, and artifact paths before planning commands.
8. Require exact confirmation `run native device` before executing actions that change simulator, emulator, or device state.
9. Keep device operations separate from repository mutation; device commands must not claim atomic edit semantics.
10. Bound native command execution with the same no-shell timeout and output-capture runner used by guarded workflows.

## Non-goals

- No unrestricted shell execution
- No recursive bulk rewrite tool
- No silent formatting of unrelated files
- No automatic or broad commits; only explicitly confirmed exact-path local checkpoints
- No broad fetch, checkout, merge, or push; only `git_remote_check`, `git_branch_prepare`, `git_merge_readiness`, and `git_push_exact` under their explicit guards
- No generic package-manager or scaffold runner; only declarative setup profiles under `setup_profile_run`
- No generic native build/device runner; only typed `native_build_run`, `native_device_run`, and setup-profile actions under their explicit guards
- No hidden network calls for edit operations
