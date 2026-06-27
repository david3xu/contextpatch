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
| `read_command_log` | Let clients retrieve captured command output without forcing large JSON-RPC responses into the first call |
| `validation_profile_run` | Collapse common multi-command validation sequences into one auditable MCP call |
| `git_commit_exact` | Allow a narrow local commit checkpoint after exact-path validation |
| `git_remote_check` | Fetch exactly one explicit remote branch and report local/remote divergence without source edits |
| `git_push_exact` | Publish the exact current commit only after branch, clean-worktree, expected-HEAD, remote-divergence, and confirmation guards |

Stage 2A is implemented for the MCP server. It intentionally does not add arbitrary shell command strings, destructive Git operations, or broad automatic commits. Git mutation remains split by risk boundary: `git_commit_exact` is local only, `git_remote_check` is explicit single-branch fetch/report only, and `git_push_exact` is exact current-HEAD push only with no force or multi-ref publishing.

## Stage 2B: latency and workflow compression

These tools should reduce MCP round trips and process-spawn overhead while keeping the same guardrails:

| Tool | Reason |
| --- | --- |
| `repo_snapshot` | One call for branch, HEAD, dirty paths, and short status |
| `read_many_ranges` | Batch related bounded reads instead of repeated JSON-RPC calls |
| `grep_profile` | Run named drift-search bundles without many separate `rg` calls |
| `list_files` | Native repo-confined file listing for simple `rg --files` use cases |

Stage 2B tools should prefer native Rust operations where possible and should return compact summaries plus log ids for large details.

Latency instrumentation is a Stage 2B must-have, not a cosmetic metric. Each MCP tool should be able to report timing and size metadata precise enough to separate Claude/Desktop round-trip overhead from server-side work. At minimum, record:

1. `request_received_to_dispatch_ms`
2. `argument_validation_ms`
3. `allowlist_validation_ms`
4. `child_spawn_ms`
5. `child_runtime_ms`
6. `stdout_stderr_drain_ms`
7. `redaction_truncation_ms`
8. `log_write_ms`
9. `response_bytes`
10. `total_tool_ms`

The goal is to stop guessing whether latency comes from JSON-RPC handling, process spawn, child runtime, output draining, redaction, log writes, or response size. Instrument first, then optimize based on measured p50/p95/p99 behavior across long Claude Desktop sessions.

## Always out of scope by default

- Generic `write_file`
- Recursive directory writes
- Unrestricted delete
- Unrestricted shell execution
- Broad or automatic Git commits
- Git fetch/push, resets, checkouts, or stashes
