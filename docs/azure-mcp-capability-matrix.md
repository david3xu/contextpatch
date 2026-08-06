# Azure MCP capability matrix and topology recommendation

Evidence date 6 August 2026. Sources: the `2.0.5` Azure MCP binary executed directly, and the
`Azure/Azure-Resource-Manager-MCP` repository cloned at `HEAD`. The ARM MCP endpoint itself
(`mcp.management.azure.com`) was not contacted; its tool surface below is read from Microsoft's own
repository documentation, not from a live handshake.

## Capability matrix

| Required capability | Local Azure MCP 2.0.5 | ARM MCP (remote) |
| --- | --- | --- |
| Resource inventory and state | Yes: `group_list`, `group_resource_list`, `subscription_list` | Yes, via `execute_query` over Resource Graph |
| Resource Graph query | **No.** Zero of 235 tools | Yes: `execute_query`, plus `generate_query` and `validate_query` |
| Cost Management, actual spend | **No.** Zero of 235 tools. `pricing_get` is retail list pricing only | Yes: `CostManagement` toolset, documented for cost and usage queries, budgets, alerts, savings, and AKS cost split, with spend-this-month and forecast examples |
| Speech SKU inspection | **No.** Only `speech_stt_recognize` and `speech_tts_synthesize` | Probably, via `execute_query`, since Resource Graph exposes `sku` on resource records. **Not proven.** No dedicated tool exists and this needs a live query to confirm |
| ARM deployment state | **No.** Only `appservice_webapp_deployment_get`, `functions_template_get` | Yes: `get_arm_template_deployment_status` |

So the ARM MCP supplies four capability areas outright and the fifth probably, while local Azure MCP
supplies one. Step 4 of the investigation is answered: an actual-spend query does exist on a
Microsoft-controlled MCP surface, and it is the ARM MCP `CostManagement` toolset. No further surface
needs hunting, and retail pricing is not being substituted.

## ARM MCP: authentication, filtering, and mutation surface

**Default tool surface**, all six shipped on by default:

| Tool | Kind |
| --- | --- |
| `execute_query` | read |
| `generate_query` | read |
| `validate_query` | read |
| `get_arm_template_deployment_status` | read |
| `create_template_deployment` | **mutation** |
| `cancel_arm_template_deployment` | **mutation** |

**Optional toolsets**, off by default, opted into per client with an `x-mcp-toolset` header on the MCP
connection, comma separated: `CostManagement` and `Pricing`.

**Filtering is opt-in only.** The header adds optional toolsets. Nothing in the documentation removes
tools from the default set, and there is no `--read-only` equivalent. The two mutation tools cannot be
filtered off at the client.

**Microsoft's own mitigation is an ARM deny policy, not tool filtering.** The repository ships
`docs/BlockingDeploymentsPolicy.json`, an Azure Policy with `mode: All` that denies any request whose
`requestContext().identity.appid` equals `22bfbae3-f4e7-485f-be43-8cee15065084`, the ARM MCP first
party application. That is a stronger control than a client-side flag, because it denies at ARM rather
than hiding a tool, and it is the recommended way to neutralise the deployment tools. Assigning it
requires policy authority at the target scope.

**Authentication** is Entra on the remote endpoint. Third-party client setup requires an app
registration, manifest permissions, a web redirect URI, and `az ad sp create --id
22bfbae3-f4e7-485f-be43-8cee15065084` to materialise the service principal in the tenant. Creating
that service principal is a directory operation, not a subscription one.

## Three blockers on the ARM MCP

1. **It documents an Owner requirement, which contradicts a hard constraint.** Microsoft's other-client
   guidance states that the account used must have at least Owner permissions on at least one
   subscription for the tools to work properly. The rollout constraints forbid an Owner identity and
   specify Reader at `ccl-pronunciation-trainer-rg` plus Cost Management Reader at the narrowest
   workable scope. These cannot both hold as written.

   The claim may be over-broad rather than wrong. Resource Graph reads need Reader, and cost queries
   need Cost Management Reader; Owner is plausibly required only by the deployment tools, which this
   rollout does not want. That is testable, and testing it is the fastest way to unblock. It cannot be
   tested from here.

2. **Claude Desktop is not a supported client.** The README states that during preview the server works
   with GitHub Copilot Chat in VS Code and GitHub Copilot CLI. Third-party access is documented
   separately and points at a claude.ai custom connector, with a redirect URI of
   `https://claude.ai/api/mcp/auth_callback/`. That is a hosted connector configured against an app
   registration and client secret, not a local Claude Desktop stdio entry. It is a materially different
   integration from the one this runbook was written for, and it reintroduces a client secret, held at
   the connector rather than in Claude Desktop JSON.

