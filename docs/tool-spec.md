# Tool Spec

Initial contextpatch tools are intentionally narrow.

This is deliberate: `contextpatch` is a safe patch layer for AI coding agents, not a general filesystem toolbox.

| Tool | Writes? | Guard |
| --- | --- | --- |
| `project_execute` | Depends on selected action | Project-surface wrapper; exactly one existing guarded action per call |
| `capability_manifest` | No | Reports exact supported capabilities and unsupported boundaries |
| `preflight_health` | No | Repository/tool readiness summary |
| `read_range` | No | Bounded path and line range |
| `read_write_receipts` | No | Bounded recent receipt query under the repository-specific scratch root |
| `diff_preview` | No | Proposed edit input |
| `replace_exact` | Yes | Old text must match exactly once; optional complete-file SHA-256 guard |
| `bulk_replace_exact` | Yes | Validates the full batch before writing; several hunks may target one file and land in its single atomic write; per-file apply may leave a prefix after interruption or apply failure |
| `move_tracked` | Yes | Clean tracked regular source, absent destination, clean index, dry-run and confirmation |
| `status_guard` | No | Repository status inspection |
| `write_new_file` | Yes | Destination must not exist |
| `write_new_file_base64` | Yes | Destination must not exist; base64 decoded bytes with optional expected size |
| `write_existing_file_exact_hash` | Yes | Existing regular file, exact current SHA-256, dry-run default, explicit confirmation |
| `file_info` | No | Repo-root-confined metadata for one path or up to 64 unique paths, including mode, symlink status, SHA-256 for files, and UTF-8 line count |
| `set_file_executable` | Executable bits only | Existing regular file, exact current SHA-256 and mode, dry-run default, explicit confirmation |
| `list_directory` | No | Bounded deterministic directory listing with optional depth-limited recursion that reports symlink entries without ever following them |
| `read_file_bytes` | No | Bounded byte range from a regular file as hex or base64 |
| `artifact_write_text` | Sidecar artifact only | Writes outside repo under fixed artifact root; create-only, no repo mutation |
| `artifact_write_base64` | Sidecar artifact only | Binary sidecar artifact under fixed artifact root; create-only, size/byte-count guards |
| `artifact_delete_exact` | Sidecar artifact only | Exact regular sidecar file, digest-reporting dry run, matching SHA-256, explicit confirmation |
| `artifact_python_run` | No source edits | Runs a Python script under the fixed artifact root with no shell and bounded logs |
| `task_image_python_run` | Docker image/cache only | Plans a task-image Python run; confirmed execution starts asynchronously and returns a pollable log id |
| `bulk_write_new_files_base64` | Yes | Bounded multi-file create-only import from base64 entries |
| `create_directory` | Yes | Destination must not exist; optional explicit parent creation inside repo root |
| `run_guarded_command` | No source edits | Allowlisted validation command with repo-root-confined arguments and no shell interposed by this server; not a sandbox |
| `fixture_generator_run` | Declared fixture outputs only | Dry-run/confirmation-gated repo-relative Python generator with changed-path verification |
| `base_image_check_run` | No source edits | Exact `references/check-base-image.sh` validation workflow, optionally with the exact `task` arg; no arbitrary shell |
| `image_cleanliness_check_run` | No source edits | Narrow Docker image `find / -name <filename>` check; dry-run default and exact confirmation |
| `docker_image_inspect` | No source edits | Narrow `docker image inspect <image>` workflow; dry-run default and exact confirmation |
| `fixture_manifest_verify` | No | Exact fixture file set and SHA-256 digest verification; mismatches are refusals |
| `fixture_manifest_refresh` | Manifest file only | Regenerates fixture manifest from declared files/prefixes with dry-run, confirmation, and existing-manifest hash guard |
| `read_command_log` | No | Reads captured command logs and asynchronous lifecycle state by opaque id |
| `harbor_run_start` | Harbor job artifacts | Starts one typed Harbor run asynchronously and exposes pollable structured evidence through an opaque log id |
| `validation_profile_run` | No source edits | Starts predefined allowlisted validation command sequences asynchronously |
| `setup_profile_run` | External setup command | Dry-run default, clean-worktree and confirmation gates, profile-derived command plan, typed params only, no caller-supplied raw commands |
| `native_build_run` | External build/test command | Dry-run default, typed action params, source-status unchanged after execution, no raw native commands |
| `native_device_run` | External device command | Dry-run default, typed action params, device-state confirmation gate, no raw `xcrun` or `adb` commands |
| `git_commit_exact` | Git index + one local commit | Exact full dirty-path set, dry-run default, explicit confirmation, never pushes |
| `git_stage_exact` | Git index only | Explicit dirty paths, clean-index gate, dry-run default, no commit |
| `git_staged_scope_check` | No | Read-only staged-path policy check against allowed exact paths/prefixes and optional required paths |
| `git_commit_scoped` | Git index + one local commit | Explicit dirty subset, clean-index gate, preserves unrelated dirty paths, never pushes |
| `git_commit_prefix` | Git index + one local commit | Explicit prefixes expand to exact dirty paths, clean-index gate, dry-run default, never pushes |
| `git_restore_exact` | Git worktree/index restore | Explicit dirty tracked paths only, dry-run default, explicit confirmation, never resets the whole worktree |
| `delete_untracked_exact` | Git worktree cleanup | Explicit untracked regular files only, dry-run default, explicit confirmation, no broad clean |
| `delete_generated_prefix` | Git worktree cleanup | Explicit prefixes expand only to ignored/untracked files and empty dirs, dry-run default, no broad clean |
| `git_remote_list` | No | Read-only parsed `git remote -v` |
| `git_remote_check` | Remote-tracking refs only | Fetches one explicit remote branch and reports HEAD/remote divergence without source edits |
| `git_branch_prepare` | Remote-tracking ref + local branch/worktree | Clean worktree, exact remote base fetch, validated branch, optional confirmed reset, required-file gates |
| `git_merge_readiness` | Optional remote-tracking ref update only | Validated refs, optional single-branch fetch, merge-base/ahead/diff-name analysis, no merge |
| `git_push_exact` | Remote branch update | Clean worktree, exact HEAD, no remote-ahead divergence, explicit confirmation, no force |
| `github_pr_run` | GitHub mutation for PR creation only | `gh`-backed auth/PR/check/run/job-log evidence plus confirmation-gated PR creation |
| `github_fork_prepare` | GitHub fork mutation | `gh repo fork` plan by default, explicit confirmation for mutation |
| `delete_guarded` | Yes | Clean tracked regular file, exact current SHA-256, dry-run and confirmation |

## Naming

The public tool names use snake_case because they are protocol-facing. The CLI uses kebab-case commands.

## Tool annotations

Every server-advertised MCP tool must include an `annotations` object in `tools/list` so clients can interpret tool intent. The server sets `destructiveHint: false` for all tools, and derives `readOnlyHint` and `idempotentHint` together from the read-only classification.

`openWorldHint` is per action and is derived from one centralized classification in
`crates/server/src/tools/schema/authority.rs`. An action is advertised open-world when it either
contacts a remote system itself, or starts repository-controlled code that inherits this server's
environment and with it the server user's network capability. That covers Git fetch and push,
GitHub actions, dependency-capable builds and scripts, fixture generators, validation profiles,
Harbor, setup profiles, native builds, and artifact Python. The `project_execute` wrapper is
open-world because it dispatches every inner action.

Actions stay closed-world when they remain on the local host, and when execution happens under the
documented container isolation with networking disabled: `task_image_python_run` and
`image_cleanliness_check_run`. Note that `git_merge_readiness` is both read-only and open-world,
because it may fetch from a remote in order to report state.

These hints describe reach, not containment. Executable-name allowlisting is entry-point and
argument policy, not sandboxing. See `docs/execution-threat-model.md`.

## Transport and concurrent calls

Every ID-bearing `tools/call` request executes through a bounded 16-worker pool so one slow validator, Git operation, or filesystem call does not block stdin processing or later independent calls. Responses can arrive out of request order; clients must correlate every response with its request by JSON-RPC `id`. Response writes remain serialized so each stdio line is one complete JSON-RPC document. Stdin is read through a bounded queue so a worker that detects broken stdout can wake the dispatcher and terminate the server even if the client leaves stdin open. Initialization, tool listing, notifications, malformed requests, and unknown methods remain inline.

Guarded repository file inspection, directory listing, creation, replacement, and mode changes currently require Unix descriptor-relative path support. Other platforms must refuse these operations rather than reopen path strings after validation.

Confirmed `task_image_python_run`, `harbor_run_start`, and `validation_profile_run` calls start background work and immediately return `status: "running"` plus a stable `log_id`. The three workflows share a two-job cap. Poll with `read_command_log` on the same running server; polling never starts work again. A log that was still active when its owning server stopped must report `unknown`.

These annotations are a client-permission hint only. They must not remove internal contextpatch safety gates: mutation-capable tools still default to dry-run where designed, still require exact confirmation strings for execution where designed, and still enforce repository-root, hash, path, clean-index/worktree, timeout, and refusal policies.

