# Claude Desktop

`contextpatch` is intended to run as a local context server for desktop agent clients.

## Planned behavior

The server should expose safe edit tools rather than generic filesystem writes. Claude Desktop or another client can request bounded reads, preview diffs, and apply guarded edits.

## Server boundary

The server should not expose broad filesystem write tools. In particular, the default server must not expose:

- generic `write_file`
- unrestricted delete
- recursive directory writes
- shell execution
- Git reset/checkout/stash/commit

The expected agent workflow is:

1. Use `read_range` to inspect a bounded file section.
2. Use `write_new_file` for create-only file creation or `replace_exact` for exact anchored edits.
3. Use `diff_preview` or `status_guard` once those roadmap tools are implemented.
4. Let the human review Git diff outside the server.

## Configuration shape

During local development, build the server and point Claude Desktop at the compiled binary:

```json
{
  "mcpServers": {
    "contextpatch": {
      "command": "/path/to/contextpatch/target/debug/contextpatch-server",
      "args": [
        "--repo-root",
        "/path/to/repo"
      ]
    }
  }
}
```

Change `--repo-root` to the repository Claude should edit. The server treats that directory as the path guard root.

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

## Currently exposed tools

The current server exposes only the implemented safe primitives:

- `read_range`
- `replace_exact`
- `write_new_file`

Other documented tools remain roadmap items until implemented.

## Failure behavior

If a tool refuses an edit, Claude Desktop should receive a clear refusal reason. Refusal is a successful safety outcome, not a server failure.
