# Execution threat model

Audit basis: `HEAD` `3def154`. Source of truth is `crates/core/src/process/guarded_command.rs`
(`validate_command`, `is_allowlisted_program`) and `crates/core/src/process/runner.rs`
(`run_bounded_command`), corroborated by live probes against the installed server.

This document exists because the public claims describe argv shape correctly and then
overstate it into containment. Allowlisting an executable name narrows policy. It does not
sandbox the process that name starts.

## Authority of each allowlisted program

`validate_command` gates the first argument only. Everything after it is passed through, and
the program itself decides what to do with it.

| Program | Permitted first argument | Executes repository-authored code | May reach network |
| --- | --- | --- | --- |
| `git` | `status`, `diff`, `log`, `show`, `rev-parse`, `ls-tree` | No | No for these subcommands |
| `cargo` | `check`, `test`, `build`, `clippy` | Yes: `build.rs`, proc macros, test binaries | Yes: registry and dependency fetch |
| `bun` | `run`, `test` | Yes: `package.json` scripts, test files | Yes |
| `npm` | `run`, `test` | Yes: `package.json` scripts, via a shell npm starts | Yes |
| `pnpm` | `run`, `test` | Yes: `package.json` scripts | Yes |
| `python`, `python3` | any repo-relative path ending `.py` | Yes: the whole script | Yes |
| `pytest` | node ids and options, minus plugin-loading `-p` | Yes: `conftest.py`, collected tests | Yes |
| `rg` | any argument | No | No |
| `bash` | exactly `references/check-base-image.sh`, optionally `task` | Yes: whatever that tracked script contains | Depends on script |
| `harbor` | `run` | Yes: agent workload | Yes |

## Authority axes

### Filesystem

Argv path confinement is real and enforced. A path argument outside the repository root is
refused before spawn, confirmed by probe: `rg --files --max-depth 1 /usr/share/zoneinfo` is
refused with `command argument may not reference paths outside the repository root`. The child
working directory is opened relative to the root descriptor and held open until spawn.

Confinement applies to argv. It does not apply to code the child then executes. A repository
Python script, a `build.rs`, a `conftest.py`, or a `package.json` script may open, read, and
write any path the server user can reach, including paths outside the repository.

### Environment

`run_bounded_command` sets `GIT_PAGER=cat` and `NO_COLOR=1`, nulls stdin, and pipes stdout and
stderr. It calls neither `env_clear` nor `env_remove`. The child therefore inherits the server
process environment in full, including `PATH`, `HOME`, and any exported tokens.

### Subprocess

ContextPatch interposes no shell: it spawns the named program with an explicit argument array.
That is not the same as the absence of a shell in the resulting process tree. `npm run` and
`pnpm run` execute their script entries through a shell that the package manager itself starts,
so repository-authored shell text does run, sourced from `package.json` rather than from a
caller-supplied string.

Process group termination is best effort. `docs/safety-contract.md` already states correctly
that a descendant escaping into a new session defeats cleanup and that the runner is not a
process sandbox. That admission should govern the general claims rather than sit only in the
timeout section.

### Credential

Because the environment is inherited and the filesystem is reachable by executed code, any
ambient credential the server user holds is reachable: Git credential helpers and keychain
entries, `~/.ssh`, `~/.azure`, cloud CLI token caches, and exported API tokens. Output redaction
reduces accidental disclosure through the transcript. It does not reduce the authority of the
child.

### Network

No network namespace, proxy, or egress restriction is applied to guarded commands. Dependency
resolution alone reaches the network on ordinary use.

Separately from `run_guarded_command`, the typed Git and GitHub actions `git_push_exact`,
`git_remote_check`, `github_pr_run`, and `github_fork_prepare` contact remotes by design.

## Claims corrected