Claude Desktop approval prompts are controlled separately from MCP annotations. `contextpatch configure-claude-desktop` validates detected native and `.exe` ContextPatch commands in the ordinary `claude_desktop_config.json` `mcpServers` map, preserves those entries, unrelated data, user-authored policies, and unrelated command arguments, and defaults each targeted entry to `--tool-surface project`. Use `--tool-surface full` for the direct-tool rollback. The configurator removes only the exact legacy ContextPatch wildcard `toolPolicy: {"*":"allow"}` from targeted entries and does not consult or write `Claude-3p/configLibrary`. Changed configs receive an exact backup, preserved permissions, cooperative locking, a stale-read check, and atomic replacement; `--dry-run` leaves the config unchanged and creates no backup, and repeated runs are idempotent. The local ContextPatch MCP connection requires no authentication. Project mode reduces the persistent approval surface to one stable `project_execute` identity per configured project, but Claude still owns the approval decision and local metadata cannot silently preauthorize it.

## Tool contracts

### `project_execute`

The sole public MCP tool when the server starts with `--tool-surface project`. It discovers or executes
one existing ContextPatch action within the server's fixed workspace boundary.

Required inputs:

- `action`: an internal action name, or the reserved value `describe`

Optional inputs:

- `arguments`: the original JSON object for the selected action
- `repository`: a normalized workspace-relative path to an exact descendant Git worktree root; omit
  it to use the configured `--repo-root` unchanged

Discovery:

- `{"action":"describe"}` returns build provenance and the sorted internal action names.
- `{"action":"describe","arguments":{"name":"read_range"}}` returns that action's exact schema and annotations.
- A `repository` supplied with `describe` is validated against the workspace boundary before discovery
  is returned.

Rules:

- Execute exactly one action per call; batching and recursive `project_execute` dispatch are refused.
- Keep the configured `--repo-root` immutable. `repository` accepts no absolute path, traversal,
  malformed separator, control character, `.git` component, symlink component, non-directory,
  non-Git directory, or worktree subdirectory.
- Require the selected directory to equal `git rev-parse --show-toplevel`; merely being inside a Git
  repository is insufficient.
- Do not accept an arbitrary repository root or server identity. The selector grants authority only
  to an exact descendant worktree beneath the configured workspace.
- Keep the public wrapper name, description, input schema, and annotations stable as internal actions are added.
- Resolve the effective repository root and internal action before selecting the action's deadline,
  repository mutation lock, handler, receipt/log identity, refusal text, and timeout recovery guidance.
- Reuse the existing action handlers and all repository, path, hash, Git-state, dry-run, confirmation,
  process, timeout, and receipt policy without weakening or duplicating it.
- Hand every repository-rooted action the selection's retained directory authority rather than its name.
  Git actions and the repository-file actions — content reads, byte ranges, metadata, directory listing,
  status, diff preview, exact replacement, bulk replacement, text and base64 creation, bulk creation,
  exact-hash overwrite, executable mode, and directory creation — must all act on the directory that was
  validated, never on whatever the selector's name resolves to later. Sidecar artifact actions are excluded
  and continue to resolve a sidecar root.
- Refuse a symlink at every path component, including the leaf, under both a configured root and a
  selection. Metadata and listing may still describe a symlink without following it.
- Reject direct calls to hidden internal tools while project mode is active.
- Advertise `readOnlyHint: false` because the wrapper can route mutation-capable actions.

The wrapper-level `repository` is distinct from
`github_pr_run.arguments.repository`, which is an optional GitHub `OWNER/REPO` target after the local
worktree has already been selected.

### `capability_manifest`

Reports the server's capability contract in machine-readable JSON text.

Required inputs: none.

Optional inputs:

- `names_only`: return the active public tool names and sorted internal action names. In project mode the
  action names include `describe`, matching what the wrapper will actually dispatch; in full mode they do
  not, because the wrapper is not advertised there and naming it would name something uncallable.
- `section`: return one of `build`, `tool_surface`, `scratch`, `deadlines_seconds`, `mutation_coordination`,
  `write_receipts`, `file_tools`, `git_workflows`, `github_workflows`, `process_execution`,
  `setup_profiles`, `native_build`, or `native_device`

Rules:

- Must report file tools and process-execution availability honestly.
- Must report the active tool surface, its public names, and the internal action names.
- Projected responses must retain `build.git_sha` and `build.git_dirty_fingerprint`; a section
  response must include both `"section": "<name>"` and a property named for the requested section.
- Must report a dirty-tree fingerprint alongside `git_dirty`. A matching `git_sha` does not pin a
  build made from a dirty worktree, because the sha still matches `HEAD` while the binary carries
  uncommitted changes. The fingerprint must be `clean` for a clean worktree, `unknown` when
  provenance could not be determined, and must never contain file content.
- Unknown sections must refuse and list the valid choices.
- The no-argument response must retain the complete legacy manifest shape.
- Must identify the configured repository root.
- Must state unsupported operations, including arbitrary shell and destructive Git mutations.
- Must distinguish the narrow local `git_commit_exact`/`git_commit_scoped` checkpoints and guarded `git_remote_check`/`git_push_exact` workflow from unsupported broad Git/destructive workflows.
- Must report `git_branch_prepare` honestly when available and distinguish it from broad checkout/rebase/reset authority.
- Must report `git_merge_readiness` honestly when available and distinguish it from generic merge authority.
- Must report setup profiles honestly when available and distinguish declarative profile planning from generic package-manager or shell authority.
- Must include compact examples for typed setup, native build, and native device actions so clients can choose correct params without raw command knowledge.
- Must report build Git SHA, dirty state, build timestamp, and profile so clients can detect a stale installed binary.
- Must report the stable repository-specific scratch root and `{scratch}` argument token.
- Must report direct read, write, Git, and GitHub reply deadlines, the active-worker ceiling, and state that expiry leaves the operation outcome unknown.
- Must report the shorter hard Git subprocess timeout and that Unix process-group termination also stops blocked descendants before the outer Git reply deadline.
- Must report cooperative repository mutation locking honestly and distinguish it from universal exclusion of external writers.
- Must report the receipt outcome vocabulary and its recovery meaning.
- Must not mutate repository state.

### `preflight_health`

Reports whether the repository and local validation tools are ready for agent work.

Required inputs: none.

Optional inputs:

- `response_mode`: `full` (default), `compact`, or `minimal`

Rules:

- Must report repository cleanliness using the same Git guard semantics as `status_guard`.
- Must bound dirty-status evidence in every mode. `full` may sample at most 100 entries and 64 KiB,
  `compact` at most 20 entries and 12 KiB, and `minimal` must return counts without path samples.
- Must report the total change count and whether the returned path sample was truncated.
- `full` must retain detailed probes and resolved executable paths, `compact` must project probe
  details to availability booleans, and `minimal` must return availability counts and unavailable names.
- Must report whether guarded process execution is available.
- Must report local availability of expected validation tools without treating missing optional tools as server failure.
- Must include every executable that appears in a shipped validation profile or documented guarded validation workflow, including `python3`, `pytest`, `harbor`, and the `bash` base-image script path, so clients can detect host gaps before launching a long profile.
- Must make unavailable validation executables explicit. For example, when `harbor` is missing or cannot be launched in the host environment, clients should skip `dynamo-harbor-task` profile execution and use task-local verifier/oracle comparisons only as an environment-limited fallback, not as proof that Harbor scoring succeeded.
- Must report setup-profile prerequisites without running setup commands.
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

### `read_write_receipts`

Returns recent durable mutation receipts, newest first.

Optional inputs:

- `interrupted_only`: defaults to `false`; when true, returns only attempts with a persisted begin record and no settle record
- `limit`: defaults to `25`, maximum `100`

Rules:

- The journal must live under the stable repository-specific scratch root, outside the repository.
- Each receipt must identify the tool and include the available path, before/after SHA-256 values, before/after Git HEAD values, outcome, and timestamps.
- A mutation must persist its begin record before running and its settle record after collecting resulting state.
- Settled outcomes are `applied` when the operation returned success, `refused` when it failed and observed state stayed unchanged, and `unknown` when it failed after observed state changed or could not be read. `interrupted` means no settle record exists.
- Journal reads, appends, and rotation must use cross-process locking.
- Rotation must preserve interrupted entries.
- Bounded multi-file mutations must reserve capacity before the first write and prevent rotation
  between their per-file records, preserving a settled prefix next to a later interrupted entry.
- An interrupted receipt means the transport did not observe settlement; callers must compare current file or Git state before retrying.
- Receipt coverage is explicit, not universal. The current implementation records `replace_exact`,
  each `bulk_replace_exact` apply attempt, `write_existing_file_exact_hash`,
  `delete_untracked_exact`, `git_commit_exact`, `git_commit_scoped`, and `git_commit_prefix`.
- The tool must not mutate repository state.

### `diff_preview`

Returns a unified diff for a proposed edit without writing.

Required inputs:

- `path`
- `old`
- `new`

Optional inputs:

- `expected_sha256`: lowercase SHA-256 of the complete current file

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
- When `expected_sha256` is provided, validate it and refuse before mutation if the current complete-file digest differs.
- Write atomically.
- Return the changed byte range or a concise edit summary, together with the SHA-256 digest of the
  bytes written. Reporting the digest is what lets a caller chain it as the next `expected_sha256`
  without a separate read, which is the difference between a guard that is cheap enough to use on every
  write and one that is quietly skipped.

