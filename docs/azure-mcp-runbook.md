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

## Decision: do not install yet

Two gaps still block installation. Neither is the CVE.

1. **Platform-version skew is unresolved for the line that would be pinned.** Skew was observed on
   the `3.0.0-beta.x` line, where `@azure/mcp-darwin-arm64` sat at `beta.13` while several siblings
   were at `beta.27`. Whether `@azure/mcp-darwin-arm64` publishes a matching `2.0.2` has not been
   checked. On Apple Silicon that package supplies the binary, so it decides what executes and must
   be confirmed at the exact pinned version.
2. **The tag, commit, and publish workflow for the pinned version are still unrecorded.** Release
   tags use the `Azure.Mcp.Server-` prefix and the filtered release list is reachable, but the exact
   newest tag, its commit SHA, and the workflow that publishes to npm were not captured. GitHub
   blocks automated access to the raw changelog and rejects the query-string release URL, so these
   need a browser. Without the commit, a pinned npm version cannot be tied to reviewed source.

The Microsoft Artifact Registry image is still **not** adopted, though the picture improved: 2.0
ships AMD64 and ARM64 Docker images with trimmed binaries. The standing condition is unchanged.
Microsoft must identify a distribution as current and its digest must be pinnable.

## A distribution worth checking before anything else

The same third-party review reports an **MCP Bundle** format, `.mcpb`, dated 24 April 2026, which
installs Azure MCP into Claude Desktop by dragging one file and requires no Node.js, Python, or .NET
runtime. If Microsoft confirms that, it is materially better than the npm path for this rollout: no
platform package skew, no `npx` resolution at launch, no Node toolchain on the host, and a single
artifact that can be hashed and archived.

This is unverified and comes from a non-Microsoft source. Confirm it against the server README and
Microsoft Learn before treating it as an option.

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
