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
8. If validation command execution is exposed, it must interpose no shell, confine argument paths to the repository root, be allowlisted, timeout-bound, and auditable. It must not be described as a sandbox: the child inherits the server environment, and reviewed repository code it runs can read files, start subprocesses, and use the network with the server user's permissions. Only the task-image path carries the documented container isolation. See `docs/execution-threat-model.md`.
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
22. Exact replacement should accept an optional complete-file SHA-256 so shared-worktree callers can reject stale reads in addition to validating the text anchor.
23. Direct MCP reads, writes, and Git operations must have bounded reply deadlines. Expiry must report an unknown outcome and recovery steps, not claim failure or roll back a worker that may still be running.
24. Receipt-enabled mutations must persist a begin record before mutation and a settle record with resulting state afterward so an interrupted transport can be reconciled.
25. ContextPatch mutations for one repository must be cooperatively serialized across server processes; file compare-and-write operations must hold a filesystem-identity target lock shared across configured-root and descendant-repository views during verification and atomic replacement.
26. Detached deadline workers must have a fixed ceiling. Saturation must refuse a new operation before it starts.
27. If sidecar artifact cleanup is exposed, it must delete one exact regular file under the fixed artifact root only after a digest-reporting dry run, matching SHA-256, and explicit confirmation.
28. If bulk exact replacement is exposed, every entry must validate before the first write and each
    target must be revalidated under its mutation lock before apply. Only each file write is atomic:
    interruption or apply failure may leave a prefix applied and must direct callers to per-file
    receipts and current-state inspection.
29. ID-bearing MCP tool calls must run through a bounded worker pool so no tool call blocks stdin
    processing or an independent request. Concurrent replies must retain their JSON-RPC ids and be
    serialized as complete response lines so clients can safely correlate out-of-order completion.
    Stdin ingestion must use a bounded queue so a failed stdout write wakes the dispatcher and
    terminates the server promptly even when the client leaves stdin open.
30. Background task-image, Harbor, and validation-profile work must share a fixed concurrency cap,
    return stable pollable log ids, and report restart-orphaned active work as unknown.
31. Repository file reads and writes must refuse symlink components at every position, including the
    leaf, rather than follow them, under both configured and selected repository roots. Content reads,
    metadata inspection, directory listing, exact-hash replacement, executable-bit changes, directory
    creation, and create-only publication currently require Unix and must traverse through no-follow
    directory descriptors rooted in the repository's own authority; non-Unix builds must refuse rather
    than use path-based check-then-open fallbacks.
32. Regular-file metadata and bounded byte reads must derive size, digest, line count, and returned
    bytes from one fixed-extent streamed snapshot. They must use bounded memory, refuse metadata or
    path-identity changes, and never chase a concurrently growing end-of-file.

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
11. A supplied complete-file `expected_sha256` is malformed or no longer matches the current file.
12. A sidecar artifact deletion targets a missing, non-normalized, symlinked, non-regular, hash-mismatched, or outside-root path.
13. A bulk exact-replacement request repeats the same filesystem file, including through case or
    hard-link aliases, or any entry fails the normal exact-replacement checks.
14. A repository file operation encounters an intermediate symlink or non-directory component, or
    a symlink target where an existing regular file is required.
15. An executable-bit change targets a file with more than one hard link.

Refusals must return a clear reason. They must not pretend success.

## Atomic write expectation

Persistent file writes should use this pattern:

1. Read the current file state.
2. Validate anchors, hashes, patch context, or destination absence.
3. Build the complete new file contents in memory.
4. Write to a temporary file in the same directory.
5. Flush and rename the temporary file over an existing target, or publish a create-only target with an atomic no-overwrite operation.

On Unix, guarded repository file operations must hold already-open parent-directory descriptors
while inspecting, creating, publishing, or replacing leaf entries. Path-based prechecks alone are
not sufficient because an intermediate symlink must not redirect the operation outside the root.
Immediately before publication, replacement, or descriptor-based mode mutation, the expected parent
must be re-walked from the retained repository-root descriptor and its filesystem identity must still
match the held descriptor.