3. **It is preview**, with an expanding surface, and the mutation tools are on by default.

## Smallest complete topology

**Target: one server, ARM MCP only. Drop local Azure MCP.**

The ARM MCP's `execute_query` covers resource inventory over Resource Graph, which makes all three of
local Azure MCP's inventory tools redundant. Running both would mean two identities, two credential
paths, two update cadences, and a larger aggregate tool surface, in exchange for nothing the single
server does not already do. A two-server topology is therefore rejected on evidence rather than taste.

That target is **not yet executable**, because of blocker 1 above. Under the stated identity
constraints, the required capability set has no executable path today: the surface that can answer the
cost, graph, and deployment questions documents an Owner requirement, and the surface that respects
Reader-only scoping cannot answer them.

## Source-level authentication and RBAC audit

Audit date 6 August 2026, before any live connection. One finding governs the rest.

### A source-level audit of ARM MCP is not possible

The repository contains **zero source files**. Complete inventory: ten markdown files, one JSON policy,
two GitHub issue templates, and media. No `.cs`, `.ts`, `.js`, `.py`, `.go`, project or solution files.
It is a feedback and issue-tracking repository for a hosted service whose implementation is not
published.

So "enforced in code" cannot be determined for any claim below. What follows is the strongest available
evidence, labelled by kind, and nothing is upgraded to proof that is not proof.

### 1. Is "Owner required" enforced, or stated for the deployment workflow?

**Undetermined, and the documentation contradicts itself.**

`docs/OtherClientSupport.md` step 7 states the account must have at least Owner permissions on at least
one subscription for the tools to work properly. `docs/FAQ.md` states the opposite model twice: that the
server operates on behalf of the signed-in user so that account's Azure role and permissions determine
what it can query or manage, and that the server respects Azure's authorization layer and will deny the
operation if the account lacks permission.

The FAQ position is the architecturally coherent one, and a third FAQ answer reinforces it by suggesting
an account with *subscription write access* specifically for cross-tenant querying. The Owner sentence
reads as operational guidance covering the full deploy demo rather than an enforcement gate. That is an
inference, not a finding, and it cannot be settled without source or a live call.

### 2. OAuth flow and effective Azure principal

**Determined from the documented app-registration manifest: the effective principal is a delegated human
account.**

The required manifest entry grants `resourceAppId 22bfbae3-f4e7-485f-be43-8cee15065084`, permission id
`601b3b64-a3c4-4faa-ab9c-8f97ba332aa3`, with `"type": "Scope"`. In Entra manifests `Scope` denotes a
**delegated** permission; an application permission usable by client credentials would be `"type":
"Role"`. No `Role` entry is documented.

The rest of the flow matches: a web redirect URI, an interactive Connect-and-sign-in step, and tenant
switching performed through an interactive account picker. The FAQ confirms operation on behalf of the
signed-in user.

The app registration and client secret are therefore the **OAuth client**, not the Azure principal. The
secret authenticates the connector application to Entra; it does not make that application the identity
acting against ARM. Every ARM call carries the delegated human account's authorization.

### 3. Client credentials or certificate authentication for third-party clients?

**Not supported as documented.** Only the delegated `Scope` permission appears. No client-credentials
flow is described. Certificates are never mentioned; step 5 directs the operator to the Certificates and
secrets blade but specifically to create a client secret. There is no documented path by which a
dedicated service principal becomes the acting Azure identity.

### 4. RBAC action mapping

The ARM MCP documentation does not enumerate RBAC actions, so this mapping is **derived from Azure's
platform model**, not read from a Microsoft-controlled source at a pinned version. Treat it as a
hypothesis to verify, not as evidence.

| Tool | Required action, derived | Covered by Reader | Covered by Cost Management Reader |
| --- | --- | --- | --- |
| `execute_query` | `Microsoft.ResourceGraph/resources/read` plus read on target resources | yes | no |
| `get_arm_template_deployment_status` | `Microsoft.Resources/deployments/read` | yes | no |
| `CostManagement` queries | Cost Management query and read actions | no | yes |
| `create_template_deployment` | `Microsoft.Resources/deployments/write` plus write per resource type | **no** | no |
| `cancel_arm_template_deployment` | `Microsoft.Resources/deployments/cancel/action` | **no** | no |

