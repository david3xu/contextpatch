# Azure evidence runbook, operator-only

Status: active. This is the Phase 0 fallback path, adopted after both candidate MCP surfaces were
rejected. See `docs/azure-mcp-capability-matrix.md` for why.

No MCP server is involved. The operator runs each command, and each command writes its own output into
an evidence directory. ContextPatch has no part in any of it and gains no Azure authority.

## What this replaces

Local Azure MCP `2.0.5` supplied one of five required capabilities. ARM MCP supplied the rest but is
delegated-user by construction, so a dedicated scoped service principal cannot be its acting identity.
Running `az` directly under that dedicated identity satisfies every capability and keeps the identity
constraint intact, which no MCP topology managed.

## Verification status of this document

The procedures are structured so each step produces its own evidence, and step 0 checks syntax before
any authenticated call. That structure is the verification mechanism. What is **not** claimed: nobody
has executed these commands against the target subscription, and no output is recorded yet. Command
syntax comes from Azure CLI documentation, so step 0 exists to catch drift before it matters.

## Prerequisites

| Item | Value |
| --- | --- |
| Identity | dedicated Entra service principal, certificate credential preferred |
| Role 1 | Reader at `ccl-pronunciation-trainer-rg` |
| Role 2 | Cost Management Reader at the narrowest scope that answers the cost query |
| Explicitly not granted | Owner, Contributor, any role-assignment authority |
| CLI extensions | `resource-graph`, `costmanagement` |

The two roles are the whole authorization boundary. Steps 6 and 7 exist to prove that boundary holds,
so do not widen it to make them pass.

## Isolated profile and evidence directory

```
export AZURE_CONFIG_DIR="$HOME/.azure-ccl-reader"
mkdir -p "$AZURE_CONFIG_DIR" && chmod 700 "$AZURE_CONFIG_DIR"

EVIDENCE="$HOME/azure-evidence/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$EVIDENCE" && chmod 700 "$EVIDENCE"
```

`AZURE_CONFIG_DIR` must never be the default `~/.azure`. Its purpose is to guarantee that nothing here
can pick up the operator's ordinary interactive Azure identity. Set it in the shell that runs these
commands, not in a shell profile, so it cannot leak into unrelated work.

The evidence directory is operator-owned and **is not committed to this repository**.

## Capture wrapper

Every step below runs through this, so that output, exit code, and command line are all recorded
together and redacted on the way in.

```
redact() {
  sed -E \
    -e 's/("(accessToken|access_token|refresh_token|id_token|password|clientSecret|client_secret|key[0-9]?|primaryKey|secondaryKey|connectionString|sas|signature)"[[:space:]]*:[[:space:]]*)"[^"]*"/\1"[REDACTED]"/gI' \
    -e 's/(Bearer )[A-Za-z0-9._-]+/\1[REDACTED]/g' \
    -e 's/eyJ[A-Za-z0-9._-]{20,}/[REDACTED_JWT]/g'
}

cap() {
  local name="$1"; shift
  local out="$EVIDENCE/${name}.txt"
  {
    echo "# command: $*"
    echo "# utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "# ---"
  } > "$out"
  set +e
  "$@" 2>&1 | redact >> "$out"
  local rc=${PIPESTATUS[0]}
  set -e
  echo "# exit_code: $rc" >> "$out"
  chmod 600 "$out"
  echo "$name exit=$rc"
}
```

Redaction is a safety net, not a licence. Do not pass secrets on command lines, and never run
`az account get-access-token` inside `cap`.

## Step 0: syntax check before authenticating

```
for c in "resource list" "graph query" "costmanagement query" \
         "cognitiveservices account show" "deployment group list" \
         "deployment group what-if"; do
  az $c --help > /dev/null 2>&1 && echo "ok: az $c" || echo "CHECK FAILED: az $c"
done
```

Any failure means a missing extension or changed syntax. Fix it before signing in.

## Step 1: sign in as the dedicated identity

Certificate preferred:

```
az login --service-principal \
  --username "$APP_ID" \
  --certificate "$CERT_PEM_PATH" \
  --tenant "$TENANT_ID"
```

If a secret must be used instead, read it from a prompt so it never enters shell history:

```
read -rs -p "client secret: " SP_SECRET && echo
az login --service-principal --username "$APP_ID" --password "$SP_SECRET" --tenant "$TENANT_ID"
unset SP_SECRET
```

Confirm the acting identity is the service principal and not a human account:

```
cap 00-identity az account show --query '{id:id, tenantId:tenantId, user:user}' -o json
```

Expect `user.type` of `servicePrincipal`. If it says `user`, stop: the isolated profile is not in effect.

## Step 2: resource inventory and state

```
cap 01-inventory az resource list -g "$RG" -o json
cap 02-resource-state az resource show --ids "$RESOURCE_ID" -o json
```

## Step 3: Resource Graph

```
cap 03-resource-graph az graph query -q \
  "Resources | where resourceGroup =~ '$RG' | project name, type, location, sku, tags | order by type asc" \
  -o json
```

This is also the tool that answers step 5 for SKUs generally, since Resource Graph projects `sku`
directly.

## Step 4: actual Cost Management evidence

Actual spend, not retail pricing. Daily granularity so the spending floor is visible per day.

```
cap 04-cost-daily az costmanagement query \
  --type ActualCost \
  --dataset-granularity Daily \
  --timeframe MonthToDate \
  --scope "/subscriptions/$SUBSCRIPTION_ID/resourceGroups/$RG" \
  -o json

cap 05-cost-by-service az costmanagement query \
  --type ActualCost \
  --dataset-granularity None \
  --timeframe MonthToDate \
  --dataset-grouping name=ServiceName type=Dimension \
  --scope "/subscriptions/$SUBSCRIPTION_ID/resourceGroups/$RG" \
  -o json
```

