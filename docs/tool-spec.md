# Tool Spec

Initial contextpatch tools are intentionally narrow.

This is deliberate: `contextpatch` is a safe patch layer for AI coding agents, not a general filesystem toolbox.

| Tool | Writes? | Guard |
| --- | --- | --- |
| `capability_manifest` | No | Reports exact supported capabilities and unsupported boundaries |
| `preflight_health` | No | Repository/tool readiness summary |
| `read_range` | No | Bounded path and line range |
| `diff_preview` | No | Proposed edit input |
| `replace_exact` | Yes | Old text must match exactly once |
| `apply_patch` | Yes | Unified patch context must apply cleanly |
| `insert_at_anchor` | Yes | Anchor must match exactly once |
| `move_tracked` | Yes | Source exists, destination absent, Git state visible |
| `status_guard` | No | Repository status inspection |
| `write_new_file` | Yes | Destination must not exist |
| `run_guarded_command` | No source edits | Repo-root-confined, no-shell, allowlisted validation command |
| `read_command_log` | No | Reads captured guarded-command logs by opaque id |
| `validation_profile_run` | No source edits | Runs predefined allowlisted validation command sequences |
| `git_commit_exact` | Git index + one local commit | Exact full dirty-path set, dry-run default, explicit confirmation, never pushes |
| `git_remote_check` | Remote-tracking refs only | Fetches one explicit remote branch and reports HEAD/remote divergence without source edits |
| `git_push_exact` | Remote branch update | Clean worktree, exact HEAD, no remote-ahead divergence, explicit confirmation, no force |
| `delete_guarded` | Yes | Expected hash/path confirmation |

## Naming

The public tool names use snake_case because they are protocol-facing. The CLI uses kebab-case commands.

## Tool contracts

### `capability_manifest`

Reports the server's capability contract in machine-readable JSON text.

Required inputs: none.

Rules:

- Must report file tools and process-execution availability honestly.
- Must identify the configured repository root.
- Must state unsupported operations, including arbitrary shell and destructive Git mutations.
- Must distinguish the narrow local `git_commit_exact` checkpoint and guarded `git_remote_check`/`git_push_exact` workflow from unsupported broad Git/destructive workflows.
- Must not mutate repository state.

### `preflight_health`

Reports whether the repository and local validation tools are ready for agent work.

Required inputs: none.

Rules:

- Must report repository cleanliness using the same Git guard semantics as `status_guard`.
- Must report whether guarded process execution is available.
- Must report local availability of expected validation tools without treating missing optional tools as server failure.
- Must not mutate repository state.

### `read_range`

Reads a bounded section of a UTF-8 text file.

Required inputs:

- `path`
- `start_line`
- `end_line`

CLI shape:

```bash
contextpatch read-range <path> --start <line> --end <line>
```

The CLI treats the current working directory as the repository root guard.

Rules:

- Line numbers are 1-based.
- The tool must refuse paths outside the repository root.
- The tool must return line numbers with the content.
- The tool must not read unbounded files by default.

### `diff_preview`

Returns a unified diff for a proposed edit without writing.

Required inputs:

- `path`
- `old`
- `new`

CLI shape:

```bash
contextpatch diff-preview <path> --old <text> --new <text>
```

The CLI treats the current working directory as the repository root guard.

Rules:

- The tool must not mutate files.
- The diff must be generated from the current file contents.
- The current implementation previews exact replacements; `old` must match exactly once.
- If the proposed edit cannot be validated, return a refusal reason instead of a diff.

### `replace_exact`

Replaces text only when the old text appears exactly once.

Required inputs:

- `path`
- `old`
- `new`

CLI shape:

```bash
contextpatch replace-exact <path> --old <text> --new <text>
```

The CLI treats the current working directory as the repository root guard.

Rules:

- Refuse if `old` is empty.
- Refuse if `old` appears zero times.
- Refuse if `old` appears more than once.
- Write atomically.
- Return the changed byte range or a concise edit summary.

### `apply_patch`

Applies a unified patch with context validation.

Required inputs:

- `patch`
- optional repository root guard

Rules:

- Patch context must apply cleanly.
- Paths must stay inside the repository root.
- Partial application must not leave persistent changes.
- The tool must report changed files.

