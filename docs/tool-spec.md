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
  - `git`: `status`, `diff`, `log`, `show`, `rev-parse`
  - `cargo`: `check`, `test`, `build`, `clippy`
  - `bun`: `run`, `test`
  - `npm`: `run`, `test`
  - `rg`: search invocation
- Arguments that directly reference paths outside the repository root must be refused.
- The tool must return command, cwd, allowlist rule, exit code, duration, stdout, and stderr.
- Output must redact secret-like lines and truncate large streams.
- The tool must refuse arbitrary shell, environment inspection, destructive Git commands, and automatic commits.

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

The remaining tools stay documented as planned Stage 2 boundaries until implemented. See `docs/implementation-roadmap.md`.