### `bulk_replace_exact`

Validates many exact replacements together, then applies them with one atomic write per file. Several
entries may target the same file; those are hunks, resolved against a single snapshot of that file and
carried by its single write.

Exists because a single logical change often spans several files. Registering one tool in this
repository touches six: name, handler, dispatch arm, schema, module export, capability entry. Six
sequential calls is six chances to lose the answer to a transport interruption, and losing one midway
leaves a half-wired change that still compiles.

Required inputs:

- `entries`, a non-empty array of `{ path, old, new }` objects, each optionally carrying
  `expected_sha256`. More than one entry may name the same path; each such entry is one hunk within
  that file.

Rules:

- Refuse if `entries` is empty or exceeds 64 entries. The ceiling counts entries rather than files, and
  is deliberately lower than the bulk import limit: a batch of edits is one logical change, and a caller
  sending hundreds has more likely built the list by mistake than by intent.
- Group entries by normalized path. Resolve every hunk for one file against a single captured snapshot
  of that file, never against text an earlier hunk already rewrote, so a hunk's validity never depends
  on the order its siblings would have been applied in.
- Refuse if two *different* paths identify the same filesystem file, including case aliases and hard
  links. Grouping is by path, so an alias would otherwise produce two independent snapshots of the same
  bytes and the second write would silently discard the first. Repeating one path is a hunk; spelling
  one file two ways is an error.
- Refuse two hunks in one file that resolve to the same byte range. That is a duplicate entry rather
  than two edits, and applying either silently would discard the other.
- Refuse two hunks in one file whose byte ranges intersect. Both entries claim the same bytes, so no
  ordering satisfies them.
- Refuse two entries for one file that carry different `expected_sha256` values. One file cannot have
  two current digests, and letting whichever entry is checked first decide would make the guard depend
  on submission order.
- Validate every entry, and compute each file's resulting content, before writing anything. Each entry
  is subject to the same rules as `replace_exact`, including the exactly-once match requirement and the
  optional complete-file digest guard.
- If any entry fails validation, refuse before the first write and leave every target unchanged. The
  refusal names the offending entry by its position in the caller's list, not its position after
  grouping.
- Journal a `refused` receipt for every named target when validation fails, before any mutation could
  have occurred. Without it a refused batch is indistinguishable from a batch that was never attempted,
  which is the one gap the journal is supposed to close. One receipt per file, not per hunk, because one
  file would have received one write. Each receipt records the digest before and after and reports
  `unknown` rather than `refused` when they differ, since an external writer during validation means the
  tool cannot claim the file is untouched. A journal problem is appended to the refusal rather than
  replacing it, so it never masks the validation error that caused it.
- Order plans by normalized path, so grouping, lock acquisition, and refusal reporting stay
  deterministic regardless of the order entries were submitted in.
- Apply every hunk for one file in that file's single atomic write, so no file is observed partially
  edited.
- Return one result per submitted entry, in submission order, each naming its entry index and the byte
  range that entry resolved to, together with the number of files written. `bytes_written` is the size
  of the single write that carried every hunk for that file, and `sha256` is that file's post-write
  digest, so every hunk of one file reports the same digest.
- Before applying each plan, re-read the target under its mutation lock and compare the exact bytes
  captured during validation. Refuse that entry if the file changed, even when no caller-supplied
  `expected_sha256` was provided.
- Bind every plan to the canonical repository root used during validation. Refuse applying it under a
  different root even when that root contains a hard link to the same target inode.
- Each file replacement is atomic, but the batch is not a cross-file transaction and is not rolled
  back. An interruption or apply-phase failure can leave an applied prefix of files, and the current
  file's outcome can be unknown if receipt settlement or transport fails.
- Journal one apply attempt per file. Entries that share a file share its receipt, because they share
  its write. After an interrupted or failed apply phase, inspect `read_write_receipts` and current file
  state before retrying.
- Reserve receipt-journal capacity for the bounded batch before the first write and suppress rotation
  between entries, so a later interrupted receipt cannot evict the settled prefix.
- File locks use filesystem identity in one user-wide ContextPatch lock namespace, so hard-link,
  case, configured-workspace, and descendant-repository views cannot acquire divergent locks for the
  same target. Advisory locks coordinate ContextPatch writers only; external programs that ignore
  them can still race the final comparison and rename.

### `move_tracked`

Moves or renames a file with repository guardrails.

Required inputs:

- `from`
- `to`

Optional inputs:

- `dry_run`: defaults to `true`
- `confirm`: required literal `move tracked file` when `dry_run` is `false`

Rules:

- Require normalized repository-relative paths under the configured Git worktree root.
- Require the source to be a clean tracked regular file with no symlink path components.
- Require an absent, untracked destination whose parent already exists inside the repository and has no symlink path components.
- Require a clean Git index so the staged rename cannot mix with pre-existing staged work.
- Refuse mutation without the exact confirmation literal.
- Use `git mv`, then verify the source is absent, the destination is tracked, and its SHA-256 matches the source.

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

Creates a new UTF-8 text file only when the destination does not exist.

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
- Refuse symlinked or non-directory parent components instead of following them.
- Refuse missing parent directories.
- Write atomically.
- Report the SHA-256 digest of the created content, so the new file can be guarded on the next write
  without reading it back.

### `write_new_file_base64`

Creates a new binary file from base64 only when the destination does not exist.

Required inputs:

- `path`
- `content_base64`

Optional inputs:

- `expected_bytes`: decoded byte-count guard

Rules:

- Refuse if the file already exists.
- Refuse parent traversal outside the repository root.
- Refuse missing parent directories.
- Refuse invalid base64 or decoded content larger than 20 MiB.
- When `expected_bytes` is provided, refuse if the decoded byte count does not match.
- Write atomically.
- Report the SHA-256 digest of the created content, so the new file can be guarded on the next write
  without reading it back.

### `file_info`

Reports bounded metadata for one repository path or a batch of paths.

Exactly one input form is required:

- `path`: one repository-relative path
- `paths`: 1 to 64 unique repository-relative paths

Rules:

- Every intermediate component must be a non-symlinked directory inside the repository root.
- A single-path response retains the original object shape; a batch response returns `path_count` and an ordered `entries` array.
- Each entry must identify whether the path exists, whether it is a symlink, its normalized repository-relative path, its type, and its Unix mode/executable state when available.
- Existing regular files must include byte length and lowercase SHA-256 digest.
- File size, digest, and UTF-8 line count must come from one fixed-extent streamed snapshot of the
  already-open file. The tool must retain only a fixed read buffer, refuse if content metadata or
  path identity changes during inspection, and must not allocate the complete file.
- A final symlink may be described, including its link target and whether that target resolves inside the repository, but it must not be followed or hashed.
- Existing UTF-8 regular files should include `line_count`; binary or invalid UTF-8 files must not be treated as text.
- Existing directories must be reported as directories without recursively listing contents.
- Duplicate paths, an empty batch, more than 64 paths, or supplying both input forms must be refused.
- The tool must not mutate repository state.

### `set_file_executable`

Plans or changes only the executable permission bits of one existing repository file.

Required inputs:

- `path`
- `executable`: enable or clear owner, group, and other executable bits

Optional inputs:

- `expected_sha256`: current lowercase SHA-256; required for execution
- `expected_mode`: current three- or four-digit octal mode; required for execution and normalized to four digits before comparison
- `dry_run`: defaults to `true`
- `confirm`: required literal `set file executable` when `dry_run` is `false`

Rules:

- The target and every path component must be non-symlinked, repository-confined, and an existing regular file with exactly one hard link.
- The operation is supported only on Unix.
- A dry run must report the current SHA-256, current mode, proposed mode, and whether a change is needed.
- Execution must hold the target mutation lock and revalidate both the SHA-256 and mode after acquiring it.
- Enabling executable status must OR mode `0111`; disabling it must clear only mode `0111`, preserving all other permission bits.
- The file content must remain byte-identical, and the resulting mode must be verified after mutation.
- An already-satisfied request may succeed as a no-op only after the same execution guards are met.

### `list_directory`

Lists a repository directory with compact entry metadata and optional bounded recursion.

Optional inputs:

- `path`: defaults to the configured repository root
- `include_hidden`: defaults to `false`
- `recursive`: defaults to `false`
- `max_depth`: from 1 to 16; defaults to 4 and is reported as 1 when recursion is disabled
- `max_entries`: from 1 to 2000; defaults to 2000

Rules:

- The path must resolve inside the repository root and must be an existing directory.
- By default the tool lists only direct children. Recursive mode traverses ordinary directories breadth-first only while their depth is below `max_depth`.
- Entries must include name, normalized path, depth, type, symlink flag, and byte size when available.
- The starting directory and traversed directories must not be symlinks; symlink entries are reported but never followed.
- Hidden entries are omitted unless `include_hidden` is true.
- Entries should be sorted by path for deterministic output.
- The response must report `entry_count`, the effective bounds, and `truncated: true` when `max_entries` stops traversal.
- Enumeration must retain at most the lexically smallest remaining entries plus one truncation
  sentinel per visited directory before reading entry metadata, so large directories do not turn a
  small `max_entries` request into unbounded metadata work.
