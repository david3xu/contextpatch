# Implementation Roadmap

`contextpatch` should ship in small stages. Each stage must preserve the product thesis: AI coding agents get guarded edit primitives, not broad filesystem write power.

## Stage 1: useful safe-edit MVP

Stage 1 should finish the smallest serious product surface.

| Tool | Why it ships first |
| --- | --- |
| `read_range` | Safe bounded inspection before editing |
| `replace_exact` | Core anchored edit primitive |
| `diff_preview` | Reviewability before mutation |
| `status_guard` | Repository state visibility and edit gating |
| `write_new_file` | Safe create-only file creation |

Stage 1 is complete: these tools work through the core crate, CLI, and MCP server, with success/refusal tests and protocol-facing schemas.

## Stage 1 implementation order

1. `replace_exact` core behavior - implemented
2. `replace-exact` CLI command - implemented
3. `read_range` core behavior and `read-range` CLI command - implemented
4. `write_new_file` core behavior and `write-new-file` CLI command - implemented
5. `diff_preview` core behavior and `diff-preview` CLI command - implemented
6. `status_guard` core behavior and `status-guard` CLI command - implemented
7. Server tool schemas for implemented Stage 1 tools - implemented for `read_range`, `diff_preview`, `replace_exact`, `write_new_file`, and `status_guard`
8. Server transport for implemented Stage 1 tools - implemented for stdio MCP

## Stage 1 refusal tests

Stage 1 must test these refusals:

1. Empty old text for `replace_exact`
2. Zero-match replacement
3. Multi-match replacement
4. Path outside repository root
5. Create-only write where destination already exists
6. Dirty repository when a clean guard is requested

## Stage 2: advanced edit operations

| Tool | Reason for Stage 2 |
| --- | --- |
| `insert_at_anchor` | Useful convenience built on exact-anchor semantics |
| `apply_patch` | More complex atomicity and partial-apply behavior |
| `move_tracked` | Needs careful Git tracked/untracked behavior |
| `delete_guarded` | Higher-risk destructive primitive requiring hash confirmation |

Stage 2 should not start until Stage 1 has stable tests and docs.

## Stage 2A: Claude Desktop readiness and validation

Claude Desktop can continue real project work only if it can discover capabilities and run bounded validation without falling back to a broad shell. Stage 2A adds that bridge while preserving the safety contract.

| Tool | Reason for Stage 2A |
| --- | --- |
| `capability_manifest` | Let clients know exactly which file and process capabilities exist |
| `preflight_health` | Report repo cleanliness and expected validation-tool availability |
| `run_guarded_command` | Run repo-confined allowlisted validation commands without shell access |

Stage 2A is implemented for the MCP server. It intentionally does not add automatic commits, destructive Git operations, or arbitrary shell command strings.

## Always out of scope by default

- Generic `write_file`
- Recursive directory writes
- Unrestricted delete
- Unrestricted shell execution
- Automatic Git commits, resets, checkouts, or stashes