The repository root itself is reached through authority rather than by name. A selected repository lends
the directory descriptor that validated it; a configured repository has one opened once with
`O_DIRECTORY | O_NOFOLLOW`, and that single open is the only name resolution an operation performs. Every
component after it is reached with descriptor-relative, no-follow calls. The two cases must behave
identically: a safety property that held only for selected repositories would not be a safety property.

A consequence is stated explicitly because it is user-visible. No repository file operation follows a
symlink at any component, including the leaf, and including a symlink whose target resolves back inside the
repository. A repository root whose own name has become a symlink is refused rather than followed to
whatever now occupies that name. Inspection is the sole exception and is not an exception to following: a
final symlink may be *described*, including its target and whether that target resolves inside the
repository, but it is never opened, hashed, or traversed.

Boundaries that require a stable name rather than authority — repository and file mutation locks, the
receipt journal, and scratch identity — must derive it from one canonical label helper that resolves a
configured root once and returns a selected root's recorded path unchanged. A selected repository's path
must never be re-resolved, and no operation may read that label to gain access to a file.

If the platform cannot provide descriptor-relative operations, guarded repository file access must refuse
rather than degrade to path resolution.

If the platform cannot provide the expected atomic behavior, the operation must report that limitation.

Repository locks and final parent-identity checks coordinate ContextPatch and reject namespace changes
observed before mutation. No portable POSIX primitive can atomically prove ancestry while publishing
through a directory descriptor, so a privileged external process that renames that directory after
the final identity check remains outside the cooperative-writer guarantee. Callers must use current
hash/state checks when unrelated external programs may mutate the same tree.

Git actions carry an explicit per-action worktree-root policy, because the configured root bounds them
unequally. An action that names the paths it touches is already confined by path normalization, which
rejects parent components, absolute paths, and pathspec metacharacters, so it only needs the root to
resolve and a descendant root stays useful. An action that names no paths has nothing else bounding it:
branch preparation and pushing act on whatever repository encloses the configured root, so under a
subdirectory root they would reach files and refs the operator never configured. Those actions therefore
require the root to be exactly a Git worktree root, verified by resolving it and requiring Git's own
reported top level to equal it, with no upward walk. The check runs before any fetch, ref update, branch
switch, or push, so a refusal leaves the enclosing repository untouched. Argument-shape validation may
precede it, which is harmless because it cannot mutate. Descendant worktrees stay reachable through the
wrapper `repository` selector, which itself resolves to an exact worktree root.

Every successful file mutation reports the SHA-256 digest of the content it wrote. `replace_exact`,
`write_new_file`, `write_new_file_base64`, `bulk_replace_exact`, `bulk_write_new_files_base64`, the
artifact writers, and `write_existing_file_exact_hash` all carry it, so a caller can pass a reported
digest straight back as the next `expected_sha256` without an intervening read. That matters because a
guard that costs an extra round trip on every write is a guard that gets skipped. A reported digest
describes the bytes that write produced and nothing later: once an external writer touches the file, the
digest stops matching and the next guarded write refuses, which is the intended outcome rather than a
regression. For a multi-hunk file the digest covers the single write that carried every hunk, so all of
that file's entries report the same value.

For `bulk_replace_exact`, validation is batch-wide but application is deliberately per-file. Entries are
grouped by normalized path, and every hunk for one file is resolved against a single captured snapshot of
that file rather than against text an earlier hunk already rewrote, so a hunk's validity never depends on
the order its siblings would have been applied in. Hunks that resolve to the same byte range, hunks whose
ranges intersect, and entries demanding different digests for one file are all refused during validation.
Two different paths that resolve to one file remain refused, because grouping is by path and an alias
would otherwise yield two snapshots of the same bytes whose second write discards the first.

A validation refusal occurs before the first write and leaves all targets unchanged, and it journals a
`refused` receipt for every named target so that a refused batch is distinguishable from a batch that was
never attempted. Those receipts are written per file rather than per hunk, matching the one write a file
would have received, and each records the digest before and after, reporting `unknown` instead of
`refused` when they differ because an external writer during validation defeats any claim that the file
is untouched. A journal failure is appended to the refusal rather than replacing it, so it never masks the
validation error.