- The tool must not mutate repository state.

### `read_file_bytes`

Reads a bounded byte range from an existing regular file.

Required inputs:

- `path`

Optional inputs:

- `offset`: byte offset, defaults to `0`
- `max_bytes`: byte count to return, defaults to a bounded value
- `encoding`: `hex` or `base64`, defaults to `hex`

Rules:

- The target and every intermediate component must be non-symlinked, repository-confined, and an existing regular file or directory as appropriate.
- The response must include total file size, returned byte count, offset, encoding, returned data, and full-file SHA-256.
- The tool must refuse offsets past end-of-file and excessive byte counts.
- Range bytes, total size, and full-file SHA-256 must come from one fixed-extent streamed snapshot of
  the already-open file. Range reads must use positional I/O and retain memory proportional to
  `max_bytes`; digesting must use a fixed-size buffer rather than buffering the complete file.
  Computing the digest still reads the complete initial extent.
- The tool must refuse if content metadata or path identity changes during inspection. Appends after
  the initial snapshot must not extend the scan indefinitely.
- The tool must not decode bytes as text and must not mutate repository state.

### `artifact_write_text`

Creates a new UTF-8 text artifact outside the repository under a fixed `contextpatch` artifact root.

Required inputs:

- `path`
- `content`

Optional inputs:

- `parents`: defaults to `false`; when `true`, create missing parent directories under the artifact root.

Rules:

- Refuse absolute paths, `..`, `.` components, and NUL bytes.
- Refuse if the artifact already exists.
- Refuse missing parent directories unless `parents` is true.
- Write atomically through the same create-only byte path as repository file creation.
- Return the artifact root and created path, and report `repo_mutation: false`.

### `artifact_write_base64`

Creates a new binary artifact outside the repository under the same fixed artifact root.

Required inputs:

- `path`
- `content_base64`

Optional inputs:

- `expected_bytes`: decoded byte-count guard
- `parents`: defaults to `false`; when `true`, create missing parent directories under the artifact root.

Rules:

- Apply all `artifact_write_text` path and create-only rules.
- Refuse invalid base64 or decoded content larger than 20 MiB.
- When `expected_bytes` is provided, refuse if the decoded byte count does not match.

### `artifact_delete_exact`

Deletes one exact regular file under the fixed contextpatch artifact root without touching the repository or parent directories.

Required inputs:

- `path`: normalized artifact-root-relative path

Optional inputs:

- `expected_sha256`: current lowercase SHA-256; required for mutation
- `dry_run`: defaults to `true`
- `confirm`: must equal `delete artifact exact` when `dry_run` is false

Rules:

- Refuse absolute, non-normalized, traversal, missing, symlinked, and non-regular paths.
- Hold the target mutation lock while inspecting and deleting the file.
- A dry run must report the current SHA-256, byte size, artifact root, and required confirmation without deleting.
- Mutation must require the current SHA-256 and exact confirmation, then re-read the digest immediately before deletion.
- Delete only the named file, retain all parent directories, verify the file is absent, and report `repo_mutation: false`.

### `artifact_python_run`

Runs a Python script that already exists under the fixed contextpatch artifact root.

Required inputs:

- `script`: artifact-root-relative script path

Optional inputs:

- `args`: argument array
- `program`: `python3` or `python`, defaults to `python3`
- `cwd`: repository-relative working directory, defaults to repository root
- `timeout_secs`: from 1 to 600

Rules:

- The script path must resolve under the fixed artifact root, not under the repository.
- The working directory must resolve inside the repository root.
- The tool must invoke Python directly with an argument array and no shell.
- Args must be bounded strings and must not contain NUL bytes.
- Output must be redacted, truncated, logged by opaque id, and readable through `read_command_log`.
- The tool must not mutate repository state by itself and must not accept caller-supplied executable paths or environment overrides.

### `task_image_python_run`

Builds `task/environment/Dockerfile`, then runs one existing repository-relative Python script inside the resulting task image.

Required inputs:

- `script`: normalized repository-relative `.py` file

Optional inputs:

- `program`: `python3` or `python`, defaults to `python3`
- `args`: up to 32 strings of at most 4096 bytes each
- `timeout_secs`: script timeout from 1 to 600; defaults to 120
- `build_timeout_secs`: image-build timeout from 1 to 1800; defaults to 600
- `dry_run`: defaults to `true`
- `confirm`: required literal `run task image python` when `dry_run` is `false`

Rules:

- The repository must contain non-symlinked `task/environment/`, `task/environment/Dockerfile`, and script paths.
- The image build must use `task/environment` as its context and the fixed task Dockerfile; callers cannot supply Docker arguments, image names, mounts, entrypoints, or environment variables. The core-owned execution plan must expose read-only inspection only, so callers cannot alter its repository root, Docker arguments, names, or timeouts after validation. Each job must receive a unique execution-image tag and container name so concurrent jobs cannot run an image published by another build; a separate repository-specific cache tag may be shared.
- A dry run must return the exact Docker build and run plans without invoking Docker.
- Runtime must mount the repository read-only at `/workspace`, disable networking, drop all capabilities, set `no-new-privileges`, cap PIDs, use a read-only container root, provide a bounded `/tmp` tmpfs, and invoke Python directly without a shell.
- Build and runtime have separate timeouts. A failed or timed-out build must prevent the script run. A runtime timeout, indeterminate run-wrapper failure, or Docker exit code 125 must force-remove the uniquely named container after terminating the Docker client. The unique execution tag must be removed after every build attempt that may have created it, and terminal evidence must report container and execution-tag cleanup separately.
- Confirmed execution must return `status: "running"` and an opaque `log_id` immediately instead of holding the MCP response open for the build and run.
- Build and script output must be redacted and truncated to bounded fields. Terminal results must be saved under the returned `log_id` and read through `read_command_log`.
- The tool grants typed task-image execution only; it must not expose generic Docker or package installation.

### `bulk_write_new_files_base64`

Creates many new repository files from base64 entries in one bounded fixture import.

Required inputs:

- `entries`: array of objects with `path`, `content_base64`, and optional `expected_bytes`

Optional inputs:

- `parents`: defaults to `false`; when `true`, create missing parent directories inside the repository root.

Rules:

- Refuse an empty entry list or more than 500 files.
- Refuse duplicate paths, absolute paths, traversal, NUL bytes, and existing destinations.
- Refuse symlinked or non-directory parent components. The operation is supported only on Unix, where parent traversal and optional parent creation must remain descriptor-relative and no-follow.
- Refuse missing parent directories unless `parents` is true.
- Refuse invalid base64, per-entry byte-count mismatches, or total decoded content larger than 20 MiB.
- Write each file through the same create-only root guard as `write_new_file_base64`.

### `create_directory`

Creates a new directory only when the destination does not exist.

Required inputs:

- `path`

Optional inputs:

- `parents`: defaults to `false`; when `true`, create missing parent directories inside the repository root.

CLI shape:

```bash
contextpatch create-directory <path> [--parents]
```

The CLI treats the current working directory as the repository root guard.

Rules:

- Refuse if the target directory already exists.
- Refuse if any existing path component is a file.
- Refuse parent traversal outside the repository root.
- Refuse missing parent directories unless `parents` is true.
- With `parents: true`, create only the explicit path components needed for the requested directory path.

### `run_guarded_command`

Runs a bounded validation-oriented command without invoking a shell.

Required inputs:

- `program`: one of `git`, `cargo`, `bun`, `npm`, `pnpm`, `python`, `python3`, `pytest`, `bash`, or `rg`
- `args`: command arguments; the first argument must be an allowlisted subcommand

Optional inputs:

- `cwd`: working directory relative to the configured repository root
- `timeout_secs`: timeout in seconds from 1 to 600

Rules:

- The command must run without shell interpolation.
- The working directory must resolve inside the configured repository root.
- The executable must be an allowlisted program name, not a path.
- The subcommand must be allowlisted:
  - `git`: `status`, `diff`, `log`, `show`, `rev-parse`, `ls-tree`
  - `cargo`: `check`, `test`, `build`, `clippy`
  - `bun`: `run`, `test`
  - `npm`: `run`, `test`
  - `pnpm`: `run`, `test`
  - `python`/`python3`: a repo-relative `.py` script path as the first argument
  - `pytest`: validation invocation
  - `bash`: exactly `references/check-base-image.sh` or `references/check-base-image.sh task`
  - `rg`: search invocation
- The default timeout is 120 seconds and the maximum is 600 seconds.
- Arguments that directly reference paths outside the repository root must be refused, except for the server-owned `{scratch}` token.
- `{scratch}` may appear inside data/output arguments and expands to the stable repository-specific scratch root; traversal outside that root must be refused.
- `{scratch}` must not select a `python`/`python3` script. Repository scripts stay repository-relative, and scratch-hosted Python code uses `artifact_python_run`.
- The tool must return command, cwd, allowlist rule, exit code, duration, stdout, and stderr.
- Output must redact probable secret values without masking ordinary path-shaped output, env-var names, or documentation prose, then truncate large streams.
- Program resolution may include server-host configuration through `CONTEXTPATCH_VALIDATION_PATHS` in addition to the process `PATH`; callers still supply only executable names, never paths or environment variables.
- The tool must refuse direct `harbor run` and direct callers to `harbor_run_start`. It must also refuse arbitrary shell, shell snippets, shell scripts other than `references/check-base-image.sh` with its optional exact `task` argument, environment inspection, destructive Git commands, package installation, Docker, and automatic commits.

