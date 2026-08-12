# B3 merge plan

This branch is too large to merge as one diff and cannot be merged as independent pieces. It can be
merged as sequential prefixes, which gets the review benefit without the conflict risk. The
difference matters, and the reason is measured below rather than assumed.

## The five groups

Boundaries are given as commits rather than counts. `git rev-list --count <range>` is the way to get a
count, available since C38; a number written here would be wrong by the next commit, which is exactly
how it went wrong twice before C38 existed.

| Group | Range | What it is |
| --- | --- | --- |
| G1 | `main..5abcc96` | Fixed shell script list, typed Compose runner, artifact build gate, workspace formatting |
| G2 | `5abcc96..e7c0b8a` | Tool registry: snapshots, five migration waves, module splits |
| G3 | `e7c0b8a..6a2c755` | C37 rg option surface, and the audit corrections that followed it |
| G4 | `6a2c755..b5d3489` | Manifest derivations, schema promise clause, constants and name collisions |
| G5 | `b5d3489..HEAD` | Rename commits end to end, the confirming-test rule, C38 |

## Independence: tested, and false

The proposal was that the groups touch disjoint files, which would make them four or five reviewable
merges in any order. Measured with `git diff --name-only` per range, they do not.

G2 and G3 overlap on seven paths, including the three most heavily edited files in the branch:

```
crates/server/src/tools/registry.rs
crates/server/src/tools/dispatch.rs
crates/server/src/tools/capability.rs
crates/core/src/process/guarded_command.rs
crates/core/src/process/guidance.rs
docs/tool-registry-plan.md
CLAUDE.md
```

G1 is broader still: 42 paths, reaching `guarded_command.rs`, `dispatch.rs`, `capability.rs`,
`state.rs`, `commit.rs`, and both contract documents. Every later group edits files G1 introduced or
changed.

So the groups are thematically separable and textually entangled. That is not a reason to abandon the
split; it is a reason to change what the split is for.

## What to do instead

Merge sequential prefixes, in order, as separate merges:

```
main  <-  5abcc96      (G1)
main  <-  e7c0b8a      (G2)
main  <-  6a2c755      (G3)
main  <-  b5d3489      (G4)
main  <-  HEAD         (G5)
```

This works because the branch is linear. Each merge is a fast-forward or a trivial merge of a strict
ancestor, so no group needs cherry-picking and no conflict can arise from the overlap above. What the
overlap costs is the freedom to reorder or to review a group in isolation, not the ability to land the
work in stages.

The order is forced rather than chosen, and that is worth knowing during review. G3 edits lines G2
introduced: the registry table, the attribution helper, the manifest sections. Reviewing G3 without G2
in hand reads as unexplained churn. The sequence is a prerequisite for the review making sense, not a
convenience.

## What each group is reviewable as

- **G1** is the largest single review and the least self-explanatory, because it predates the
  conventions the rest of the branch established. Review it as capability additions with their
  guards, not as a refactor.
- **G2** is verifiable mechanically: both snapshot fixtures are byte-identical across the whole group,
  which is the evidence that a 54-tool migration changed no advertised behaviour. Check that claim
  first; if it holds, the diff needs far less scrutiny than its size suggests.
- **G3** contains the one security fix on the branch, C37. `rg --pre` was demonstrated executing a
  shell over repository files and writing outside the repository root. Everything else in G3 is
  documentation correction and should be read as such.
- **G4** is almost entirely derivations and assertions replacing hand-maintained lists. Nothing in it
  changes what the server does. If it is a bottleneck, it is the safest group to land last.
- **G5** is the only group containing a capability that did not previously work: committing a rename.
  It carries the one test on that path that confirms rather than plans.

## Carried risk

The count grows with every commit, and each one raises the review cost of the merge that has to happen
anyway. Three commits are also unpushed behind a credential failure, which is a separate problem: it
gates publication of G5's tail, not the merge of anything.
