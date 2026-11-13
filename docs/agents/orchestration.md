# Agent orchestration

Private host facts and tooling live in `.scratch/process/orchestration.md`.

## Roles

One coordinator session (Claude Opus) on the main checkout owns the queue, dispatch, review, merge, the combined build and the tickets. Workers are Claude Code subagents, one per ticket:

| Work | Model |
|---|---|
| Decided implementation, UI from recorded evidence, tests, docs, mechanical refactors | Haiku |
| Cross-layer design, public contracts, concurrency, persistence, security, reviews, combined build | Sonnet |
| A ticket Sonnet returned with an unresolved design gap | Opus |

The coordinator decides technical, design and product questions that the reference evidence answers, records them in the ticket and moves on. The maintainer decides credentials, hardware, assets, outward-facing actions and anything that destroys their work.

## Dispatch

- Read the whole ticket first. Set `Status: claimed` and `Assignee`, then start the subagent in its own worktree branched from `origin/main`, in the background.
- The brief names the ticket path, scope, owners to read, files another live worker owns, and the handback path. The ticket carries the detail; the brief does not restate it.
- Keep ready work running by priority ([tracker](issue-tracker.md)). Serialize competing changes to the same behaviour or public contract; independent edits to one file proceed and the coordinator integrates them. Only unmerged code dependencies belong in `Blocked by`.
- Size follows the working boundary, not line counts. Workers ask only for a new responsibility, dependency or changed contract, never for another path or more lines.

## Worker loop

Implement through the owners with the [development loop](../engineering/workflows.md#development-loop). Workers do not build or test ([checks](../engineering/verification.md#checks)): code must be compile-clean by reading, with every caller of a changed signature updated in the same commit. Fold new cases as rows into the existing behaviour tests. Commit with a Conventional subject and `git commit -s`, push the branch, never `main`.

Hand back a report under `.scratch/<milestone>/reports/` that starts with `## TL;DR`: branch, head SHA, what is done, what remains, what the combined build must check. Append a dated line to the ticket and end.

## Review and merge

1. The coordinator reads the diff, its callers and tests, and rejects invented values, weakened or retried tests, broad allows, dead code, and any dual version: an old path kept beside the new one, a re-export or alias kept for old callers, a fallback for old ids, files or preferences, or migration code. Small problems it fixes itself; a design finding goes back to the worker once. There is no second review round.
2. Rebase on `origin/main`, resolve conflicts keeping both sides' intent, and fast-forward `main`. Append `Merged unbuilt` with the SHA and the build's checks to the ticket.
3. After a batch of merges, one combined-build worker runs the full gate on `main` and fixes failures at their root. Tickets resolve when that build is green; open items stay in the ticket or become new tickets.
4. A found bug always gets a worker, even while new work is paused.
5. After merge, remove the worktree and the local and remote branch.