### `fixture_generator_run`

Plans or runs a repo-relative Python fixture generator that may mutate only declared outputs.

Required inputs:

- `script_path`: repository-relative `.py` file

Optional inputs:

- `args`: argument array passed after the script path
- `cwd`: working directory relative to the repository root
- `expected_output_paths`: exact changed paths allowed after execution
- `expected_output_prefixes`: changed-path prefixes allowed after execution
- `allowed_existing_dirty_paths`: dirty paths allowed before execution; defaults to the script path
- `timeout_secs`: timeout from 1 to 600
- `dry_run`: defaults to `true`
- `confirm`: required literal `run fixture generator` when `dry_run` is `false`

Rules:

- The script must be a repository-relative `.py` file executed as `python3 <script_path> ...` without shell interpolation.
- At least one expected output path or prefix is required.
- Dry-run must return the exact command plan and must not run Python.
- Execution requires exact confirmation.
- Before execution, the worktree may be dirty only at `allowed_existing_dirty_paths`; this permits temporary untracked generator scripts while preventing unrelated changes from being hidden.
- After execution, every dirty path other than allowed pre-existing dirty paths must match `expected_output_paths` or `expected_output_prefixes`.
- If the generator exits non-zero or leaves undeclared changes, the tool must refuse and report the command output or undeclared paths.
- The tool does not commit or delete the generator; callers should use `delete_untracked_exact` for temporary scripts that must not enter the submission.

### `base_image_check_run`

Plans or runs the exact `references/check-base-image.sh` validation workflow.

Optional inputs:

- `project_path`: when provided, must be exactly `task`
- `timeout_secs`: timeout from 1 to 600
- `dry_run`: defaults to `true`
- `confirm`: required literal `run base image check` when `dry_run` is `false`

Rules:

- The required script path is fixed as `references/check-base-image.sh`.
- Dry-run must return either the exact `bash references/check-base-image.sh` or `bash references/check-base-image.sh task` plan without running it.
- Execution requires exact confirmation and uses the guarded no-shell process runner.
- `project_path` values other than `task` must be refused.
- This tool must not expose arbitrary `bash`, shell snippets, or caller-selected shell scripts.

### `image_cleanliness_check_run`

Plans or runs the narrow Docker image-cleanliness check used by task validators.

Required inputs:

- `image`: Docker image reference

Optional inputs:

- `filename`: simple file name to search for; defaults to `solve.sh`
- `timeout_secs`: timeout from 1 to 600
- `dry_run`: defaults to `true`
- `confirm`: required literal `run image cleanliness check` when `dry_run` is `false`

Rules:

- Dry-run must return the exact plan `docker run --rm --network none --entrypoint find <image> / -name <filename>` without running Docker.
- Execution requires exact confirmation.
- The image reference and file name must be bounded single tokens, not shell fragments.
- The tool must not expose generic Docker arguments, bind mounts, network access, caller-selected entrypoints, shell snippets, or arbitrary commands.
- A clean result is an exit status success with empty stdout; any found path is reported as a match instead of hidden.

### `docker_image_inspect`

Plans or runs a read-only Docker image metadata inspection.

Required inputs:

- `image`: Docker image reference

Optional inputs:

- `timeout_secs`: timeout from 1 to 600
- `dry_run`: defaults to `true`
- `confirm`: required literal `inspect docker image` when `dry_run` is `false`

Rules:

- Dry-run must return the exact plan `docker image inspect <image>` without running Docker.
- Execution requires exact confirmation.
- The image reference must be a bounded Docker image token, not a shell fragment.
- The tool must not expose generic Docker arguments, container execution, bind mounts, network controls, shell snippets, or arbitrary commands.
- Output must be redacted, truncated, and represented as external validator output rather than repository mutation.

### `write_existing_file_exact_hash`

Overwrites an existing UTF-8 repository file only when its current SHA-256 digest matches the caller's expectation.

Required inputs:

- `path`
- `content`
- `expected_sha256`: lowercase SHA-256 hex digest of the current file content

Optional inputs:

- `dry_run`: defaults to `true`
- `confirm`: required literal `write exact hash` when `dry_run` is `false`

Rules:

- The target and every intermediate component must be non-symlinked and repository-confined, and the target must already be a regular file.
- The expected digest must be lowercase SHA-256 hex and must match the file bytes currently on disk.
- Dry-run must report the current digest, new digest, byte count, and required confirmation without writing.
- Execution requires exact confirmation, reopens and locks the guarded target, rechecks its hash, and replaces it through a same-directory temporary file plus descriptor-relative rename on Unix.
- The tool must refuse hash mismatches, missing files, directories, non-lowercase digests, and unconfirmed mutation.

### `fixture_manifest_verify`

Verifies that a fixture manifest exactly matches the current fixture file set and SHA-256 digests.

Required inputs:

- `manifest_path`
- at least one of `fixture_paths` or `fixture_prefixes`

Optional inputs:

- `fixture_paths`: exact regular fixture files
- `fixture_prefixes`: repository-relative files or directories whose regular files are included recursively

Rules:

- The manifest path and all fixture paths/prefixes must resolve inside the repository root.
- Supported manifest shapes are `{ "files": [{ "path": "...", "sha256": "..." }] }` and `{ "path": "sha256" }`; `digest` and `sha256_digest` are accepted aliases inside `files` entries.
- Manifest digests must be lowercase SHA-256 hex.
- The tool must compute actual SHA-256 values for every declared fixture file.
- The tool must fail with `isError: true` when any manifest file is missing, any collected fixture file is unlisted, or any digest differs.
- The success report must include the manifest SHA-256, expected file count, actual file count, and empty mismatch sets.

### `fixture_manifest_refresh`

Regenerates a fixture manifest from declared fixture files or prefixes.

Required inputs:

- `manifest_path`
- at least one of `fixture_paths` or `fixture_prefixes`

Optional inputs:

- `fixture_paths`: exact regular fixture files
- `fixture_prefixes`: repository-relative files or directories whose regular files are included recursively
- `expected_manifest_sha256`: required when overwriting an existing manifest
- `dry_run`: defaults to `true`
- `confirm`: required literal `refresh fixture manifest` when `dry_run` is `false`

Rules:

- The output manifest shape is `{ "version": 1, "algorithm": "sha256", "files": [{ "path", "sha256", "bytes" }] }`.
- File entries must be sorted by normalized repository-relative path.
- Dry-run must report the proposed file count, new manifest SHA-256, and current manifest SHA-256 when present.
- Execution requires exact confirmation.
- Overwriting an existing manifest requires `expected_manifest_sha256` matching the current manifest bytes.
- The manifest parent directory must already exist.
- The tool must write through a temporary file plus rename and must not create or mutate fixture files.

### `read_command_log`

Reads a captured command log produced by guarded validation or an asynchronous task-image, Harbor, or validation-profile run.

Required inputs:

- `log_id`: opaque id returned by a command-running tool

Optional inputs:

- `offset`: character offset into the redacted log, defaults to `0`
- `max_chars`: maximum characters to return, from 1 to 200000; defaults to 12000

Rules:

- `log_id` must be an opaque command-log id, not a path.
- The tool must only read from the contextpatch command-log directory.
- Logs contain the same redacted command output shape as guarded command responses.
- The tool must page from the requested character offset and truncate large responses rather than returning unbounded JSON-RPC payloads.
- Offsets are character offsets in the redacted UTF-8 log text, not byte offsets.
- The response must report lifecycle status. Ordinary completed logs report `completed`; asynchronous logs may report `running`, `completed`, `failed`, or `timed_out`.
- An asynchronous log still marked `running` but owned by an earlier server instance must report `unknown`. The caller must inspect current repository and external state before retrying because the earlier process outcome is not known.

### `harbor_run_start`

Starts one typed Harbor invocation in a background worker and returns immediately with an opaque log id.

Required inputs:

- `agent`: Harbor agent identifier containing only ASCII letters, digits, `.`, `_`, or `-`, and not beginning with `-`

Optional inputs:

- `project`: normalized repository-relative project directory; defaults to `task`
- `timeout_secs`: from 1 to 3600; defaults to 3600

Rules:

- The exact command is `harbor run -p <project> --agent <agent>` with explicit argv and no shell.
- The project must use `/` separators, contain no backslashes, and identify an existing repository directory with no symlink or leading-hyphen path components. The exact normalized string validated against the repository must be the argv value passed to Harbor.
- Harbor, task-image, and validation-profile runs share a limit of two active background jobs per server process.
- The start response must return `status: "running"`, the stable `log_id`, an exact `read_command_log` polling request, and restart semantics.
- Polling must not restart the run. While active it reports `running`; terminal status is `completed`, `failed`, or `timed_out`.
- Completed logs must include the bounded command output and structured Harbor evidence when available: authoritative rewards, result/job paths, the union of rewarded and exception-only trials, aggregate exception types, trial metadata, verifier reward, and redacted/truncated verifier stdout.
- Structured evidence must prioritize exception-bearing trials, return at most 100 trials and 1000 rewards, remain within 700000 serialized bytes, and report total, returned, omitted, and truncation counts explicitly.
- Evidence reads must accept only the exact `jobs/<job>/result.json` layout and trial directories named by that result. Traversal, symlink components, malformed JSON, oversized artifacts, missing artifacts, and unsafe trial names must be surfaced explicitly rather than guessed.
- After a server restart, an in-progress log owned by the previous instance reports `unknown`; callers must inspect existing Harbor job state before deciding whether to start another run.

