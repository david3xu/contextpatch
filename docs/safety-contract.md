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
13. If binary file creation is exposed, it must be create-only, repo-root-confined, atomic, byte-count-guardable, and size-bounded.
14. If sidecar artifact creation is exposed, it must write only under a fixed non-repository artifact root, create-only, and report that it does not mutate the repo.
15. If untracked cleanup is exposed, it must delete only explicit untracked regular files with dry-run and confirmation gates, not expose broad `git clean`.
16. If GitHub PR or fork support is exposed, it must use narrow typed workflows with dry-run/confirmation for mutations, not arbitrary `gh` passthrough.
17. If fixture-generator execution is exposed, it must be a typed repo-relative script workflow with dry-run, confirmation, pre-existing dirty-path gates, timeout, and post-run changed-path verification.
18. If bulk fixture import is exposed, it must be create-only, repo-root-confined, file-count and byte-count bounded, and must refuse overwrites and traversal.
19. If a shell-script validation exception is exposed, it must be fixed to a specific repository script such as `references/check-base-image.sh`, not arbitrary shell authority.
20. If tracked-file moves are exposed, they must require a clean tracked regular source, absent destination, clean index, dry-run, exact confirmation, and post-move hash verification.
21. If tracked-file deletion is exposed, it must require a clean tracked regular file, exact current SHA-256, dry-run, exact confirmation, and leave a reviewable unstaged deletion.

## Required refusal cases

Write tools must refuse the operation when:

1. The target path is outside the configured repository root.
2. The target path points to a directory when a file is required.
3. The expected anchor or old text is missing.
4. The expected anchor or old text appears more than once.
5. The destination already exists for create-only writes.
6. The destination already exists for create-only directory creation.
7. A delete request lacks the expected file hash or equivalent confirmation.
8. Repository status violates the requested guard policy.
9. A tracked-file move or deletion encounters a symlink component, non-regular file, untracked source, dirty source, or mismatched content hash.
10. A tracked-file move has a pre-existing or indexed destination or a non-clean Git index.

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

Git state is a guardrail, not a hidden side effect. Tools may inspect Git state and may run read-only Git validation commands. `git_stage_exact` is index-only: it stages explicit dirty paths after a clean-index gate, defaults to dry-run, requires exact confirmation, and must not commit. `git_staged_scope_check` is read-only: it may inspect the index and report whether staged paths match allowed exact paths/prefixes and required paths, but it must not stage, unstage, commit, fetch, push, reset, checkout, stash, clean, or modify remotes. The allowed commit workflows are `git_commit_exact`, `git_commit_scoped`, and `git_commit_prefix`: all default to dry-run, require explicit confirmation before mutation, stage only requested or prefix-expanded paths, create at most one local commit, and never push. `git_commit_exact` requires an exact complete dirty-path set. `git_commit_scoped` permits an explicit subset of dirty paths only when the index is clean, preserving unrelated dirty paths. `git_commit_prefix` expands explicit prefixes to exact dirty paths first, reports that set, and commits only that expanded set. `git_remote_list` may run only read-only remote inspection. `delete_untracked_exact` may remove only explicit untracked regular files after dry-run and confirmation. `delete_generated_prefix` may expand explicit prefixes only to ignored/untracked files and empty directories, must refuse tracked paths, and must not behave like broad `git clean`.

`move_tracked` may move only one clean tracked regular file to one absent untracked path with an existing in-repository parent. It must reject symlink components and pathspec metacharacters, require a clean index, default to dry-run, require `confirm: "move tracked file"` for mutation, use guarded `git mv`, and verify source absence, destination tracking, and SHA-256 preservation.

`delete_guarded` may delete only one clean tracked regular worktree file whose current lowercase SHA-256 matches the caller's expected digest. It must reject symlinks and non-regular files, default to dry-run, require `confirm: "delete tracked file"` for mutation, recheck the digest immediately before removal, preserve the index entry, and verify an unstaged tracked deletion.

Remote Git authority is intentionally split. `git_remote_check` may fetch exactly one explicit remote branch and report local/remote divergence without changing source files. `git_branch_prepare` may fetch exactly one explicit remote base branch, create or switch to one validated local branch from that base, and reset an existing local branch only when the worktree is clean and `confirm: "reset branch from remote base"` is supplied. `git_merge_readiness` may optionally fetch one explicit remote branch, then run read-only merge-base, rev-count, and diff-name queries to identify files changed on both sides of two validated refs. `git_push_exact` may push only the current `HEAD` to the matching named remote branch after verifying a clean worktree, current branch match, expected HEAD hash, no remote-ahead divergence, and explicit `confirm: "push exact commit"`. Tools must not expose broad reset, checkout, merge, merge-tree through generic command execution, rebase, stash, clean, force-push, push tags, push multiple refs, delete refs, or discard user work outside those explicit branch-prep reset guards.

## Default-deny tools

The server should not expose generic `write_file`, unrestricted `delete`, recursive directory writes, or shell execution as default tools. Directory creation, if exposed, must be create-only for one explicit path, refuse existing targets, and keep all created components inside the repository root.

Binary file creation, if exposed, must be create-only for one explicit path, refuse existing targets, decode bounded base64 content, optionally enforce an expected decoded byte count, and use the same repository-root and atomic publish guards as text create-only writes.

Bulk fixture import, if exposed, must remain a bounded create-only variant of binary file creation. It may create missing parents only when explicitly requested, but it must reject overwrites, traversal, duplicate paths, excessive file counts, and excessive total decoded bytes.

