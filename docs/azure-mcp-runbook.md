# Azure MCP companion runbook

Status: repository deliverables ready, host setup and live verification outstanding (C35 open).

ContextPatch holds zero Azure authority and must keep holding zero. See
`docs/azure-workload-position.md`. Azure access lives in a separate Claude Desktop entry running
Microsoft's Azure MCP Server, outside this repository and outside ContextPatch's authority
boundary. ContextPatch cannot install packages, cannot run `az` or `npx`, and cannot write outside
its repository root; those refusals are pinned by test in `crates/core/src/process/guarded_command.rs`.

## Provenance trace

Traced 6 August 2026 against Microsoft-controlled sources only.

The archive is a planned migration, not abandonment. The `Azure/azure-mcp` README states that the
repository is archived and that going forward the Azure MCP team uses `github.com/microsoft/mcp` for
the code and issues formerly in that repository. Note the two dates: the README gives the archive
announcement as 25 August 2025, while GitHub's banner records the flip to read-only on 6 February
2026. The announcement preceded the enforcement by months, which is why the release list stops at
`0.5.8` while npm kept publishing.

The current source of record is therefore:

| Item | Value |
| --- | --- |
| Repository | `github.com/microsoft/mcp` |
| Source path | `servers/Azure.Mcp.Server` |
| Changelog | `servers/Azure.Mcp.Server/CHANGELOG.md` |
| Release tag prefix | `Azure.Mcp.Server-` |
| Documentation | `learn.microsoft.com/azure/developer/azure-mcp-server/` |
| Troubleshooting and support | `servers/Azure.Mcp.Server/TROUBLESHOOTING.md`, `SUPPORT.md` |
| npm package | `@azure/mcp` |
| npm version observed at `latest` | `3.0.0-beta.31` |
| VS Code extension | `ms-azuretools.vscode-azure-mcp-server` |

`microsoft/mcp` publishes per-server release tags, so Azure MCP releases are found by filtering
releases on the `Azure.Mcp.Server-` prefix rather than by reading the repository's latest release,
which belongs to whichever server published most recently.

## Decision: do not install yet

A source-to-package mapping now exists and is authoritative. Two gaps still block pinning, and both
are narrower than the questions this section previously recorded.

1. **The tag to package correspondence is unverified.** Nothing consulted states which
   `Azure.Mcp.Server-<version>` tag corresponds to npm `3.0.0-beta.31`. Pinning an npm version
   without that mapping means pinning a build whose source commit and changelog entry cannot be
   identified, which defeats the point of pinning.
2. **No general-availability release was found, and the platform packages are skewed.** Every
   release on the archived repository was marked pre-release and the current npm `latest` is a beta.
   `@azure/mcp-darwin-arm64` was observed at `3.0.0-beta.13` while `@azure/mcp-darwin-x64` and
   several siblings were at `3.0.0-beta.27`, and `@azure/mcp-linux-x64` at `2.0.0-beta.33`. On Apple
   Silicon the platform package supplies the binary, so the skew decides what actually executes.

The Microsoft Artifact Registry image is **not** adopted. It appeared only in release notes on the
archived repository, and nothing consulted identifies it as the current supported distribution. The
standing condition for using it is unchanged: Microsoft must identify it as current and its digest
must be pinnable.

## Alternative worth evaluating before installing anything

`microsoft/mcp` also catalogues **Azure Resource Manager MCP** at `Azure/Azure-Resource-Manager-MCP`,
described as providing tools to use Azure Resource Graph to retrieve and filter information about
resources in a subscription, plus tools to manage ARM template deployments. It is a remote server at
`https://mcp.management.azure.com`.

That covers three of the five required capabilities, namely resource inspection, Resource Graph, and
ARM deployment inspection, with no local install, no npm version to pin, and no platform binary
skew. It does not cover Cost Management or the Speech pricing tier, so it is not a full replacement.
A smaller remote surface for the three capabilities it does cover may still be the better trade than
a local beta, and it should be evaluated before the local package is installed. Note that its
deployment management tools are mutating and must be excluded under the read-only scope.

Record the resolution here before installing:

| Field | Value |
| --- | --- |
| Chosen surface | to be decided |
| Pinned version | |
| Source tag and commit for that version | |
| Reason for that version | |
| Platform package and version actually resolved | |
| Tarball SHA-512 integrity from the registry | |
| Reviewed by, date | |

Capture integrity from the registry metadata rather than trusting the tag, and record the value
above so a later reinstall can be compared against it.

## Startup command

The documented invocation is `server start`. Microsoft's published client configuration uses
`npx -y @azure/mcp@latest server start`. That form is rejected here on two counts: `@latest` floats,
and `npx -y` resolves and executes from the network on every launch. Use a controlled installation
at an exact version and invoke its binary directly.

## Tool filtering

The server supports `--mode` and `--namespace`. Mode `all` exposes every tool individually and can
exceed client tool limits; from `0.5.0` onward the default listing groups tools by namespace, which
reduces the advertised count. Tool counts have grown quickly, reported as 118 at `0.5.7` and 131 at
`0.5.8`, so an unfiltered surface is both large and moving.

The exact namespace tokens are not recorded here on purpose. They are build-specific and this
document will not guess them. After installing the pinned version, capture the real list from the
binary's own help output and fill in the table below, then use only those tokens in the Claude
Desktop entry.

