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

## Recommendation

Settle the Owner question before anything is installed, created, or assigned. In order:

1. Determine empirically whether Reader plus Cost Management Reader suffices for `execute_query` and
   the `CostManagement` toolset, with the deployment tools expected to fail. If they do, the constraint
   and the capability set are compatible and topology is settled as single-server ARM MCP.
2. If Owner genuinely is required for any tool this rollout uses, do not grant it against
   `ccl-pronunciation-trainer-rg`. Either narrow the capability set and record what is dropped, or move
   the evidence-gathering for cost to a human-run path outside MCP entirely.
3. Whichever way it resolves, apply `BlockingDeploymentsPolicy.json` at the target scope before first
   use, so the two default mutation tools are denied at ARM rather than merely unused.
4. Only then create the identity, assign roles, and configure the client.

The eight smoke checks stay as written. C35 remains open because checks 3 through 6 still have no
executable path that satisfies the identity constraints, and the constraints are not being relaxed to
manufacture one.

## ContextPatch is unchanged

No Azure dependency, no credential, no network authority was added. `npm install`, `npx`, and `az`
remain refused by policy with those refusals pinned by test, and writes outside the repository root
remain refused. See `docs/azure-workload-position.md` and `docs/execution-threat-model.md`.
