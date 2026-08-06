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
| npm stable version confirmed published | `2.0.2`, at `npmjs.com/package/@azure/mcp/v/2.0.2` |
| npm `latest` tag | a `3.0.0-beta.x` prerelease; snippets observed both `beta.27` and `beta.31` within the same day |
| GA status | Azure MCP Server 2.0 is generally available |
| GA announcement | `devblogs.microsoft.com/azure-sdk/announcing-azure-mcp-server-2-0-stable-release/` |
| GA date | 10 April 2026 |
| Server README | `github.com/microsoft/mcp/tree/main/servers/Azure.Mcp.Server` |
| Release filter | `github.com/microsoft/mcp/releases?q=Azure.Mcp.Server-` |
| VS Code extension | `ms-azuretools.vscode-azure-mcp-server` |

`microsoft/mcp` publishes per-server release tags, so Azure MCP releases are found by filtering
releases on the `Azure.Mcp.Server-` prefix rather than by reading the repository's latest release,
which belongs to whichever server published most recently.

## Decision: do not install yet

The source-to-package mapping is now authoritative, and one earlier conclusion in this document was
**wrong and is retracted**: it recorded that no general-availability release existed. That was an
artifact of reading only the archived repository, whose versions never passed `0.5.x`. Versioning
continued in `microsoft/mcp` with a 1.0 stable release and then a 2.0 stable release. The server
README states that Azure MCP Server 2.0 is generally available, and Microsoft's Azure SDK blog
announced the 2.0 stable release on 10 April 2026, reporting 276 tools across 57 Azure services with
self-hosted remote deployment as the defining change. The changelog frames 2.0 as spanning 40 beta
iterations, `2.0.0-beta.1` dated 2025-10-29 through `2.0.0-beta.40` dated 2026-04-07.

So GA status is resolved, and a stable line exists to pin: `@azure/mcp` `2.0.2` is confirmed
published. Microsoft also documents pinning directly, the server README advising a pinned version
rather than `@latest` when startup is slow.

## CVE-2026-32211: verified real, retracted as a blocker

Verified 6 August 2026. The CVE exists. The characterisation that reached this document, that patching
was pending, was **false**, and it is retracted along with the blocker status it justified.

Evidence:

| Source | URL | Finding |
| --- | --- | --- |
| MSRC Security Update Guide | `msrc.microsoft.com/update-guide/vulnerability/CVE-2026-32211` | Azure MCP Server Information Disclosure Vulnerability, released 2 April 2026, assigning CNA Microsoft |
| GitHub Advisory Database | `github.com/advisories/GHSA-5w7p-v6h9-q8c5` | Critical, Unreviewed, published 3 April 2026, no package listed |
| NVD | `nvd.nist.gov/vuln/detail/CVE-2026-32211` | published by the National Vulnerability Database |
| CVE.org | `cve.org/CVERecord?id=CVE-2026-32211` | record exists |

MSRC states that the vulnerability has already been fully mitigated by Microsoft, that there is no
action for users of this service to take, and that the CVE exists to provide transparency under
Microsoft's cloud service CVE programme. The record's own summary line says the vulnerability
requires no customer action to resolve.

Against the four questions that were asked:

**Affected component and versions.** Weakness is CWE-306, missing authentication for a critical
function. No version range is enumerated anywhere consulted. Third-party classification places the
affected surface in the cloud service rather than a distributable artefact, associating it with Azure
Web Apps, which Microsoft patches and manages directly.

**Whether `@azure/mcp@2.0.2` is affected.** No. More precisely, no package in any ecosystem is
listed as affected. The GitHub advisory records no package and no affected or fixed version, and says
in terms that Dependabot alerts are unsupported for it because it has no package from a supported
ecosystem with an affected and fixed version. There is no evidence that the npm package is in scope
at any version.

**Fixed version or commit.** None published, because remediation was service-side. The CVSS vector
carries `RL:O`, official fix, alongside `E:U`, unproven exploit maturity, and `RC:C`, confirmed report
confidence. Full vector: `CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:N/E:U/RL:O/RC:C`, base 9.1,
temporal 7.9.