Apply compares each
target's exact captured bytes while holding its mutation lock, verifies that the plan is being applied
under the same canonical repository root where it was created, then uses the normal atomic replacement
path, so every hunk for a file lands in one write and no file is observed partially edited. Permission
bits are preserved by that path, so editing an executable file does not silently clear its execute bit.
A hard link in another repository therefore cannot reuse a validated plan. There is no cross-file
transaction or rollback: an apply failure, process interruption, or reply timeout can leave an applied
prefix of files, and recovery must use per-file receipts plus current file state.

## Reply deadlines and mutation receipts

Direct MCP operations have bounded reply waits: 30 seconds for reads, 60 seconds for direct writes, and 120 seconds for Git and GitHub operations. Process, profile, setup, and native tools retain their operation-specific execution timeouts. Production Git subprocesses also have a 90-second hard timeout below the outer reply deadline; a timeout terminates the child and, on Unix, its process group. The bounded runner also terminates members left in the original process group after the direct child exits, and a wait or output-setup failure must terminate and reap the child before returning. A descendant can escape process-group cleanup by creating a new session, so the runner is not a process sandbox; nonblocking cancellable output readers perform a short final drain and then close their pipe ends so an escaped writer cannot retain server reader threads. A reply deadline still does not cancel other workers because interrupting a filesystem or non-child operation mid-flight could make the result less safe. That refusal must therefore say that the outcome is unknown and name a state-recovery tool. No more than 16 deadline workers may be active; saturation refuses the next call before its operation starts.

The stdio transport runs every ID-bearing `tools/call` through a bounded 16-worker pool and may serialize completed responses in a different order than requests arrived. JSON-RPC `id`, not line position, is the correlation key. A bounded stdin queue lets the dispatcher detect worker stdout failure and terminate promptly without waiting for the client to close stdin. Initialization, listing, notifications, malformed requests, and unknown methods remain inline. Confirmed task-image execution, typed Harbor runs, and validation profiles use a separate shared two-job background limit and return immediately with `status: "running"` plus a `log_id`; callers poll that id on the same server without restarting work.

`read_write_receipts` provides durable recovery evidence outside the repository under the stable scratch root. Receipt-enabled mutations append a begin record before mutation and a settle record after collecting the resulting file digest or Git HEAD. Reads, appends, and journal rotation must be cross-process locked, and rotation must preserve entries that never settled. A bounded multi-file mutation must reserve journal capacity before its first write and suppress rotation between its per-file records, so an interrupted batch retains its settled prefix alongside the interrupted entry.

Settled receipt outcomes are `applied` when the operation returned success, `refused` when it failed and observed state stayed unchanged, and `unknown` when it failed after state changed or could not be read. `interrupted` is reserved for attempts with no settlement record. Receipt coverage must not be overstated. The current server journals exact replacements, each bulk exact-replacement apply attempt, exact-hash whole-file overwrites, exact untracked-file deletions, and exact/scoped/prefix local commits. Other operations still require their normal state checks after an interrupted reply.

ContextPatch MCP mutations for a repository use an advisory repository lock, and exact replacement/hash writes also lock their target by filesystem identity across verification and atomic write. Case aliases and hard links therefore share a lock. Lock contention refuses before mutation starts. These locks coordinate ContextPatch processes only; external programs that ignore them can still race, so caller-supplied SHA guards and post-operation state checks remain meaningful.

## Git guard expectation

Git state is a guardrail, not a hidden side effect. Tools may inspect Git state and may run read-only Git validation commands. `git_stage_exact` is index-only: it stages explicit dirty paths after a clean-index gate, defaults to dry-run, requires exact confirmation, and must not commit. `git_staged_scope_check` is read-only: it may inspect the index and report whether staged paths match allowed exact paths/prefixes and required paths, but it must not stage, unstage, commit, fetch, push, reset, checkout, stash, clean, or modify remotes. The allowed commit workflows are `git_commit_exact`, `git_commit_scoped`, and `git_commit_prefix`: all default to dry-run, require explicit confirmation before mutation, stage only requested or prefix-expanded paths, create at most one local commit, and never push. `git_commit_exact` requires an exact complete dirty-path set. `git_commit_scoped` permits an explicit subset of dirty paths only when the index is clean, preserving unrelated dirty paths. `git_commit_prefix` expands explicit prefixes to exact dirty paths first, reports that set, and commits only that expanded set. `git_remote_list` may run only read-only remote inspection. `delete_untracked_exact` may remove only explicit untracked regular files after dry-run and confirmation. `delete_generated_prefix` may expand explicit prefixes only to ignored/untracked files and empty directories, must refuse tracked paths, and must not behave like broad `git clean`.