### 5. Would Reader plus Cost Management Reader deny create and cancel?

**Strongly supported, not proven.** Proof needs either source, which does not exist, or a live call,
which this audit precedes.

The supporting argument is platform-level rather than server-level: every create, update, and delete
flows through ARM regardless of client, ARM evaluates RBAC server-side, and Reader confers read actions
only. `deployments/write` and `deployments/cancel/action` are therefore absent, so ARM denies. The
vendor states the same conclusion, that the server respects Azure's authorization layer and denies when
the account lacks permission.

The consequence matters for governance hygiene: **if the roles are scoped correctly, the shipped
`BlockingDeploymentsPolicy.json` is redundant.** That policy exists to neutralise the deployment tools
for accounts that are over-privileged, which is precisely the situation the Owner guidance would create.
It must not be assigned as compensation for granting roles wider than the workload needs.

### 6. Does the hosted custom connector surface in Claude Desktop?

**Yes.** Anthropic's documentation states that custom connectors using remote MCP are available on
Claude, Cowork, and Claude Desktop across Free, Pro, Max, Team and Enterprise plans. This condition
passes.

Two details change the trust boundary and belong on the record. Remote connectors are configured and
brokered through the Claude account, and the connection to the MCP server originates from Anthropic's
servers rather than the local machine, which is true even in Claude Desktop; local servers configured
through `claude_desktop_config.json` are a separate mechanism that does use the local network. So an ARM
MCP connector is not the local, machine-scoped arrangement the rest of this rollout assumes.

Also practical: the `x-mcp-toolset` header that enables `CostManagement` would need Anthropic's request
header authentication feature, which is documented as **beta and rolled out on request**. Without it
there is no documented way to set that header, so the cost capability is gated behind a beta feature.

## Decision: reject ARM MCP

Applying the decision rule, condition by condition.

| Condition | Result |
| --- | --- |
| Requires a delegated Owner identity | **Undetermined.** Documentation self-contradictory, no source, not live-testable yet |
| Cannot use the dedicated scoped identity | **FAILS.** Delegated `Scope` permission only; no client-credentials or certificate path; acting principal is always the signed-in human |
| Cannot surface in Claude Desktop | Passes |

ARM MCP is **rejected**, on the second condition. The identity model is delegated-user by construction,
so the dedicated scoped service principal this rollout is built around cannot be the acting Azure
principal. That is not a configuration gap to be tuned; it is the server's authentication design, and no
role scoping, toolset header, or deny policy changes it.

Rejecting on condition two also makes condition one moot. Whether Owner is truly required stops
mattering once the identity cannot be a dedicated service principal at all.

Nothing is to be created. No app registration, no client secret, no Owner assignment, no policy
assignment, per the decision rule.

## Where this leaves the capability set

Both candidate surfaces are now eliminated for the required set. Local Azure MCP 2.0.5 respects a
Reader-scoped service principal but lacks four of five capabilities. ARM MCP has the capabilities but
cannot act as a dedicated scoped identity.

So the required capability set has **no executable path across Microsoft's currently available MCP
surfaces** under these identity constraints. The honest options are to change one of the inputs:

1. Take cost, Resource Graph, and deployment evidence through a human-run path outside MCP, with the
   operator running the queries under their own identity and recording the output. This preserves the
   identity constraint and abandons the automation goal for those four checks.
2. Accept a delegated human identity for a read-only ARM MCP connector, scoped as tightly as Entra and
   RBAC allow, and record the deviation from the dedicated-service-principal rule as an explicit,
   reviewed exception rather than an oversight. Only worth considering if the deployment tools can be
   shown denied by role, since the policy must not be the compensating control.
3. Deploy local Azure MCP for inventory alone, and accept that checks 3 through 6 are out of scope for
   MCP, updating the C35 definition to match what was actually decided rather than what was hoped.

Option 1 is the one that costs nothing and breaks no constraint. Option 3 is the smallest thing that
still installs software. Option 2 should not be chosen without an explicit written exception.

C35 stays open. Checks 3 through 6 have no executable path, and the constraints are not being relaxed
to manufacture one.

## ContextPatch is unchanged

No Azure dependency, no credential, no network authority was added. `npm install`, `npx`, and `az`
remain refused by policy with those refusals pinned by test, and writes outside the repository root
remain refused. See `docs/azure-workload-position.md` and `docs/execution-threat-model.md`.
