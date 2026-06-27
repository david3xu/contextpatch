# Claude Desktop

`contextpatch` is intended to run as a local context server for desktop agent clients.

## Planned behavior

The server should expose safe edit tools rather than generic filesystem writes. Claude Desktop or another client can request bounded reads, preview diffs, and apply guarded edits.

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