| Location | Correction |
| --- | --- |
| `docs/tool-spec.md` annotations section | `openWorldHint` is no longer a blanket `false`. It is derived per action from `crates/server/src/tools/schema/authority.rs`, and the section states the derivation rule, the covered categories, and the isolated exceptions. |
| `docs/tool-spec.md` tool table | The `run_guarded_command` row now says argument confinement and no interposed shell, and states that it is not a sandbox. |
| `README.md` | The opening claim now says the server interposes no shell and confines argument paths, rather than implying containment. A threat-boundary paragraph states no Azure client, no cloud SDK, environment inheritance, npm's own shell, and the task-image exception. |
| `docs/safety-contract.md` item 8 | Rewritten with an explicit non-sandbox statement covering environment inheritance and executed repository code. |
| `docs/architecture.md` | The no-shell property is now scoped to what this server does, and separated from any containment claim about the process tree. |
| `docs/azure-workload-position.md` | The justification claimed contextpatch "cannot reach the network". Corrected to the narrower and true claim that the binary originates no network traffic, while Git actions and dependency-capable builds do reach the network. The standing decision of zero Azure authority is unchanged. |
| `run_guarded_command` schema description | Now states the argument confinement, the absence of an interposed shell, and then plainly that it is not a sandbox. |

## Classification of advertised reach

`openWorldHint` is derived from one `RemoteReach` classification rather than repeated as bare
booleans. An action is open-world when it contacts a remote system itself, or starts
repository-controlled code inheriting this server's environment and network capability.

Open-world: `git_remote_check`, `git_push_exact`, `git_branch_prepare`, `git_merge_readiness`,
`github_fork_prepare`, `github_pr_run`, `run_guarded_command`, `artifact_python_run`,
`validation_profile_run`, `harbor_run_start`, `setup_profile_run`, `native_build_run`,
`native_device_run`, `fixture_generator_run`, `base_image_check_run`, and `project_execute`,
which dispatches every inner action.

Closed-world by isolation: `task_image_python_run` and `image_cleanliness_check_run`, both of
which run with networking disabled.

`git_merge_readiness` is both read-only and open-world, because it may fetch from a remote in
order to report state. That combination is deliberate and pinned by test.

## Policy holes tightened

These narrow policy. They do not create a sandbox, and nothing here should be read as one.

`pytest` was gated by `=> true`, accepting any argument. It now refuses the plugin-loading option
`-p` in both separated and combined short forms, so a caller cannot name a plugin to import. Long
options such as `--pyargs` are unaffected. Paths outside the repository were already refused by
argument path confinement, and that refusal is now pinned by test.

pytest children additionally receive `PYTEST_DISABLE_PLUGIN_AUTOLOAD=1`, and
`PYTEST_ADDOPTS` and `PYTEST_PLUGINS` are removed from their environment so an ambient value
cannot inject options or plugins into a run the caller did not request. The environment is not
otherwise cleared, because supported builds depend on inherited toolchain variables.

Repository-local `conftest.py` collection is deliberately retained. It is reviewed repository
content, several supported suites depend on it, and disabling it would break those suites without
changing the underlying authority. A test pins that retention.

## The distinction the documents currently blur

This codebase contains a genuinely sandboxed execution path and an allowlisted one, and
sandbox vocabulary has leaked from the first to the second.

`task_image_python_run` is sandboxed: read-only repository mount, disabled networking, all
capabilities dropped, `no-new-privileges`, PID cap, read-only container root, bounded `/tmp`
tmpfs. `image_cleanliness_check_run` runs `docker run --rm --network none` with a fixed
entrypoint. `artifact_python_run` refuses caller-supplied executable paths, shell snippets, and
per-request environment overrides.

`run_guarded_command` has none of those properties. It is a narrowed policy over a trusted
repository, and its safety rests on the repository contents being reviewed, not on containment.

## Prohibitions that remain accurate and must be preserved

Probe-confirmed and source-confirmed refusals, which the corrections must not weaken:

- package installation: `npm install -g @azure/mcp` is refused as not allowlisted, with
  `permitted for this program: run, test`
- `npx`, `az`, `docker`, `pip`, and `gh` are absent from `is_allowlisted_program` entirely
- arbitrary `python -m`: the first argument must end in `.py` and must not start with `-`
- Python outside the repository: a scratch-token script path is refused and redirected to
  `artifact_python_run`
- caller-supplied shell strings: no program accepts one; `bash` is pinned to one tracked script
- argv paths outside the repository root
- direct `harbor run`, redirected to `harbor_run_start`

## Consequence for Azure work

ContextPatch cannot install, authenticate, or drive an Azure client, and must not be extended
to do so. Azure access belongs to a separate Claude Desktop entry running Microsoft's Azure MCP
Server, outside this repository and outside this authority boundary. See
`docs/azure-workload-position.md`.