If the scope is narrower than Cost Management Reader was granted at, these fail with an authorization
error. That is a scoping mismatch to fix in the role assignment, not a reason to widen the role beyond
the narrowest scope that answers the query.

## Step 5: Speech SKU

```
cap 06-speech-sku az cognitiveservices account show \
  -n "$SPEECH_ACCOUNT" -g "$RG" \
  --query '{name:name, kind:kind, sku:sku, location:location}' -o json
```

Cross-check against the Resource Graph output from step 3, which should report the same `sku`. Two
independent reads agreeing is the evidence; either alone is a single point of error.

## Step 6: ARM deployment state

```
cap 07-deployments az deployment group list -g "$RG" \
  --query '[].{name:name, state:properties.provisioningState, timestamp:properties.timestamp, mode:properties.mode}' \
  -o json
cap 08-deployment-detail az deployment group show -g "$RG" -n "$DEPLOYMENT_NAME" -o json
```

## Step 7: out-of-scope denial

A read against a resource group the identity has no role on. This must fail.

```
cap 09-denial-out-of-scope az resource list -g "$OUT_OF_SCOPE_RG" -o json
```

**Expected: non-zero exit with `AuthorizationFailed`.** A success here means the identity is scoped more
broadly than intended and the role assignment must be corrected before continuing.

## Step 8: deployment-validation denial, without mutation

`what-if` and `validate` are non-mutating by design: they evaluate a template and report, changing
nothing. They nonetheless require `Microsoft.Resources/deployments/whatIf/action` and
`deployments/validate/action` respectively, which Reader does not confer. So they are the safe way to
prove the write boundary holds without ever attempting a real deployment.

```
cap 10-denial-whatif az deployment group what-if \
  -g "$RG" --template-file "$MINIMAL_TEMPLATE" --no-pretty-print
cap 11-denial-validate az deployment group validate \
  -g "$RG" --template-file "$MINIMAL_TEMPLATE"
```

**Expected: both fail with `AuthorizationFailed`.** Do not run `az deployment group create`. If either
succeeds, the identity holds more than Reader and the role assignment is wrong.

Use a minimal template that would create nothing meaningful even if it somehow ran, for example an
empty `resources` array, so a misconfiguration cannot produce a real change.

## Step 9: seal the evidence

```
cd "$EVIDENCE"
grep -rilE 'eyJ[A-Za-z0-9._-]{20,}|accessToken|client_secret' . && echo "STOP: possible secret present"
sha256sum *.txt > SHA256SUMS
chmod 400 SHA256SUMS *.txt
echo "evidence sealed: $EVIDENCE"
```

Verify later with `sha256sum -c SHA256SUMS` from inside the directory. Record the directory path and
the `SHA256SUMS` digest wherever the Phase 0 outcome is reported; do not commit the evidence itself.

## Results table

Fill in per run. Steps 7 and 8 pass by failing.

| # | Check | File | Expected | Result |
| --- | --- | --- | --- | --- |
| 1 | Resource inventory | `01-inventory` | success | |
| 2 | Resource state | `02-resource-state` | success | |
| 3 | Resource Graph | `03-resource-graph` | success | |
| 4 | Cost Management, daily actual | `04-cost-daily` | success | |
| 5 | Cost by service | `05-cost-by-service` | success | |
| 6 | Speech SKU | `06-speech-sku` | success, matches step 3 | |
| 7 | ARM deployment state | `07-deployments`, `08-deployment-detail` | success | |
| 8 | Out-of-scope read | `09-denial-out-of-scope` | **AuthorizationFailed** | |
| 9 | Deployment validation | `10-denial-whatif`, `11-denial-validate` | **AuthorizationFailed** | |

## Prohibited during evidence gathering

No Foundry throttling, no Bicep deployment, no Speech SKU change, no `az deployment group create`, no
role assignment changes, no policy assignment. Cost reduction actions remain human-owned and separately
approved, per `docs/hardening-plan.md`.

## Conditions for reconsidering an MCP surface

Reopen the MCP question only when **all four** hold. Any one missing reproduces a rejection already
recorded, so partial satisfaction is not progress.

1. **Dedicated application identity.** The server must authenticate as a service principal of our
   choosing, through client credentials or a certificate, with that principal being the identity ARM
   authorizes. A delegated permission with an interactive sign-in does not qualify, however the client
   is registered. This is what ARM MCP failed.
2. **All required reads present.** Resource inventory and state, Resource Graph, actual Cost Management
   spend, Speech SKU, and ARM deployment state, in one surface. This is what local Azure MCP failed,
   lacking four of the five.
3. **Claude Desktop support**, with the trust boundary understood. Remote connectors are brokered
   through the Claude account and originate from Anthropic's servers rather than the local machine, so
   a remote connector is a different boundary from a local stdio entry and needs its own decision.
4. **Enforceable mutation exclusion.** Mutation tools must be absent, or removable at the client, or
   denied by role. A deny policy compensating for over-broad roles does not count; the roles must be
   correct on their own.

Record the date and the surface evaluated whenever this is revisited, so a rejection is not silently
re-litigated.

## ContextPatch authority

Unchanged, and not to be changed by this document. No Azure dependency, no cloud SDK, no credential, no
network authority. `npm install`, `npx`, `az`, and generic shell remain refused with those refusals
pinned by test, and writes outside the repository root remain refused. Every command in this runbook is
run by the operator in their own shell, never through ContextPatch. See
`docs/azure-workload-position.md` and `docs/execution-threat-model.md`.