**Local, remote, or both.** The evidence points to a network-reachable service deployment, not a
local stdio process. `AV:N` with `PR:N` and `UI:N` describes an unauthenticated network attacker, and
missing authentication is not a meaningful condition for a server that speaks over stdin and stdout
to a single parent process. This is an inference from the vector plus the cloud-service framing rather
than an explicit statement by Microsoft, so treat it as strong but not certain. It carries one
forward-looking consequence: if a self-hosted remote deployment is ever considered, this CVE class
becomes directly relevant and the authentication posture must be reviewed then. The planned local
stdio rollout is not in its scope.

One note on source quality, since it is the reason this appeared as a blocker at all. At least one
widely circulated post claims no patch is available and attributes the CVE to `@azure-devops/mcp`,
the Azure DevOps MCP server, which is a different product from the Azure MCP Server. That post is
wrong on both points and closes with a vendor pitch. Where MSRC and the third-party summaries
disagree, MSRC governs.

## Resolved: pinned artifact

Verified 6 August 2026 from npm registry metadata, git protocol, GitHub release assets, and by
executing the shipped binary. Both earlier gaps are closed. The `2.0.2` candidate named above was
simply stale.

**Pin `2.0.5`**, newest stable, published 2026-07-10. Stable line: `2.0.0` through `2.0.5`.

**The `latest` dist-tag points to `3.0.0-beta.32`, a prerelease.** So `npm install @azure/mcp` with no
version installs a beta. That is the concrete reason the floating tag is refused here.

| Item | Value |
| --- | --- |
| Git tag | `Azure.Mcp.Server-2.0.5` |
| Tag commit | `f43b47a21545e5f3f87b3bceee35986442217bb4` |
| Build commit embedded in binary | `2712e19ddf1c55f8e73ead8fb671915ec92801cc` |
| `--version` output | `2.0.5+2712e19ddf1c55f8e73ead8fb671915ec92801cc` |
| npm repository field | `git+https://github.com/microsoft/mcp.git` |
| Binary name | `azmcp` |
| Node requirement, npm path only | `>=20.0.0` |

The two commits differ. The tag resolves to `f43b47a`, the binary reports `2712e19`. Record both; the
build commit describes what actually executes.

### Integrity digests

| Artifact | Digest |
| --- | --- |
| `Azure.Mcp.Server-osx-arm64.mcpb` | SHA-256 `b49909a925f7e4fade530098e68f5d81bf2522f083ea9b5c5534f4be60a0c348`, 51,687,927 bytes |
| `Azure.Mcp.Server-linux-x64.mcpb` | SHA-256 `2ae7ede16013d2646bb50ad10e9c9f4807117242ec9d5c0979521528c563326d` |
| npm `@azure/mcp@2.0.5` | `sha512-o451hyeCa9u1jr1zYNi3OpF560IRH7LkHQHr7uOjWgaXNgF8m8aNbuxLdqs1PJcJEfrib4W8vVl0GY5X1fXtsw==` |
| npm `@azure/mcp-darwin-arm64@2.0.5` | `sha512-cK+Q9InsmgI8bq+rHpkbIu+I4Ha9/p3A5hAfchjdEkSepvpc0IgL+OBKLDuO3mYtp345Z+/elfKkFXKu+9b51g==` |

Asset URL pattern, confirmed reachable:
`github.com/microsoft/mcp/releases/download/Azure.Mcp.Server-2.0.5/Azure.Mcp.Server-osx-arm64.mcpb`

### Platform skew: resolved, and never real on the stable line

The `2.0.5` wrapper pins all six platform packages to exactly `2.0.5`, not ranges.
`@azure/mcp-darwin-arm64@2.0.5` exists with `os: [darwin]`, `cpu: [arm64]`. The skew reported earlier
was an artifact of search snippets cached at different times.

### Two residual risks, recorded rather than resolved

1. **No npm provenance attestations.** Every stable `2.0.x` release has `attestations: null`. A
   registry signature exists, but nothing cryptographically binds package to source build. The
   source-to-package link rests on the binary's embedded commit, not on verifiable provenance.