### `validation_profile_run`

Starts a predefined sequence of allowlisted validation commands as one asynchronous MCP workflow.

Required inputs:

- `profile`: one of `repo-basic`, `rust-workspace`, `datacore-vscode`, `datacore-m6-vscode`, or `dynamo-harbor-task`

Optional inputs:

- `timeout_secs`: per-command timeout override, from 1 to 600
- `stop_on_failure`: stop after the first non-zero or timed-out command; defaults to true

Rules:

- Profiles must be explicit server-owned command lists, not user-supplied shell snippets.
- Each command must pass the same `run_guarded_command` allowlist, cwd, timeout, and redaction rules.
- The `dynamo-harbor-task` profile assigns 3600-second defaults to its exact `harbor run` steps while retaining lower defaults for Git and base-image checks. A caller-supplied profile-wide override remains limited to 600 seconds.
- The first response must return `status: "running"`, one stable profile `log_id`, an exact `read_command_log` polling request, and restart semantics.
- The terminal profile log must contain compact per-command status, duration, timeout state, and command log ids. Full per-command output should be retrieved through those ids only when needed.
- Harbor, task-image, and validation-profile runs share a limit of two active background jobs per server process.
- Profiles may use executable resolution from `CONTEXTPATCH_VALIDATION_PATHS` through the guarded runner; callers cannot pass environment overrides per request.
- The `dynamo-harbor-task` profile must append a structured `harbor_summary` object with oracle rewards, nop rewards, deterministic repeat checks, expected oracle/nop pass checks, missing-reward diagnostics, and an aggregate `passed` boolean.
- A false `harbor_summary.passed` value is a terminal profile failure even when every Harbor process
  exits successfully.
- Harbor rewards must come from the repository-confined `result.json` path printed by Harbor when that file is readable and valid. Rendered reward tables, labeled rewards, and progress means are compatibility fallbacks only.
- Profiles must not commit, push, reset, checkout, clean, stash, or mutate product files.

### `setup_profile_run`

Plans or runs a predefined repository setup action from a typed profile.

This tool exists for real project setup cases that need package-manager or scaffold commands without turning `run_guarded_command` into a broad shell/package-manager gateway.

Required inputs:

- `profile`: currently `node-capacitor-shell`
- `action`: profile-owned action name

Optional inputs:

- `params`: typed action parameters
- `cwd`: working directory relative to the repository root
- `timeout_secs`: future execution timeout, from 1 to 600
- `dry_run`: defaults to `true`
- `confirm`: required mutation confirmation literal `run setup profile` when `dry_run` is `false`

Supported `node-capacitor-shell` actions:

- `install_capacitor_dependencies`
- `install_capacitor_filesystem`
- `cap_init` with `app_id`, `app_name`, and `web_dir`
- `cap_add_ios`
- `cap_add_android`
- `cap_sync` with optional `platform`: `ios`, `android`, or `all`
- `ios_pod_install`

Rules:

- The caller must not supply raw `program` or `args`.
- The profile must derive the exact planned command and expected changed-path classes.
- The profile may select npm or pnpm from repository lockfiles, but callers still must not choose arbitrary package-manager commands.
- The `node-capacitor-shell` dependency install action installs `@capacitor/core`, `@capacitor/ios`, and `@capacitor/android` as dependencies, and `@capacitor/cli` as a dev dependency.
- The `install_capacitor_filesystem` action installs only `@capacitor/filesystem`; callers must not supply arbitrary package names.
- `ios_pod_install` must refuse when the cwd has no `Podfile`; Swift Package Manager based Capacitor projects do not need CocoaPods.
- Action params must be typed and validated by the profile.
- The working directory must resolve inside the configured repository root.
- Dry-run output must clearly mark the plan as an external mutator and not claim atomic contextpatch writes.
- `dry_run=false` must require a clean worktree, exact confirmation, before/after status capture, changed-path reporting, and refusal for changed paths outside the profile's expected classes.
- The tool must not broaden the `run_guarded_command` validation allowlist.
- `pnpm` support for `node-capacitor-shell` remains profile-owned setup authority and must not make raw `pnpm add` available through `run_guarded_command`.

### `native_build_run`

Plans or runs a typed native build/test action without exposing raw native build tools.

Required inputs:

- `action`: one of `ios_build`, `ios_test`, `android_assemble_debug`, or `android_unit_test`
- `params`: typed action parameters

Optional inputs:

- `cwd`: working directory relative to the repository root
- `timeout_secs`: timeout from 1 to 600
- `dry_run`: defaults to `true`

iOS params:

- `workspace`: repository-relative `.xcworkspace` or `.xcodeproj`
- `scheme`
- optional `configuration`, default `Debug`
- optional `sdk`, default `iphonesimulator`
- optional `destination`
- optional `derived_data_path`: repository-relative Xcode DerivedData directory, useful when a later `ios_install_app` needs a repository-relative `.app` path

Android params:

- optional `gradlew`: repository-relative Gradle wrapper path, default `gradlew`

Rules:

- The caller must not supply raw `program` or `args`.
- Core must derive exact `xcodebuild` or Gradle wrapper command plans from typed params.
- `preflight_health` must probe Xcode with `xcodebuild -version`, not a generic `--version` flag, and should report `xcode-select -p` so callers can diagnose missing Xcode or unaccepted-license states.
- `derived_data_path`, when supplied, must be repository-relative and is passed as `-derivedDataPath`; use a gitignored/cache path so the source-status guard can still verify the build did not change tracked or visible source state.
- Android Gradle wrapper execution is allowed only through this native build policy and must resolve to a file inside the repository.
- The tool must not broaden the `run_guarded_command` validation allowlist or global executable-name validation.
- Dry-run must not execute native tools.
- Executed builds must compare Git source status before and after execution and refuse if source status changes.
- Native build output is external validator output; the tool must not claim atomic edit semantics for build products or caches.

### `native_device_run`

Plans or runs a typed native simulator/emulator/device action without exposing raw device tools.

Required inputs:

- `action`: one of `ios_list_simulators`, `ios_create_simulator`, `ios_boot_simulator`, `ios_cap_run`, `ios_install_app`, `ios_launch_app`, `ios_read_logs`, `android_list_devices`, `android_install_app`, `android_launch_app`, or `android_read_logcat`

Optional inputs:

- `params`: typed action parameters
- `cwd`: working directory relative to the repository root
- `timeout_secs`: timeout from 1 to 600
- `dry_run`: defaults to `true`
- `confirm`: required literal `run native device` when `dry_run` is `false` for device-state-changing actions

Params by action:

- `ios_create_simulator`: `name`, `device_type`, optional `runtime`
- `ios_cap_run`: `target`
- `ios_boot_simulator`: `device`
- `ios_read_logs`: `device`, optional `duration` in seconds from `1` to `30`
- `ios_install_app`: `device`, `app_path`
- `ios_launch_app`: `device`, `app_id`
- `android_list_devices`: optional `serial`
- `android_install_app`: optional `serial`, `apk_path`
- `android_launch_app`: optional `serial`, `app_id`
- `android_read_logcat`: optional `serial`, optional `lines`

`ios_install_app` requires `app_path` to be repository-relative. For Xcode builds, pair this with `native_build_run`'s `derived_data_path`; for example, a build using `derived_data_path: ".contextpatch-derived-data"` can install `.contextpatch-derived-data/Build/Products/Debug-iphonesimulator/App.app`.

Rules:

- The caller must not supply raw `program` or `args`.
- Core must derive exact `xcrun simctl` or `adb` command plans from typed params.
- `ios_cap_run` must select `pnpm exec cap run ios --target <target> --no-sync` when `pnpm-lock.yaml` is present in cwd, otherwise `npm exec -- cap run ios --target <target> --no-sync`; it is for running already-synced Capacitor iOS projects, not broad package-manager execution.
- `ios_read_logs` must use `log stream --timeout <duration>` so simulator log reads terminate at the platform command level instead of relying on external command timeouts.
- Artifact paths such as `app_path` and `apk_path` must be repository-relative paths.
- Device ids, serials, and app ids must be single-line bounded values with only supported characters.
- Dry-run must not touch simulator, emulator, or device state.
- `dry_run=false` for device-state-changing actions must require exact confirmation `run native device`.
- Device operations must not mutate repository source and must not claim atomic edit semantics.

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
- Commit subjects and bodies should remain project-owned and must not add Claude, Anthropic, AI, assistant, or co-authored-by attribution unless explicitly requested by the repository owner for that commit.
- The tool must verify that the staged path set exactly matches `paths` before committing.
- On success, the tool returns the commit hash, short hash, committed paths, and post-commit short status.
- Commit failure after staging must be reported explicitly; it must not pretend the commit succeeded.

### `git_commit_scoped`

