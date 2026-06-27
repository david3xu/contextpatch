# Claude Desktop

`contextpatch` is intended to run as a local context server for desktop agent clients.

## Planned behavior

The server should expose safe edit and validation tools rather than generic filesystem writes or broad shell access. Claude Desktop or another client can request bounded reads, preview diffs, apply guarded edits, discover capabilities, run preflight health checks, and run allowlisted validation commands.

## Server boundary

The server should not expose broad filesystem write tools. In particular, the default server must not expose:

- generic `write_file`
- unrestricted delete
- recursive directory writes
- shell execution
- Git reset/checkout/stash/fetch/push
- broad or automatic Git commits

The expected agent workflow is:

1. Use `read_range` to inspect a bounded file section.
2. Use `diff_preview` before `replace_exact` when reviewing exact anchored edits.
3. Use `status_guard` before writes when a clean repository or clean target path is required.
4. Use `write_new_file` for create-only file creation.
5. Use `capability_manifest` and `preflight_health` to determine whether this server can support the current workflow.
6. Use `run_guarded_command` only for allowlisted validation commands such as `git status`, `git diff`, `cargo check`, project `bun run` checks, or `rg` drift searches.
7. Use `git_commit_exact` only when the desired local commit path set is explicit and complete. Let the human or another explicitly trusted tool perform fetches and pushes outside the server.

## Build and configure Claude Desktop

Build the release server:

```bash
cargo build --release -p server --bin contextpatch-server
```

Then point Claude Desktop at the compiled binary. Use absolute paths because Claude Desktop does not inherit your shell's current directory:

```json
{
  "mcpServers": {
    "contextpatch": {
      "command": "/absolute/path/to/contextpatch/target/release/contextpatch-server",
      "args": [
        "--repo-root",
        "/absolute/path/to/repo"
      ]
    }
  }
}
```

Change `--repo-root` to the repository Claude should edit. The server treats that directory as the path guard root.

On macOS, Claude Desktop reads this configuration from:

```text
~/Library/Application Support/Claude/claude_desktop_config.json
```

Restart Claude Desktop after changing the config or rebuilding the server binary.

## Local development command

Before packaging, the server can be launched from the workspace with:

```bash
cargo run -p server --bin contextpatch-server -- --repo-root /path/to/repo
```

After installation, the intended command is:

```bash
contextpatch-server --repo-root /path/to/repo
```

The workspace package is named `server`; the installed binary remains `contextpatch-server` to avoid colliding with generic commands.

## Quick MCP smoke test

After restarting Claude Desktop, ask it to list available `contextpatch` tools. It should see:

- `read_range`
- `diff_preview`
- `replace_exact`
- `status_guard`
- `write_new_file`
- `capability_manifest`
- `preflight_health`
- `run_guarded_command`
- `read_command_log`
- `validation_profile_run`
- `git_commit_exact`

## Currently exposed tools

The current server exposes the implemented safe primitives:

- `read_range`
- `diff_preview`
- `replace_exact`
- `status_guard`
- `write_new_file`
- `capability_manifest`
- `preflight_health`
- `run_guarded_command`
- `read_command_log`
- `validation_profile_run`
- `git_commit_exact`

Other documented tools remain roadmap items until implemented.

`run_guarded_command` is not a shell. It accepts an executable name and argument array, runs from a repo-root-confined working directory, allows only documented validation-oriented programs/subcommands, drains stdout/stderr concurrently, times out, redacts probable secret values without hiding ordinary paths or docs, and returns command/cwd/exit-code/duration metadata.

Use `validation_profile_run` when a workflow has a named validation sequence, such as `repo-basic`, `rust-workspace`, `datacore-vscode`, or `datacore-m6-vscode`. It reduces MCP round trips by running the server-owned allowlisted commands in sequence and returning a compact summary plus `log_id` values. Use `read_command_log` only for logs that need inspection.

Use `git_commit_exact` for the narrow local-commit case that previously required leaving contextpatch entirely: the tool validates that `paths` exactly equals the repository's full dirty-path set, defaults to dry-run, requires `confirm: "commit exact paths"` when `dry_run` is false, stages only those paths, creates one local commit, and reports the commit hash. It still does not run `git fetch`, `git push`, reset, checkout, stash, or clean.

## Failure behavior

If a tool refuses an edit, Claude Desktop should receive a clear refusal reason. Refusal is a successful safety outcome, not a server failure.