2. **`2.0.5` has no changelog entry.** At its own tag the changelog's highest `2.x` entry is
   `2.0.2 (2026-04-24)`, with no entries for `2.0.3`, `2.0.4`, or `2.0.5`, and the file opens with
   `3.0.0-beta.25 (Unreleased)`. What changed between `2.0.2` and `2.0.5` is undocumented. If
   documented change coverage outweighs recency, `2.0.2` is the alternative.

The Microsoft Artifact Registry image is still **not** adopted. 2.0 does ship AMD64 and ARM64 Docker
images, but the standing condition is unchanged: Microsoft must identify a distribution as current
and its digest must be pinnable.

## MCPB: officially supported, and rejected for this rollout

`.mcpb` is real and Microsoft-documented, not a third-party rumour. The server README describes MCP
Bundles as portable server versions installable directly into clients like Claude Desktop, produced
per platform and architecture, containing the binary and all dependencies so no Node.js or .NET is
needed. Install is by dragging the file into Claude Desktop, or via Settings, Extensions, Advanced
Settings, Install Extension.

It is nonetheless **not selectable here**, on evidence from the bundle's own manifest:

```
manifest_version 0.3, version 2.0.5
server.type         binary
entry_point         server/azmcp
mcp_config.command  ${__dirname}/server/azmcp
mcp_config.args     ["server", "start"]
mcp_config.env      {}
user_config         absent
```

Args are fixed at `server start`, environment is empty, and there is no `user_config` block. A bundle
install therefore cannot pass `--read-only`, cannot pass `--namespace`, and cannot set
`AZURE_CONFIG_DIR`. Installing it would produce exactly the failure this design exists to prevent: an
unfiltered, write-capable server resolving credentials from whatever ambient Azure CLI profile the
operator happens to hold.

**Recommended path instead:** download the pinned `.mcpb`, verify the SHA-256 above, extract it to an
operator-owned directory, and point Claude Desktop at `<dir>/server/azmcp` with explicit args and env.
That keeps the hash-pinned Microsoft artifact and avoids a Node toolchain, while restoring flag
control. Two caveats: extracting a bundle and invoking the binary directly is not a documented
Microsoft installation method, and on macOS the extracted binary may be quarantined, so verify code
signing and notarisation before assuming it launches.

## Verified startup and filtering options

From `azmcp server start --help` on the `2.0.5` Linux build, so these are the real options for this
exact version rather than documentation inference. Worth noting: the README at this tag surfaces
namespace filtering and read-only only as VS Code extension settings, so the binary is the authority.

| Option | Meaning |
| --- | --- |
| `--transport` | transport mechanism, default `stdio` |
| `--namespace <namespace>` | service namespaces to expose, for example `storage`, `keyvault`, `cosmos` |
| `--mode <mode>` | `single`, `namespace` (default), or `all` |
| `--tool <tool>` | expose only named tools, repeatable, forces `all` mode, **mutually exclusive with `--namespace`** |
| `--read-only` | no write operations allowed |
| `--cloud <cloud>` | `AzureCloud` default, `AzureChinaCloud`, `AzureUSGovernment`, or custom authority URL |
| `--outgoing-auth-strategy` | `NotSet` default, `UseHostingEnvironmentIdentity`, `UseOnBehalfOf` |
| `--env-file` | load environment from a file |
| `--debug` | verbose logging to stderr |

`--read-only` is a material improvement on the original design, which relied on RBAC alone. Use both:
`--read-only` blocks writes at the server, Reader-only roles block them at Azure. Check 8 still proves
the RBAC layer independently, which matters because a server-side flag is not an authorisation
boundary.

**Never enable these.** Each is named dangerous by Microsoft and each defeats part of this design:
`--dangerously-disable-http-incoming-auth`, `--dangerously-disable-elicitation` (removes confirmation
before high-risk commands such as returning Key Vault secrets),
`--dangerously-write-support-logs-to-dir` (may write sensitive data to disk),
`--dangerously-disable-retry-limits`.

One namespace to exclude deliberately: the bundle ships an `Azure.Mcp.Tools.Authorization` module, so
role-assignment tooling exists in the product and must not appear in the enabled namespace set.

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
