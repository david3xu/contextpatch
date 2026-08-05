# Project Tool Surface Plan

Claude Desktop authorizes local MCP tools by server and tool identity. A ContextPatch server currently
advertises 52 tools, so each configured project can require many separate approval decisions even though
all tools share the same safety implementation and configured trust boundary.

Add an optional project tool surface that advertises one stable `project_execute` tool. The tool routes one
named action to the existing handler for that action. Claude can then persist one approval for the wrapper
per configured project, while ContextPatch continues to enforce every repository, path, hash, Git-state,
dry-run, confirmation, timeout, mutation-lock, and receipt guard.

This reduces Claude approval friction; it does not bypass Claude authorization. An organization policy can
still force per-call approval, and the user must select **Always allow** once when Claude offers it.

## Product decision

- Keep one MCP server per configured project or workspace with a fixed `--repo-root` trust boundary.
- Add `--tool-surface full|project` to `contextpatch-server`.
- Keep `full` as the server default for backward compatibility and direct MCP users.
- Make Claude Desktop configuration use `project` mode for ContextPatch entries.
- In `project` mode, advertise only `project_execute`.
- Keep the existing 52 tool names as internal action names.
- Execute exactly one action per wrapper call. Do not add batching.
- Let project mode optionally select exact descendant Git worktree roots beneath that boundary.
- Do not create one global server that can select arbitrary filesystem project roots.

The fixed workspace boundary is an important usability guard. A broad global wrapper would reduce
approvals further, but it would also grant every call authority to choose any project on the machine.
The bounded selector handles the real multi-repository workspace case without crossing the configured
root.

## Public wrapper contract

Use a deliberately stable schema:

```json
{
  "name": "project_execute",
  "description": "Describe or execute one guarded ContextPatch action for this configured project.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "action": {
        "type": "string",
        "description": "An existing ContextPatch action name, or describe."
      },
      "arguments": {
        "type": "object",
        "description": "Arguments for the selected action."
      },
      "repository": {
        "type": "string",
        "description": "Optional normalized workspace-relative path to an exact descendant Git worktree root. Omit it to use the configured --repo-root unchanged."
      }
    },
    "required": ["action"],
    "additionalProperties": false
  }
}
```

Do not put action names in an `enum`, action count in the description, or generated schemas in the wrapper
definition. Claude's persistent approval key can include tool metadata. The wrapper metadata must remain
unchanged when an internal action is added or its schema evolves. Adding the bounded `repository` property
is an intentional one-time metadata change and can require one renewed Claude approval.

Reserve `action: "describe"` for discovery:

- With no `arguments.name`, return the build stamp and compact sorted action list.
- With `arguments.name`, return that action's description, input schema, annotations, and confirmation
  requirements.
- Reject unknown names and `project_execute` recursion.

All other action values are existing tool names, for example:

```json
{
  "repository": "dynamo-75751f6-build-dependency-and-release-management",
  "action": "read_range",
  "arguments": {
    "path": "README.md",
    "start_line": 1,
    "end_line": 20
  }
}
```

## Dispatch design

Resolve the public invocation before selecting a deadline or acquiring a mutation lock:

```text
MCP tools/call
  -> validate public tool for configured surface
  -> resolve project_execute.action to existing internal tool name
  -> resolve optional repository under the fixed workspace boundary
  -> select the existing per-tool deadline
  -> acquire the existing per-tool repository mutation lock when required
  -> call the existing handler
  -> preserve the existing MCP success/isError response shape
```

The resolved internal name, not `project_execute`, must be passed to:

- `deadline_for`
- `serializes_repository_mutation`
- `call_tool`
- receipt and command-log reporting
- refusal and unknown-outcome recovery guidance

This keeps behavior identical to a direct call against the effective root. A wrapped
`git_commit_exact` remains a Git-deadline, serialized, journaled commit operation; a wrapped
`read_range` remains a read-deadline, non-mutating operation.

Wrapped calls retain the transport's bounded concurrent dispatch and JSON-RPC id correlation.
Wrapped task-image, Harbor, and validation-profile starts return the same pollable log ids as direct
calls; polling must use wrapped `read_command_log` with the same repository selector and server
instance.

Do not duplicate handlers or move safety policy into the wrapper. `project_execute` is routing only.

## Code changes

### Server options

In `crates/server/src/server.rs`:

- Introduce a `ToolSurface` enum with `Full` and `Project`.
- Parse `--tool-surface full|project`.
- Carry the selected surface through initialization, `tools/list`, and `tools/call`.
- Keep omitted `--tool-surface` equivalent to `full`.

### Schema registry

In `crates/server/src/tools/schema/`:

- Keep one canonical collection of the 52 internal action definitions.
- Add the fixed `project_execute` definition.
- Return the 52 definitions for the full public surface.
- Return only `project_execute` for the project public surface.
- Add helpers to look up one internal action definition for `describe`.
- Separate `public_tool_names(surface)` from `internal_action_names()` so capability reporting cannot
  confuse the one advertised wrapper with the actions it exposes.

The documentation drift test must cover both the wrapper contract and all internal action contracts.

### Wrapper resolution

Add a small project-surface module under `crates/server/src/tools/` that:

- parses `action`, optional `arguments`, and the optional wrapper-level `repository`;
- implements the reserved `describe` action;
- rejects recursive wrapper dispatch;
- returns a resolved internal name plus argument map for normal actions.

