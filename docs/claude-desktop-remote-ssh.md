# Claude Desktop with ContextPatch over SSH

ContextPatch uses line-oriented MCP over standard input and standard output. It does not provide a
network listener, but Claude Desktop can use SSH as the transport and run
`contextpatch-server` on a remote machine.

This setup gives Claude Desktop the guarded ContextPatch tools for a remote repository. It does not
provide unrestricted SSH or remote-shell access.

## Architecture

```text
Claude Desktop
    |
    | MCP JSON-RPC over local ssh stdin/stdout
    v
remote contextpatch-server
    |
    v
remote repository and remote tools
```

The SSH process runs locally and reads the normal `~/.ssh/config`. Host aliases, included files,
identity files, jump hosts, and other OpenSSH settings therefore continue to apply.

## Prerequisites

- The host alias works from Terminal, for example `ssh curtin`.
- Authentication can complete without an interactive password, key-passphrase, or MFA prompt.
- Any required VPN is connected.
- The remote machine has a compatible `contextpatch-server` binary.
- The repository and required supporting tools, such as `git`, exist on the remote machine.
- Remote shell startup files do not print banners or other text to stdout for non-interactive
  commands.

Claude Desktop cannot answer an SSH prompt. Verify unattended authentication before configuring
MCP:

```sh
/usr/bin/ssh -T \
  -o BatchMode=yes \
  -o ConnectTimeout=15 \
  curtin \
  'printf "ready\n"'
```

The command must print `ready` and exit successfully. Run `ssh curtin` interactively first if the
host key has not yet been accepted.

On macOS, an encrypted key can be stored in the login Keychain:

```sh
ssh-add --apple-use-keychain ~/.ssh/id_ed25519
```

The corresponding SSH configuration can include:

```sshconfig
Host *
    AddKeysToAgent yes
    UseKeychain yes
    IdentityFile ~/.ssh/id_ed25519
```

Do not place passwords or private-key contents in Claude Desktop's configuration.

## Install the remote server

The binary must match the remote operating system and CPU architecture. A binary built normally on
macOS will not run on a Linux host. Check the remote platform:

```sh
ssh curtin 'uname -s && uname -m'
```

The simplest option is to build ContextPatch on the remote host:

```sh
ssh curtin
cd /remote/path/to/contextpatch
cargo build --release -p server --bin contextpatch-server
mkdir -p "$HOME/bin"
install -m 0755 target/release/contextpatch-server "$HOME/bin/contextpatch-server"
"$HOME/bin/contextpatch-server" --help
```

Rust is needed only to build the binary. Alternatively, upload a prebuilt binary for the exact
remote platform.

## Create a remote launcher

Create `~/bin/contextpatch-mcp` on the remote machine:

```sh
#!/bin/sh

PATH="$HOME/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
export PATH

exec "$HOME/bin/contextpatch-server" \
  --repo-root "/remote/path/to/repository" \
  --tool-surface project
```

Make it executable:

```sh
chmod 0755 "$HOME/bin/contextpatch-mcp"
```

Use absolute remote paths for the repository. The launcher must not print anything to stdout.
ContextPatch reserves stdout for MCP JSON-RPC responses; diagnostics belong on stderr.

`--tool-surface project` is recommended because it exposes one stable `project_execute` tool. Use
`--tool-surface full` only when the full direct-tool surface is required.

## Configure Claude Desktop

Edit the local Claude Desktop configuration:

```text
~/Library/Application Support/Claude/claude_desktop_config.json
```

Add an entry under `mcpServers`, preserving any existing entries:

```json
{
  "mcpServers": {
    "contextpatch-curtin": {
      "command": "/usr/bin/ssh",
      "args": [
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=15",
        "curtin",
        "exec \"$HOME/bin/contextpatch-mcp\""
      ]
    }
  }
}
```

The `curtin` alias resolves through the local SSH configuration; the Claude configuration does not
need to duplicate its hostname, username, or identity file.

Fully quit and reopen Claude Desktop after changing the configuration.

For multiple remote repositories, create one remote launcher and one named MCP entry per trust
boundary. A workspace-rooted project surface may instead use repository selectors when each selected
directory is an exact Git worktree root.

## Internal hosts and jump servers

An internal address such as the host behind `curtin-waf` may require a system VPN. If the approved
network design reaches it through a bastion, express that in `~/.ssh/config`, for example:

```sshconfig
Host curtin-waf
    ProxyJump curtin
```

Only add a jump host when it matches the institution's network and security policy. Claude Desktop's
SSH child uses the same VPN route and SSH configuration as `/usr/bin/ssh` in Terminal.

## Troubleshooting

### Claude reports that the MCP server disconnected

- Run the `BatchMode=yes` readiness command above.
- Use absolute paths in the remote launcher.
- Confirm the remote binary is executable and matches the remote platform.
- Remove output from remote `.profile`, `.bashrc`, `.zshrc`, or other startup files.
- Confirm that the launcher uses `exec` and prints nothing.

### SSH works interactively but not from Claude Desktop

The connection probably requires a prompt. Load the key into the macOS Keychain or use an
organization-approved non-interactive authentication method. If every login requires MFA, an
administrator-approved persistent SSH connection may be required; do not store an MFA response or
password in configuration.

### `Permission denied (publickey)`

Confirm that the public key is authorized remotely and that `BatchMode=yes` succeeds from Terminal.
`AddKeysToAgent yes` alone does not make an unavailable or unauthorized key usable.

### `Exec format error`

The uploaded binary targets the wrong operating system or CPU architecture. Build on the remote host
or upload the correct release artifact.

### A ContextPatch action cannot find `git`, `gh`, `cargo`, or another tool

Commands execute on the remote machine and inherit the launcher's `PATH`. Install the required tool
remotely or add its existing directory to the launcher. Do not emit environment diagnostics on
stdout.

### The internal host is unreachable

Connect the required VPN and verify the same alias with `/usr/bin/ssh`. Configure an approved
`ProxyJump` if the host is reachable only through a bastion.

## Security and limitations

- SSH provides transport encryption, host authentication, and user authentication. ContextPatch
  does not add TCP, HTTP, TLS, or SSH credential management.
- The remote server runs with the permissions of the remote SSH account.
- ContextPatch remains a narrow guarded repository tool, not a general remote administration or
  shell service.
- Do not enable SSH agent forwarding unless a reviewed workflow specifically requires it.
- A dropped SSH connection terminates the MCP server. Active background-job state may become
  unavailable after reconnection.
- Interactive remote prompts are unsupported and can make startup appear to hang.