### `insert_at_anchor`

Inserts content before or after an exact anchor.

Required inputs:

- `path`
- `anchor`
- `position`: `before` or `after`
- `content`

Rules:

- Refuse if the anchor is missing.
- Refuse if the anchor appears more than once.
- Write atomically.

### `move_tracked`

Moves or renames a file with repository guardrails.

Required inputs:

- `from`
- `to`

Rules:

- Refuse if source is missing.
- Refuse if destination exists.
- Prefer `git mv` when the source is tracked.
- Do not overwrite destination paths.

### `status_guard`

Reports repository readiness for edits and refuses when Git status is dirty.

Required inputs:

- optional `path`

CLI shape:

```bash
contextpatch status-guard [path]
```

The CLI treats the current working directory as the repository root guard. `contextpatch status` is an alias.

Rules:

- Return a clean summary when no Git changes are present.
- Refuse when the repository, or optional scoped path, has uncommitted changes.
- Include changed path summaries in the refusal.
- Refuse paths outside the repository root.
- Do not mutate repository state.

### `write_new_file`

Creates a new file only when the destination does not exist.

Required inputs:

- `path`
- `content`

CLI shape:

```bash
contextpatch write-new-file <path> --content <text>
```

The CLI treats the current working directory as the repository root guard.

Rules:

- Refuse if the file already exists.
- Refuse parent traversal outside the repository root.
- Refuse missing parent directories.
- Write atomically.

### `run_guarded_command`

Runs a bounded validation-oriented command without invoking a shell.

Required inputs:

- `program`: one of `git`, `cargo`, `bun`, `npm`, or `rg`
- `args`: command arguments; the first argument must be an allowlisted subcommand

Optional inputs:

- `cwd`: working directory relative to the configured repository root
- `timeout_secs`: timeout in seconds, from 1 to 600

Rules:

- The command must run without shell interpolation.
- The working directory must resolve inside the configured repository root.
- The executable must be an allowlisted program name, not a path.
- The subcommand must be allowlisted:
  - `git`: `status`, `diff`, `log`, `show`, `rev-parse`, `ls-tree`
  - `cargo`: `check`, `test`, `build`, `clippy`
  - `bun`: `run`, `test`
  - `npm`: `run`, `test`
  - `rg`: search invocation
- Arguments that directly reference paths outside the repository root must be refused.
- The tool must return command, cwd, allowlist rule, exit code, duration, stdout, and stderr.
- Output must redact probable secret values without masking ordinary path-shaped output, env-var names, or documentation prose, then truncate large streams.
- The tool must refuse arbitrary shell, environment inspection, destructive Git commands, and automatic commits.

### `read_command_log`

Reads a captured guarded-command log produced by `run_guarded_command` or `validation_profile_run`.

Required inputs:

- `log_id`: opaque id returned by a command-running tool

Optional inputs:

- `max_chars`: maximum characters to return, from 1 to 200000; defaults to 12000

Rules:

- `log_id` must be an opaque command-log id, not a path.
- The tool must only read from the contextpatch command-log directory.
- Logs contain the same redacted command output shape as guarded command responses.
- The tool must truncate large responses rather than returning unbounded JSON-RPC payloads.

### `validation_profile_run`

Runs a predefined sequence of allowlisted validation commands as one MCP call.

Required inputs:

- `profile`: one of `repo-basic`, `rust-workspace`, `datacore-vscode`, or `datacore-m6-vscode`

Optional inputs:

- `timeout_secs`: per-command timeout override, from 1 to 600
- `stop_on_failure`: stop after the first non-zero or timed-out command; defaults to true

Rules:

- Profiles must be explicit server-owned command lists, not user-supplied shell snippets.
- Each command must pass the same `run_guarded_command` allowlist, cwd, timeout, and redaction rules.
- The first response should be compact: per-command status, duration, timeout state, and log id.
- Full command output should be retrieved with `read_command_log` only when needed.
- Profiles must not commit, push, reset, checkout, clean, stash, or mutate product files.

### `git_commit_exact`

Creates a local Git commit only when the caller provides the exact complete dirty-path set.

Required inputs:

- `paths`: non-empty list of repository-relative paths
- `subject`: single-line commit subject

Optional inputs:

- `body`: commit body/trailers
- `dry_run`: defaults to `true`
- `confirm`: required literal `commit exact paths` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `commit exact paths`.
- `paths` must exactly match the repository's full dirty-path set from Git status, including untracked files. If any dirty path is missing or any extra path is supplied, the tool must refuse.
- Paths must be normalized repository-relative paths and must not use path traversal, absolute paths, NUL bytes, or Git pathspec metacharacters.
- Rename/copy status entries are refused until a dedicated tracked-move workflow exists.
- The tool may run `git add -- <paths>` and one local `git commit`; it must not fetch, pull, push, reset, checkout, stash, clean, or modify remotes.
- The tool must verify that the staged path set exactly matches `paths` before committing.
- On success, the tool returns the commit hash, short hash, committed paths, and post-commit short status.
- Commit failure after staging must be reported explicitly; it must not pretend the commit succeeded.

### `git_remote_check`

Fetches one explicit remote branch and reports whether the local `HEAD` is behind that remote branch.

Required inputs:

- `branch`: branch name to check

Optional inputs:

- `remote`: remote name; defaults to `origin`

Rules:

- The tool may run only `git fetch <remote> <branch>` plus read-only Git queries.
- The tool must reject malformed remote or branch names.
- The tool must not modify source files or the Git index. It may update remote-tracking refs as the explicit purpose of the tool.
- The response must include `head`, `remote_head`, `remote_ref`, `head_to_remote_empty`, `remote_ahead_count`, and `local_ahead_count`.
- The tool must report whether source status changed during the fetch and refuse if source status changes.

### `git_push_exact`

Pushes exactly the current branch `HEAD` to the matching branch on an explicit remote.

Required inputs:

- `remote`
- `branch`
- `expected_head`
- `confirm`: literal `push exact commit`

Rules:

- The tool must require a clean worktree before pushing.
- The current branch must exactly match `branch`.
- The current `HEAD` must match `expected_head`.
- The tool must fetch `<remote> <branch>` immediately before the push.
- The remote-tracking ref must not be ahead of `HEAD`, and it must be an ancestor of `HEAD`; divergent or non-fast-forward states must be refused.
- The push refspec must be exactly `HEAD:refs/heads/<branch>` for the requested branch.
- The tool must not use force push, delete refs, push tags, push multiple branches, or push a different local branch.
- On success, the response must include the pushed commit hash, branch, remote, previous remote head, refspec, and post-push status.

### Latency instrumentation

Future Stage 2B command and workflow tools should expose optional timing metadata. The metadata should be diagnostic, not part of the safety decision itself.

Recommended fields:

- `request_received_to_dispatch_ms`
- `argument_validation_ms`
- `allowlist_validation_ms`
- `child_spawn_ms`
- `child_runtime_ms`
- `stdout_stderr_drain_ms`
- `redaction_truncation_ms`
- `log_write_ms`
- `response_bytes`
- `total_tool_ms`

Rules:

- Timing metadata must not include command output, arguments classified as secret values, or environment values.
- Timings should be monotonic-duration measurements, not wall-clock timestamps.
- Tools should keep compact summaries as the default and use `read_command_log` for large output details.
- The instrumentation goal is p50/p95/p99 diagnosis across long sessions, especially to distinguish MCP transport overhead from process/output handling.

### `delete_guarded`

Deletes a file only with explicit confirmation.

Required inputs:

- `path`
- expected file hash or equivalent exact confirmation

Rules:

- Refuse directories.
- Refuse missing confirmation.
- Refuse hash mismatch.
- Report the deleted path.

## MVP implementation subset

Stage 1 ships:

1. `replace_exact`
2. `read_range`
3. `write_new_file`
4. `diff_preview`
5. `status_guard`
6. `capability_manifest`
7. `preflight_health`
8. `run_guarded_command`
9. `read_command_log`
10. `validation_profile_run`
11. `git_commit_exact`
12. `git_remote_check`
13. `git_push_exact`

The remaining tools stay documented as planned Stage 2 boundaries until implemented. See `docs/implementation-roadmap.md`.