| Required capability | Namespace token for the pinned build |
| --- | --- |
| Resource listing and state | |
| Resource Graph query | |
| Cost Management read | |
| Speech pricing tier inspection | |
| ARM deployment inspection | |

Do not enable broad mutation namespaces in this rollout. Enable the smallest set that answers the
five capabilities above and nothing else.

## Authentication

The server authenticates through Microsoft Entra ID and resolves credentials with
`DefaultAzureCredential`, which discovers an identity from local developer tooling such as the
Azure CLI. It can only perform operations the resolved identity is authorised to perform, so role
assignment is the enforcement boundary, not tool filtering.

That has a consequence worth stating plainly: because the credential is ambient, a plain `az login`
would hand the server whatever authority the interactive user holds. The rollout therefore uses a
dedicated service principal signed in through an isolated Azure CLI profile, so the companion
server cannot silently inherit an Owner identity.

Isolation is achieved with `AZURE_CONFIG_DIR` pointing at an operator-owned directory used only by
this server. That path is not a secret and may appear in Claude Desktop configuration. Certificates,
secrets, and tokens must never appear in Claude Desktop configuration or in this repository.

Prefer certificate authentication. If a secret must be used, use an entry method that keeps it out
of shell history.

## Identity and role scope

| Scope | Role | Purpose |
| --- | --- | --- |
| `ccl-pronunciation-trainer-rg` | Reader | resource listing, resource state, deployment inspection, Speech tier |
| narrowest scope that answers the cost query | Cost Management Reader | daily spending evidence |

No Owner. No Contributor. No role-assignment authority. Contributor belongs to a later, separately
approved mutation workflow, if one is ever approved.

Out-of-scope reads and mutation attempts are expected to fail by Azure authorization. Those two
failures are smoke checks 7 and 8, and they are the evidence that scoping works.

Microsoft's own listing for this server makes the same point from the other direction: clients invoke
operations according to the signed-in identity's Azure RBAC permissions, and it warns that autonomous
or misconfigured clients may perform destructive actions. Role scope is therefore the control, and
tool filtering is only a convenience on top of it. A filtered tool list with an over-privileged
identity is not a least-privilege deployment.

Do not enable the server's support-logging option, which is explicitly named as dangerous upstream
and writes logs to a directory. Diagnostics that capture cloud responses do not belong in this
rollout.

## Installation outside this repository

Install to an operator-owned directory that is not inside any repository ContextPatch manages, so
the install can never be confused with repository content. Pin the exact version, do not use a
floating tag, and keep a lockfile or an exact global version so the resolved build is reproducible.

Record the install location, the resolved binary path, and the integrity value in the table above.

**Upgrade.** Install the new exact version alongside the current one, point a test Claude Desktop
entry at it, re-run all eight smoke checks, and only then repoint the primary entry. Never upgrade
by moving a floating tag.

**Rollback.** Repoint the Claude Desktop entry at the previous exact version and restart Claude
Desktop. Keeping the prior version installed until the new one has passed smoke checks is what makes
rollback a one-line change rather than a reinstall.

**Uninstall.** Remove the Azure MCP entry from Claude Desktop, restart, confirm the tools are gone,
then remove the install directory and the isolated `AZURE_CONFIG_DIR`. Removing the credential
directory is part of uninstall, not an afterthought: leaving it behind leaves a usable identity
cache on disk.

## Claude Desktop configuration

Back up the existing configuration before touching it. Preserve every existing entry, including all
ContextPatch entries; the Azure entry is additive.

A redacted template is at `docs/examples/claude-desktop-azure-mcp.json`. It contains placeholders
only. Do not commit the live configuration to this repository.

The ContextPatch CLI already has a `configure_claude_desktop` command whose tests cover dry run,
backup, stale-read refusal, and preservation of unrelated entries. Prefer it over hand-editing JSON,
and run it in dry-run mode first.

## Live smoke checks

All eight must pass before C35 is marked complete. Record the date and the observed result for each.

| # | Check | Result |
| --- | --- | --- |
| 1 | List resources in `ccl-pronunciation-trainer-rg` | |
| 2 | Inspect current resource state | |
| 3 | Query Resource Graph | |
| 4 | Read daily Cost Management evidence for the spending floor | |
| 5 | Confirm the Speech pricing tier | |
| 6 | Inspect ARM deployment state | |
| 7 | Out-of-scope resource read is denied by Azure authorization | |
| 8 | Mutation tool is absent, or fails by role enforcement without changing state | |

Checks 1 through 6 are reads. Checks 7 and 8 must fail, and the failure must come from Azure
authorization rather than from client-side filtering, or the scoping is unproven.

Do not throttle Foundry, deploy Bicep, change the Speech SKU, or mutate any Azure resource as part
of verification. The operational pilot in `docs/hardening-plan.md` keeps mutations human-owned.

## Operator checklist

1. Resolve the two version findings above and pin a version.
2. Create or select the dedicated service principal, preferring certificate credentials.
3. Assign Reader at the resource group and Cost Management Reader at the narrowest workable scope.
4. Install the pinned version outside this repository and record integrity.
5. Capture the real namespace tokens from the pinned build and fill in the filtering table.
6. Sign in through the isolated `AZURE_CONFIG_DIR` profile.
7. Back up Claude Desktop configuration, add the Azure entry, restart.
8. Confirm the advertised tool list is restricted to the intended namespaces.
9. Run all eight smoke checks and record results.
