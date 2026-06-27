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
2. Use `diff_preview` or `replace_exact` with exact anchors.
3. Use `status_guard` before or after writes when repository state matters.
4. Let the human review Git diff outside the server.

## Configuration shape

Exact configuration will be added after the server transport is implemented.

```json
{
  "mcpServers": {
    "contextpatch": {
      "command": "contextpatch-server",
      "args": []
    }
  }
}
```

## Local development command

Before packaging, the server can be launched from the workspace with:

```bash
cargo run -p contextpatch-server
```

After installation, the intended command is:

```bash
contextpatch-server
```

## Failure behavior

If a tool refuses an edit, Claude Desktop should receive a clear refusal reason. Refusal is a successful safety outcome, not a server failure.