In `crates/server/src/tools/dispatch.rs`, resolve the invocation before calling `deadline_for` and
`call_tool_with_mutation_lock`. Resolve `repository` through a core workspace guard and pass the
derived root to the existing handler. Validate nested action argument names against the same closed
internal schema used by the full surface before applying a deadline or mutation lock. Keep the
existing `call_tool` match as the only handler router.

### Capability and client instructions

Update `capability_manifest` to report:

- the active public tool surface;
- public tool names;
- all available internal action names;
- that project mode reduces Claude approval prompts but does not bypass Claude policy.

Update initialization instructions so clients:

- call `project_execute` with `action: "describe"` when that is the advertised surface;
- otherwise continue to call `capability_manifest`;
- never infer that hidden direct tools are unavailable in project mode.

### Claude Desktop configurator

Extend `contextpatch configure-claude-desktop` to manage `--tool-surface project` in every detected
ContextPatch server entry:

- preserve each command, repository root, environment, and unrelated argument;
- append the flag when absent;
- replace one existing ContextPatch tool-surface value when explicitly changing modes;
- refuse duplicate or malformed tool-surface arguments;
- preserve unrelated MCP entries and user-authored policy;
- retain the current lock, backup, stale-read check, atomic write, and dry-run behavior;
- remain idempotent.

Provide an explicit `--tool-surface full` rollback path. New project configuration should select project
mode, and existing projects can be migrated together by rerunning the configurator and restarting Claude
Desktop.

The configurator output must say that project mode normally reduces authorization to one persistent
wrapper approval per server. It must not claim to preauthorize tools or override organization policy.

## Safety invariants

- The wrapper cannot change or escape the configured `repo_root` workspace boundary.
- A repository selector must be normalized, workspace-relative, symlink-free, and exactly equal to
  its Git worktree root.
- Omitting the selector preserves configured-root behavior, and full mode remains unchanged.
- The wrapper cannot dispatch itself.
- One call invokes one action only.
- Existing argument parsing and refusal behavior remain authoritative.
- Existing dry-run defaults and exact confirmation literals remain unchanged.
- Existing deadlines are selected from the resolved action.
- Existing mutation locks are selected from the resolved action.
- Existing receipts retain the resolved action name.
- Existing Git/GitHub/process allowlists remain unchanged.
- Project mode must not expose raw shell, broad filesystem, arbitrary Git, or cross-project authority.
- Wrapper annotations must not claim read-only behavior because the wrapper can route mutations.

## Tests

Add focused server tests for:

1. Omitted surface defaults to the existing 52-tool full surface.
2. Project surface advertises exactly one stable `project_execute` schema.
3. `describe` lists all 52 internal actions and returns an exact schema for one action.
4. A wrapped read produces the same result as its direct call.
5. A wrapped guarded write produces the same result and refusal shape as its direct call.
6. A wrapped Git dry run keeps the existing confirmation and repository-state gates.
7. Unknown, missing, malformed, and recursive actions refuse without executing a handler.
8. Read, write, and Git actions retain their existing deadline class.
9. Mutating actions retain repository serialization.
10. Receipt-enabled wrapped actions record the internal action name.
11. A non-Git workspace can select multiple exact child worktrees for reads, Git, validation,
    base-image checks, GitHub dry runs, and repository-specific receipts.
12. Absolute, traversal, malformed, `.git`, symlink, non-Git, and worktree-subdirectory selectors
    refuse before any handler runs.

Add configurator tests for:

1. Migrating all detected ContextPatch entries to project mode.
2. Preserving repository roots, unrelated args, environment, policy, and non-ContextPatch entries.
3. Idempotent repeated configuration.
4. Dry-run reporting without mutation.
5. Refusal on malformed or duplicate tool-surface flags.
6. Explicit rollback to full mode.
7. Backup, permission, lock, and stale-read behavior remaining unchanged.

## Rollout

1. Add `ToolSurface` and surface-specific `tools/list` behavior without changing the default.
2. Add `project_execute`, discovery, and wrapped dispatch.
3. Prove safety equivalence with read, write, process, Git, and receipt tests.
4. Extend the Claude Desktop configurator and documentation.
5. Add exact descendant-worktree selection for multi-repository workspace entries.
6. Build the release server and migrate configured ContextPatch projects or workspaces.
7. Restart Claude Desktop and verify that each entry advertises only `project_execute`.
8. Select **Always allow** once for the wrapper, then exercise read, write, command, and Git dry-run
   actions without another tool-specific approval prompt.
9. Restart Claude Desktop and repeat to prove the approval persists.

## Validation

Run targeted checks during implementation:

```bash
cargo test -p server stage1_mcp_tools_work_together
cargo test -p cli stage1_cli_tools_work_together
cargo fmt --all -- --check
```

Run full validation before publishing:

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p server --bin contextpatch-server
git diff --check
```

## Success criteria

- Project mode exposes one MCP tool per configured project or workspace entry.
- All 52 current actions remain reachable through that tool.
- Adding an internal action does not change the wrapper schema or invalidate its persistent approval.
- Full mode remains backward-compatible.
- Wrapped actions preserve direct-call deadlines, locks, receipts, guards, outputs, and refusals.
- Exact child-repository selection cannot escape the configured workspace boundary.
- Claude Desktop requires at most one persistent tool approval per configured entry when account and
  organization policy permit **Always allow**.