Creates a local Git commit from an explicit subset of dirty paths while preserving unrelated dirty paths.

Required inputs:

- `paths`: non-empty list of repository-relative dirty paths to include in the commit
- `subject`: single-line commit subject

Optional inputs:

- `body`: commit body/trailers
- `dry_run`: defaults to `true`
- `confirm`: required literal `commit scoped paths` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `commit scoped paths`.
- Every requested path must currently be dirty. Unrelated dirty paths may exist and must remain uncommitted.
- The Git index must be clean before mutation so pre-staged unrelated paths cannot be accidentally included.
- Paths must be normalized repository-relative paths and must not use path traversal, absolute paths, NUL bytes, or Git pathspec metacharacters.
- Rename/copy status entries are refused until a dedicated tracked-move workflow exists.
- The tool may run `git add -- <paths>` and one local `git commit`; it must not fetch, pull, push, reset, checkout, stash, clean, or modify remotes.
- Commit subjects and bodies should remain project-owned and must not add Claude, Anthropic, AI, assistant, or co-authored-by attribution unless explicitly requested by the repository owner for that commit.
- The tool must verify that the staged path set exactly matches `paths` before committing.
- On success, the tool returns the commit hash, short hash, committed paths, and remaining dirty paths.
- Commit failure after staging must be reported explicitly; it must not pretend the commit succeeded.

### `git_stage_exact`

Stages explicit dirty paths without creating a commit.

Required inputs:

- `paths`: non-empty list of repository-relative dirty paths to stage

Optional inputs:

- `dry_run`: defaults to `true`
- `confirm`: required literal `stage exact paths` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `stage exact paths`.
- Every requested path must currently be dirty.
- The Git index must be clean before mutation so pre-staged unrelated paths cannot be mixed with the staged set.
- Paths must be normalized repository-relative paths and must not use path traversal, absolute paths, NUL bytes, or Git pathspec metacharacters.
- Rename/copy status entries are refused until a dedicated tracked-move workflow exists.
- The tool may run `git add -- <paths>` only; it must not commit, fetch, pull, push, reset, checkout, stash, clean, or modify remotes.
- On success, the tool returns staged paths and remaining dirty paths.

### `git_staged_scope_check`

Checks the current Git index against a declared staged-path policy without mutating it.

Optional inputs:

- `allowed_paths`: repository-relative paths that may be staged exactly
- `allowed_prefixes`: repository-relative prefixes or directories under which staged paths are allowed
- `required_paths`: repository-relative paths that must be staged

Rules:

- The tool is read-only and must not run `git add`, `git restore`, `git commit`, fetch, push, reset, checkout, stash, clean, or modify remotes.
- Paths and prefixes must be normalized repository-relative values and must not use path traversal, absolute paths, NUL bytes, or Git pathspec metacharacters.
- A staged path passes when it equals one allowed path or is inside one allowed prefix.
- The response must include the staged paths, allowed paths, allowed prefixes, required paths, disallowed paths, missing required paths, staged path count, and aggregate `passed` boolean.

### `git_commit_prefix`

Creates a local Git commit from all dirty paths under explicit prefixes while preserving unrelated dirty paths.

Required inputs:

- `prefixes`: non-empty list of repository-relative prefixes to expand
- `subject`: single-line commit subject

Optional inputs:

- `body`: commit body/trailers
- `dry_run`: defaults to `true`
- `confirm`: required literal `commit prefix paths` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `commit prefix paths`.
- Prefixes must normalize inside the repository root and must not use path traversal, absolute paths, NUL bytes, or Git pathspec metacharacters.
- The tool expands prefixes against the current dirty-path set and reports the exact expanded paths and count before mutation.
- At least one dirty path must match the prefixes; unrelated dirty paths may exist and must remain uncommitted.
- The Git index must be clean before mutation so pre-staged unrelated paths cannot leak into the commit.
- The expanded path count is bounded; large fixture commits should still be reviewable from the dry-run path list.
- The tool may run `git add -- <expanded paths>` and one local `git commit`; it must not fetch, pull, push, reset, checkout, stash, clean, or modify remotes.
- The tool must verify that the staged path set exactly matches the expanded path set before committing.
- On success, the tool returns the commit hash, short hash, committed expanded paths, and remaining dirty paths.
- Commit failure after staging must be reported explicitly; it must not pretend the commit succeeded.

### `git_restore_exact`

Restores explicit dirty tracked paths from `HEAD` without exposing broad checkout, reset, clean, or stash operations. This is for generated-noise cleanup before exact commits.

Required inputs:

- `paths`: non-empty list of repository-relative dirty tracked paths

Optional inputs:

- `dry_run`: defaults to `true`
- `confirm`: required literal `restore exact paths` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `restore exact paths`.
- Every requested path must currently be dirty, tracked by Git, and normalized under the repository root.
- The tool may restore a subset of dirty paths, but it must not touch any path not explicitly listed.
- The tool must refuse untracked paths because restoring from `HEAD` cannot safely recreate/delete untracked files.
- The tool may run only `git restore --staged --worktree -- <paths>` plus read-only Git status/tracking checks.
- On success, the requested paths must no longer be dirty, and the response must report any remaining dirty paths.

### `delete_untracked_exact`

Deletes explicit untracked regular files without exposing broad `git clean` or recursive filesystem deletion.

Required inputs:

- `paths`: non-empty list of repository-relative untracked file paths

Optional inputs:

- `dry_run`: defaults to `true`
- `confirm`: required literal `delete untracked files` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `delete untracked files`.
- Every requested path must be normalized under the repository root and currently reported by Git as untracked.
- Every requested path must be a regular file, not a directory.
- The tool must delete only the explicit paths listed, never recurse, glob, or clean unrelated untracked files.
- On success, the requested paths must no longer be untracked, and the response must report remaining untracked paths.

### `delete_generated_prefix`

Deletes generated untracked or ignored files and empty directories under explicit prefixes without exposing broad clean/reset behavior.

Required inputs:

- `prefixes`: non-empty list of repository-relative prefixes to expand

Optional inputs:

- `dry_run`: defaults to `true`
- `confirm`: required literal `delete generated paths` when `dry_run` is `false`

Rules:

- The tool must default to dry-run and perform no mutation unless `dry_run` is `false` and `confirm` exactly equals `delete generated paths`.
- Prefixes must normalize inside the repository root and must not use path traversal, absolute paths, NUL bytes, or Git pathspec metacharacters.
- The candidate set comes from Git untracked and ignored status plus empty directories found under the prefixes.
- The tool must refuse if a matched generated directory contains any tracked path.
- Dry-run must report exact files and directories that would be deleted.
- Execution may delete regular generated files, ignored generated directories, and empty directories only within the matched prefixes.
- The tool must not run `git clean`, recurse from arbitrary roots, delete tracked files, or hide unmatched generated files elsewhere in the repository.

### `git_remote_list`

Reads configured Git remotes without exposing broader `git remote` mutation.

Rules:

- The tool may run only read-only `git remote -v`.
- The response must parse remotes into name, URL, and fetch/push kind.
- The tool must not add, rename, remove, or set-url for remotes.

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

### `git_branch_prepare`

Plans or prepares one local branch from one explicit remote base branch.

Required inputs:

- `base_branch`: remote branch name to fetch and prepare from
- `branch`: local branch name to create or switch to

Optional inputs:

- `remote`: remote name; defaults to `origin`
- `required_files`: repository-relative files that must exist after preparation
- `reset_existing`: defaults to `false`
- `dry_run`: defaults to `true`
- `confirm`: required literal `reset branch from remote base` when `reset_existing` is `true`

Rules:

- The configured repository root must be exactly a Git worktree root. This action names no paths, so nothing else bounds it: under a subdirectory root it would fetch, switch, and reset the *enclosing* repository, touching files the operator never configured. A descendant worktree remains reachable through the wrapper `repository` selector, which resolves to an exact worktree root.
- The default dry-run must validate the repository, remote, clean-worktree gate, branch names, and required-file path syntax without fetching, updating refs, creating/resetting branches, or switching the worktree.
- The dry-run response must report the exact `git fetch` and branch command argv that `dry_run: false` would run. Guards that depend on the newly fetched ref remain explicitly deferred until execution.
- Branch preparation may execute only when `dry_run` is explicitly `false`.
- The tool must require a clean worktree before fetching or switching branches.
- The tool may fetch only `refs/heads/<base_branch>:refs/remotes/<remote>/<base_branch>`.
- Remote, base branch, local branch, and required-file paths must be validated before Git mutation.
- If the local branch does not exist, the tool may create it from the fetched remote-tracking ref and switch to it.
- If the local branch exists and `reset_existing` is false, the fetched remote-tracking ref must already be an ancestor of that branch before switching.
- If the local branch exists and `reset_existing` is true, the tool must require `confirm: "reset branch from remote base"` before recreating that branch from the fetched remote-tracking ref.
- Required files must be checked against the target ref before switching/resetting and checked again in the worktree after preparation.
- A dry-run response must include the planned action, exact commands, previous branch, remote ref, required files, and deferred execution guards. An execution response must include the action taken, previous branch, current branch, remote ref, remote commit, prepared head, ancestor check, required-file results, and post-preparation status.
- The tool must not expose generic checkout, rebase, merge, merge-tree, stash, clean, arbitrary reset, arbitrary fetch, push, or multi-branch operations.