`move_tracked` may move only one clean tracked regular file to one absent untracked path with an existing in-repository parent. It must reject symlink components and pathspec metacharacters, require a clean index, default to dry-run, require `confirm: "move tracked file"` for mutation, use guarded `git mv`, and verify source absence, destination tracking, and SHA-256 preservation.

`delete_guarded` may delete only one clean tracked regular worktree file whose current lowercase SHA-256 matches the caller's expected digest. It must reject symlinks and non-regular files, default to dry-run, require `confirm: "delete tracked file"` for mutation, recheck the digest immediately before removal, preserve the index entry, and verify an unstaged tracked deletion.

Remote Git authority is intentionally split. `git_remote_check` may fetch exactly one explicit remote branch and report local/remote divergence without changing source files. `git_branch_prepare` defaults to a non-fetching dry-run that reports exact command argv; only `dry_run: false` may fetch one explicit remote base branch and create or switch to one validated local branch from that base, and it may reset an existing local branch only when the worktree is clean and `confirm: "reset branch from remote base"` is supplied. `git_merge_readiness` may optionally fetch one explicit remote branch, then run read-only merge-base, rev-count, and diff-name queries to identify files changed on both sides of two validated refs. `git_push_exact` may push only the current `HEAD` to the matching named remote branch after verifying a clean worktree, current branch match, expected HEAD hash, no remote-ahead divergence, and explicit `confirm: "push exact commit"`. Tools must not expose broad reset, checkout, merge, merge-tree through generic command execution, rebase, stash, clean, force-push, push tags, push multiple refs, delete refs, or discard user work outside those explicit branch-prep reset guards.

## Default-deny tools

The server should not expose generic `write_file`, unrestricted `delete`, recursive directory writes, or shell execution as default tools. Directory creation, if exposed, must be create-only for one explicit path, refuse existing targets, and keep all created components inside the repository root.

Binary file creation, if exposed, must be create-only for one explicit path, refuse existing targets, decode bounded base64 content, optionally enforce an expected decoded byte count, and use the same repository-root and atomic publish guards as text create-only writes.

Bulk fixture import, if exposed, must remain a bounded create-only variant of binary file creation. It may create missing parents only when explicitly requested, but it must reject overwrites, traversal, symlinked parents, duplicate paths, excessive file counts, and excessive total decoded bytes. Parent creation must not follow a pre-existing symlink or create directories through one.

Sidecar artifact creation, if exposed, must use a fixed artifact root outside the repository. It must reject absolute paths and traversal, refuse overwrites, optionally create parents only under that artifact root, and never claim repository mutation semantics.

Read-only file inspection may report metadata for one path or a bounded batch, SHA-256 digests, line counts, bounded depth-limited directory trees, symlink status, and bounded byte ranges. A final symlink may be reported but must not be opened or hashed, and intermediate symlinks must be refused. Recursive listings must cap both depth and entry count, bound retained directory candidates and metadata reads before truncation, remain deterministic, and never follow symlink directories; inspection must stay repository-root-confined and must not become unrestricted filesystem traversal.

Executable-bit mutation, if exposed, must operate on one existing non-symlinked regular file, default to dry-run, require the current content SHA-256 and current octal mode plus exact confirmation, hold the target mutation lock while revalidating, change only mode `0111`, and verify that file content remains byte-identical.

All `tools/call` responses must fit within the server's 900 KiB serialized-response envelope.
Successful operations whose output would exceed that limit return a compact success marker rather
than a client-side transport failure; mutation callers are explicitly warned not to retry solely
because detailed output was omitted. Read tools should still expose narrower ranges, limits,
projections, or paging so useful data can be retrieved without relying on the fallback.

Every advertised action schema is closed with `additionalProperties: false`, and the server enforces
that boundary at runtime for both direct calls and nested `project_execute` actions. Unknown action
arguments must refuse before reply deadlines, repository mutation locks, or handler dispatch; a
schema-only hint is not an execution guard.

Default-deny is a trust feature. Adding a broad write primitive would change the product, not merely expand the API.

## Guarded validation command expectation

