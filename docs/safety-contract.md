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

Git state is a guardrail, not a hidden side effect. Tools may inspect Git state, may run read-only Git validation commands, and may use Git for tracked moves. The only allowed commit workflow is `git_commit_exact`: it defaults to dry-run, requires an exact complete dirty-path set, requires explicit confirmation before mutation, stages only those paths, creates at most one local commit, and never pushes. Tools must not reset, checkout, stash, clean, fetch, push, or discard user work.

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

## Non-goals

- No unrestricted shell execution
- No recursive bulk rewrite tool
- No silent formatting of unrelated files
- No automatic or broad commits; only explicitly confirmed exact-path local checkpoints
- No fetch or push
- No hidden network calls for edit operations
