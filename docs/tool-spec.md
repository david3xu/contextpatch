# Tool Spec

Initial contextpatch tools are intentionally narrow.

This is deliberate: `contextpatch` is a safe patch layer for AI coding agents, not a general filesystem toolbox.

| Tool | Writes? | Guard |
| --- | --- | --- |
| `read_range` | No | Bounded path and line range |
| `diff_preview` | No | Proposed edit input |
| `replace_exact` | Yes | Old text must match exactly once |
| `apply_patch` | Yes | Unified patch context must apply cleanly |
| `insert_at_anchor` | Yes | Anchor must match exactly once |
| `move_tracked` | Yes | Source exists, destination absent, Git state visible |
| `status_guard` | No | Repository status inspection |
| `write_new_file` | Yes | Destination must not exist |
| `delete_guarded` | Yes | Expected hash/path confirmation |

## Naming

The public tool names use snake_case because they are protocol-facing. The CLI uses kebab-case commands.

## Tool contracts

### `read_range`

Reads a bounded section of a UTF-8 text file.

Required inputs:

- `path`
- `start_line`
- `end_line`

Rules:

- Line numbers are 1-based.
- The tool must refuse paths outside the repository root.
- The tool must return line numbers with the content.
- The tool must not read unbounded files by default.

### `diff_preview`

Returns a unified diff for a proposed edit without writing.

Required inputs:

- `path`
- proposed edit input, either full candidate contents or a structured replacement request

Rules:

- The tool must not mutate files.
- The diff must be generated from the current file contents.
- If the proposed edit cannot be validated, return a refusal reason instead of a diff.

### `replace_exact`

Replaces text only when the old text appears exactly once.

Required inputs:

- `path`
- `old`
- `new`

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

Reports repository readiness for edits.

Required inputs:

- repository root

Rules:

- Return clean/dirty status.
- Include changed path summaries.
- Do not mutate repository state.

### `write_new_file`

Creates a new file only when the destination does not exist.

Required inputs:

- `path`
- `content`

Rules:

- Refuse if the file already exists.
- Refuse parent traversal outside the repository root.
- Write atomically.

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

The first implementation should ship only:

1. `replace_exact`
2. `read_range`
3. `diff_preview`
4. `status_guard`

The remaining tools stay documented as planned boundaries until implemented.
