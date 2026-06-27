# Tool Spec

Initial contextpatch tools are intentionally narrow.

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