### `git_merge_readiness`

Performs read-only merge/PR readiness analysis between two validated refs.

Required inputs:

- `base_ref`: base ref, remote-tracking ref, `HEAD`, or commit hash
- `target_ref`: target ref, remote-tracking ref, `HEAD`, or commit hash

Optional inputs:

- `fetch`: defaults to `false`
- `remote`: remote name used only when `fetch` is true; defaults to `origin`
- `target_branch`: remote branch to fetch when `fetch` is true; inferred from `target_ref` when `target_ref` starts with `<remote>/` or `refs/remotes/<remote>/`

Rules:

- The tool must validate refs before invoking Git and reject ref syntax such as whitespace, pathspec metacharacters, revision operators, or ref traversal.
- If `fetch` is true, the tool may fetch only `refs/heads/<target_branch>:refs/remotes/<remote>/<target_branch>` for the validated remote and branch.
- The tool must refuse if source status changes during fetch.
- The tool may run read-only `merge-base`, `rev-list --count`, and `diff --name-only -z` queries after ref resolution.
- The response must include resolved commits, merge base, ahead counts, changed-file counts, `changed_on_both_sides`, and `likely_conflict_candidates`.
- `likely_conflict_candidates` is a conservative file-level intersection, not a guarantee that Git merge content conflicts exist.
- The tool must not checkout, merge, run merge-tree as a generic command, reset, stash, clean, edit source files, stage files, commit, or push.

### `git_push_exact`

Pushes exactly the current branch `HEAD` to the matching branch on an explicit remote.

Required inputs:

- `remote`
- `branch`
- `expected_head`
- `confirm`: literal `push exact commit`

Rules:

- The configured repository root must be exactly a Git worktree root. This action names no paths, so nothing else bounds it: under a subdirectory root it would push the *enclosing* repository. A descendant worktree remains reachable through the wrapper `repository` selector.
- The tool must require a clean worktree before pushing.
- The current branch must exactly match `branch`.
- The current `HEAD` must match `expected_head`.
- The tool must fetch `<remote> <branch>` immediately before the push.
- The remote-tracking ref must not be ahead of `HEAD`, and it must be an ancestor of `HEAD`; divergent or non-fast-forward states must be refused.
- The push refspec must be exactly `HEAD:refs/heads/<branch>` for the requested branch.
- The tool must not use force push, delete refs, push tags, push multiple branches, or push a different local branch.
- On success, the response must include the pushed commit hash, branch, remote, previous remote head, refspec, and post-push status.

### `github_pr_run`

Runs narrow GitHub pull-request workflows through `gh` without exposing arbitrary `gh` command execution.

Required inputs:

- `action`: one of `auth_status`, `pr_view`, `pr_comments`, `pr_checks`, `workflow_runs_for_commit`, `workflow_run_view`, `workflow_job_log`, `workflow_run_rerun_failed`, or `pr_create`

Action-specific inputs:

- `pr_view`, `pr_comments`, and `pr_checks`: `number`
- `workflow_runs_for_commit`: full 40-character `head_sha`
- `workflow_run_view` and `workflow_run_rerun_failed`: `run_id`
- `workflow_job_log`: `job_id`
- `pr_create`: `base`, `head`, `title`, `body`

Optional inputs:

- `repository`: validated `OWNER/REPO` target for PR and Actions actions; use it when the local checkout is a fork but the PR or run belongs to upstream
- `comment_contains`: optional case-insensitive 1-256 character local filter for `pr_comments`
- `limit`: result bound for `pr_comments` (default 20, maximum 100) or `workflow_runs_for_commit` (default 20, maximum 50)
- `log_view`: bounded `workflow_job_log` projection, `tail` by default to retain terminal failure evidence or `head` for the beginning
- `draft`: create a draft PR when `action` is `pr_create`
- `dry_run`: defaults to true for `pr_create` and `workflow_run_rerun_failed`
- `confirm`: literal `create pull request` for confirmed PR creation or `rerun failed workflow jobs` for a confirmed failed-job rerun

Rules:

- The tool must use the GitHub CLI as `gh` with explicit argv, no shell.
- Read-only actions may execute directly.
- `pr_checks` must return structured check names, states, workflows, and links so callers can identify the corresponding Actions run.
- `pr_view` must include the exact head commit SHA. `workflow_runs_for_commit` must require that full SHA and return only a bounded structured run list, so callers can distinguish current evidence from stale runs.
- `pr_comments` must filter locally rather than accepting caller-provided `jq` or API paths, return matching comments newest-first, and bound the result count.
- `workflow_run_view` may return typed run/job metadata; `workflow_job_log` may return only one explicit job's bounded log output, must default to the redacted tail, and may return the redacted head only when explicitly requested.
- `workflow_run_rerun_failed` may run only `gh run rerun <run_id> --failed`; it must default to dry-run and require exact confirmation before rerunning failed jobs and their dependencies.
- GitHub command output must be redacted for probable secrets and bounded before returning it to the client.
- Repository targeting must be passed as a separate `--repo OWNER/REPO` argv pair after strict validation, never as caller-supplied command text.
- `pr_create` must default to dry-run and return the command plan without mutating GitHub.
- `pr_create` with `dry_run: false` must require the exact confirmation literal.
- The tool must not expose arbitrary `gh` subcommands, repository deletion, workflow dispatch, release publishing, secret access, or forceful Git operations.

### `github_fork_prepare`

Plans or runs the narrow GitHub fork workflow for the current repository through `gh repo fork`.

Optional inputs:

- `remote`: defaults to true; when true, pass `--remote`
- `dry_run`: defaults to true
- `confirm`: literal `prepare github fork` when `dry_run` is false

Rules:

- The tool must default to dry-run and return the exact `gh repo fork` command plan.
- The tool must require exact confirmation before mutating GitHub or remotes.
- The tool must use explicit argv and no shell.
- The tool must not clone because the MCP server is already bound to one repository root.
- The tool must not expose arbitrary `gh repo` subcommands, repository deletion, or broad remote mutation.

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
- Every serialized `tools/call` response must remain at or below 900 KiB so it fits below the
  client's 1 MiB result ceiling after JSON escaping. If a successful handler still exceeds that
  envelope, the server must return a compact success marker stating that output was omitted and
  warning callers not to retry mutations. Oversized error diagnostics must remain `isError: true`
  while replacing the diagnostic with bounded recovery guidance.
- The instrumentation goal is p50/p95/p99 diagnosis across long sessions, especially to distinguish MCP transport overhead from process/output handling.

### `delete_guarded`

Deletes one clean tracked regular file only when its current SHA-256 matches exactly.

Required inputs:

- `path`
- `expected_sha256`: lowercase 64-character SHA-256 of the current content

Optional inputs:

- `dry_run`: defaults to `true`
- `confirm`: required literal `delete tracked file` when `dry_run` is `false`

Rules:

- Require a normalized repository-relative path under the configured Git worktree root.
- Refuse missing, untracked, dirty, non-regular, or symlink targets and symlink path components.
- Refuse malformed or mismatched SHA-256 values.
- Refuse mutation without the exact confirmation literal.
- Recheck the hash immediately before removal.
- Remove only the worktree file, preserve the index entry, and verify the result is an unstaged tracked deletion.
- Report the deleted path and verified SHA-256.

## Current MCP implementation subset

The server currently ships:

1. `replace_exact`
2. `bulk_replace_exact`
3. `read_write_receipts`
4. `read_range`
5. `write_new_file`
6. `write_new_file_base64`
7. `write_existing_file_exact_hash`
8. `file_info`
9. `set_file_executable`
10. `list_directory`
11. `read_file_bytes`
12. `artifact_write_text`
13. `artifact_write_base64`
14. `artifact_delete_exact`
15. `bulk_write_new_files_base64`
16. `create_directory`
17. `diff_preview`
18. `status_guard`
19. `capability_manifest`
20. `preflight_health`
21. `run_guarded_command`
22. `artifact_python_run`
23. `task_image_python_run`
24. `harbor_run_start`
25. `image_cleanliness_check_run`
26. `docker_image_inspect`
27. `fixture_generator_run`
28. `base_image_check_run`
29. `fixture_manifest_verify`
30. `fixture_manifest_refresh`
31. `read_command_log`
32. `validation_profile_run`
33. `git_commit_exact`
34. `git_stage_exact`
35. `git_staged_scope_check`
36. `git_commit_scoped`
37. `git_commit_prefix`
38. `git_restore_exact`
39. `move_tracked`
40. `delete_guarded`
41. `delete_untracked_exact`
42. `delete_generated_prefix`
43. `git_remote_list`
44. `git_remote_check`
45. `git_branch_prepare`
46. `git_merge_readiness`
47. `git_push_exact`
48. `github_pr_run`
49. `github_fork_prepare`
50. `setup_profile_run`
51. `native_build_run`
52. `native_device_run`
53. `project_execute` (project surface only; the other 52 remain internal actions)

The CLI `apply-patch` command remains a not-implemented stub, and `insert_at_anchor` remains a
non-advertised boundary concept. Neither is an MCP capability. See `docs/implementation-roadmap.md`.