Sidecar artifact creation, if exposed, must use a fixed artifact root outside the repository. It must reject absolute paths and traversal, refuse overwrites, optionally create parents only under that artifact root, and never claim repository mutation semantics.

Read-only file inspection may report metadata, SHA-256 digests, line counts, one-directory entries, symlink status, and bounded byte ranges, but it must stay repository-root-confined and must not become recursive unrestricted filesystem traversal.

Default-deny is a trust feature. Adding a broad write primitive would change the product, not merely expand the API.

## Guarded validation command expectation

`run_guarded_command` is validation support, not a shell. It must:

1. Accept a program name plus argument array, never a shell command string.
2. Resolve the working directory inside the configured repository root.
3. Allow only documented validation-oriented programs and subcommands.
4. Time out rather than running indefinitely. The default is 120 seconds and the general maximum is 600 seconds; only exact `harbor run` commands may use the narrow 3600-second ceiling.
5. Return command, cwd, allowlist rule, exit code, timeout state, duration, stdout, and stderr.
6. Drain stdout and stderr concurrently so child processes cannot deadlock on full pipes.
7. Redact probable secret values without masking ordinary path-shaped output, env-var names, or documentation prose, then truncate large output.
8. Refuse destructive Git operations and ungated commits.
9. Prefer predefined validation profiles for repeated workflows, but require every profile command to pass the same no-shell allowlist and timeout rules.
10. Store only redacted command logs, address them by opaque ids, and read them back through `read_command_log` rather than exposing arbitrary paths.
11. Resolve executable names from the host `PATH` plus server-configured validation path entries such as `CONTEXTPATCH_VALIDATION_PATHS`, while still refusing caller-supplied executable paths or per-request environment overrides.
12. Add latency instrumentation with monotonic durations and response sizes so performance work is evidence-based, while never recording secrets, environment values, or unredacted command output in timing metadata.

`artifact_python_run` is an artifact-root scratch runner, not a shell. It may run only a Python script that already exists under the fixed artifact root, with bounded argv and repo-root-confined cwd, so analysis scripts do not have to be created inside the repository. It must not accept caller-supplied executable paths, shell snippets, or per-request environment overrides.

`image_cleanliness_check_run` and `docker_image_inspect` are separate narrow Docker gates, not general Docker authority. They may plan or run only `docker run --rm --network none --entrypoint find <image> / -name <filename>` and `docker image inspect <image>` respectively, default to dry-run, require exact confirmation for execution, and must not expose mounts, caller-selected entrypoints, network access, shell snippets, or arbitrary Docker arguments.

## Fixture workflow expectation

Fixture generation support exists to cover data-heavy benchmark tasks without granting broad shell or filesystem authority.

Rules:

1. Prefer `fixture_generator_run` for repo-relative Python generators whose outputs can be declared as exact paths or prefixes.
2. Permit temporary untracked generator scripts by listing them as allowed pre-existing dirty paths, but require callers to delete them before commit when they are not submission artifacts.
3. Require dry-run by default and exact confirmation before running the generator.
4. Verify after execution that all dirty paths are either allowed pre-existing dirty paths or declared outputs.
5. Refuse and report undeclared changed paths rather than pretending the run was safe.
6. Keep `pip`, Docker, arbitrary `python -m`, arbitrary shell scripts, and caller-supplied shell snippets outside the fixture workflow.
7. Use `bulk_write_new_files_base64` only as a bounded create-only fallback for fixture files that cannot be generated on the host.
8. Keep the base-image shell-script exception fixed to `references/check-base-image.sh`, optionally with the exact `task` argument, through `base_image_check_run`.
9. Use `fixture_manifest_verify` to fail on missing, modified, or unlisted fixture files before deriving truth from a fixture tree.
10. Use `fixture_manifest_refresh` to regenerate manifests from declared paths or prefixes; overwrites require dry-run/confirmation and the current manifest SHA-256.
11. Use `write_existing_file_exact_hash` only for existing-file synchronization where the caller supplies the exact current SHA-256 and reviews the replacement content.
12. Use `file_info` to obtain that current SHA-256 instead of creating a temporary digest script in the repository.

## GitHub workflow expectation

GitHub automation is review workflow support, not broad platform authority. Tools may inspect authentication state, view PR details, retrieve bounded locally filtered comments, read structured PR checks, discover bounded runs for one exact commit, inspect one workflow run or job log, rerun only failed jobs from one explicit run, create one pull request, and prepare a fork through explicit typed actions.

Rules:

1. Use `gh` with explicit argv and no shell.
2. Keep read-only PR actions separate from mutating PR creation and fork preparation.
3. Permit fork/upstream reads only through a validated `OWNER/REPO` selector passed as a separate `--repo` argument.
4. Redact and bound workflow log output and require an explicit run or job database id; do not expose arbitrary Actions API access.
5. Require a full commit SHA and bounded limit for run discovery; do not infer history from a branch name.
6. Permit comment filtering only as bounded local body matching; do not accept caller-provided `jq`, GraphQL, REST paths, or query expressions.
7. Limit workflow reruns to `gh run rerun <run_id> --failed`, default to dry-run, and require exact confirmation.
8. Default PR creation and fork preparation to dry-run and return the exact command plan.
9. Require exact confirmation before creating a PR or preparing a fork.
10. Do not expose arbitrary `gh` subcommands, secret reads, workflow dispatch, release publishing, repository deletion, forceful Git operations, or broad issue/project mutation through this surface.

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
- No generic fixture generator runner; only `fixture_generator_run` with declared output verification, `fixture_manifest_verify`/`fixture_manifest_refresh` for exact file-set integrity, and `bulk_write_new_files_base64` create-only imports
- No hidden network calls for edit operations
