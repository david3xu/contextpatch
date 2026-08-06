# Azure workload position

Status: decided, 6 August 2026
Decision: contextpatch takes zero Azure services. This is a standing position, not a deferral.

Companion documents: `docs/AZURE_WORKLOAD_PLAN.md` in ccl-pronunciation-trainer holds the subscription-wide credit milestone plan; `docs/operations/azure-workload-plan.md` in datacore-platform covers that repository's scope.

## Context

Work is underway to reach a Microsoft for Startups credit milestone requiring five or more distinct Azure services, each above US$1 in spend, held for 60 continuous days. The obvious temptation is to spread services across every repository to raise the count.

contextpatch is excluded from that entirely.

## Dependency audit

As of the current tree, the whole workspace declares six dependencies.

| Crate | Dependencies |
| --- | --- |
| `contextpatch_core` | `errno`, `fs2`, `libc`, `same-file`, `sha2`, `serde_json` |
| `contextpatch-server` | `core`, `serde_json`, `sha2` |

No HTTP client. No async runtime. No TLS stack. No cloud SDK. No telemetry exporter. The binary speaks its protocol over stdio and touches the local filesystem and git. Child processes that allowlisted programs start are a separate question, measured in `docs/execution-threat-model.md`.

## Why this is load bearing

contextpatch mutates the working tree of whatever repository it is pointed at, behind an allowlist, on behalf of an autonomous agent. Its trustworthiness rests on a small number of properties, and one of them is that the binary itself originates no network traffic: it carries no HTTP client, no TLS stack, and no cloud SDK.

That property is narrower than a claim that contextpatch cannot reach the network, and the distinction is load bearing in its own right. Git and GitHub actions contact remotes by design. Allowlisted build and test programs resolve dependencies and run reviewed repository code that inherits this process's environment, and with it the server user's network capability. What the standing rule protects is the absence of an outbound path that contextpatch itself owns, initiates, and would have to be trusted to use correctly. Adding one is a materially different security proposition, regardless of what the requests are nominally for.

The allowlist in `run_guarded_command` narrows which executables may be started and which arguments they may receive. It is entry-point and argument policy, not sandboxing, and it does not stop an allowlisted program from executing reviewed repository code. Adding an HTTP client would reopen from inside the same class of capability the allowlist narrows from outside, and would do so in the binary itself rather than in reviewed repository content. See `docs/safety-contract.md` for the fuller statement of the properties this repository commits to, and `docs/execution-threat-model.md` for the per-program audit.

The dependency count is not incidental minimalism. It is a reviewable surface. Six dependencies can be audited by one person in an afternoon. Adding `reqwest` alone pulls in a TLS implementation, an async runtime, and a transitive tree in the hundreds.

## Candidates considered and rejected

| Candidate | Rejected because |
| --- | --- |
| Blob Storage for mutation journals | requires an HTTP client and cloud SDK; durability is available without either, see below |
| Container Registry for release artifacts | contextpatch ships a native binary, not a container image; distribution is not currently a cloud problem |
| Application Insights for guarded command telemetry | requires an exporter and outbound connectivity; the command log is already local and inspectable via `read_command_log` |
| Key Vault for any secret material | contextpatch holds no secrets; introducing a secret store would create the exposure it purports to manage |

## Where durability needs go instead

If mutation journals, build provenance or command logs need to outlive the local filesystem, the path is datacore's event store through the existing runtime client, not a direct cloud dependency in this repository.

That arrangement puts the network boundary in a component that already has one, keeps contextpatch hermetic, and adds zero dependencies here. It also gives the journals a schema and an audit trail rather than an opaque blob.

## Standing rule

Adding any dependency that opens a network socket requires explicit justification recorded as an amendment to this document and to `docs/safety-contract.md`. Workload counting, credit milestones and cloud provider incentives do not constitute justification.

If a future requirement genuinely needs outbound connectivity, the question to answer first is whether it belongs in contextpatch at all, or in a caller that already crosses that boundary.

## Effect on the milestone

None. The subscription tally counts distinct services across the whole subscription, not per repository, so nothing is lost by excluding this one. Seven distinct services are reachable from the trainer and datacore alone.

The framing that prompted this document, spreading services across projects to raise a count, was mistaken in any case. The program condition asks that services be genuinely used as part of the solution, and services adopted to satisfy a counter are the exact case that condition excludes.