`run_guarded_command` is validation support, not a shell. It must:

1. Accept a program name plus argument array, never a shell command string.
2. Resolve the working directory inside the configured repository root.
3. Allow only documented validation-oriented programs and subcommands.
4. Time out rather than running indefinitely. Generic guarded commands default to 120 seconds and remain capped at 600 seconds. Only the typed Harbor adapter may use the narrow 3600-second ceiling.
5. Return command, cwd, allowlist rule, exit code, timeout state, duration, stdout, and stderr.
6. Drain stdout and stderr concurrently so child processes cannot deadlock on full pipes.
   Child wait, pipe-capture, or nonblocking-setup failures must terminate and reap the child before
   returning the error.
7. Redact probable secret values without masking ordinary path-shaped output, env-var names, or documentation prose, then truncate large output and report truncation caused by either process capture or response formatting.
8. Refuse destructive Git operations and ungated commits.
9. Prefer predefined validation profiles for repeated workflows, but require every profile command to pass the same no-shell allowlist and timeout rules.
10. Store only redacted command logs, address them by opaque ids, and read them back through `read_command_log` rather than exposing arbitrary paths. Asynchronous logs must expose explicit lifecycle status and report restart-orphaned active work as unknown rather than completed or failed.
11. Resolve executable names from the host `PATH` plus server-configured validation path entries such as `CONTEXTPATCH_VALIDATION_PATHS`, while still refusing caller-supplied executable paths or per-request environment overrides.
12. Add latency instrumentation with monotonic durations and response sizes so performance work is evidence-based, while never recording secrets, environment values, or unredacted command output in timing metadata.
13. Permit the exact `{scratch}` token inside arguments as the only general outside-repository byproduct path. Expand it to the stable repository-specific scratch root and refuse lexical traversal outside that root.

`artifact_python_run` is an artifact-root scratch runner, not a shell. It may run only a Python script that already exists under the fixed artifact root, with bounded argv and repo-root-confined cwd, so analysis scripts do not have to be created inside the repository. It must not accept caller-supplied executable paths, shell snippets, or per-request environment overrides.

`task_image_python_run` is a typed asynchronous task-environment runner, not general Docker authority. It may build only `task/environment/Dockerfile` from `task/environment`, then invoke only `python3` or `python` on one existing non-symlinked repository-relative `.py` file. It must default to a plan, require exact confirmation, separate build and run timeouts, mount the repository read-only, disable runtime networking, drop capabilities, enforce no-new-privileges/PID/read-only-root/tmpfs limits, and return a stable log id before execution completes. Concurrent jobs must use distinct execution-image tags and container names. A runtime timeout, indeterminate run-wrapper failure, or Docker exit code 125 must force-remove its named container after the Docker client is terminated, and the per-job execution tag must be removed after every build attempt that may have created it. Callers must not choose Docker arguments, mounts, image names, entrypoints, environment overrides, or shell strings.

`harbor_run_start` is a typed asynchronous validation adapter. It may start only `harbor run -p <normalized repository project> --agent <validated agent>`, share the background cap with task-image and validation-profile jobs, return a stable opaque log id before completion, and use `read_command_log` for polling without restarting the command. Terminal logs may report completed, failed, or timed out; an active log owned by an earlier server instance must report unknown. Structured evidence must remain repository-confined, require Harbor's exact job/trial layout, enumerate the union of rewarded and exception-only trials with exception-bearing trials first, cap trial count and total serialized evidence, reject symlink or traversal paths, read bounded artifacts from guarded already-open file descriptors rather than canonicalize-then-reopen paths, redact verifier output, and surface malformed or missing evidence explicitly.

`validation_profile_run` is a typed asynchronous profile adapter. It must validate the named profile before starting, return one stable profile log id immediately, and run only server-owned commands through the same allowlist, timeout, redaction, and logging rules. Its terminal log may reference bounded per-command log ids. Profile-owned semantic gates, including Dynamo Harbor reward requirements, must produce a failed terminal state even when every child process exits zero. It shares the same two-job cap and unknown-after-restart semantics as task-image and Harbor work.

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
4. Redact and bound workflow log output and require an explicit run or job database id; bounded job logs must default to retaining the terminal tail and expose the head only as an explicit projection. Do not expose arbitrary Actions API access.
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
