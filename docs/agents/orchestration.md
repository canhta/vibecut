# Orchestration with Orca

## Roles

| Role | Does |
|---|---|
| Claude coordinator | Plans with the maintainer, writes tickets, dispatches, answers questions, cleans up. Never implements, reviews or merges. |
| opencode | Default implementer (cheapest): every ticket up to its current tier. |
| Codex | Tickets above opencode's tier; reviews every code branch. |
| Maintainer | Product decisions, credentials, hardware, assets. |

## Tiers

| Tier | Work |
|---|---|
| T1 | Mechanical edits, docs, research, benchmarks, fixtures |
| T2 | Features and adapters following an existing pattern; UI on settled contracts |
| T3 | New contracts, pipelines, persistence formats |
| T4 | unsafe/FFI, concurrency and job engine, render and A/V sync core (Codex only) |

Route each ticket to opencode when its tier allows. opencode rises one tier after
two consecutive reviewed code tickets at its tier merge without a design-level
`changes` verdict, and drops one tier after such a verdict or a reverted merge. The
coordinator records tier and track record in `.scratch/agents.md` on every verdict.

## Tickets

Write each ticket so it can finish without a mid-ticket ownership question: every
piece it reuses either exists on `main` or is listed in `Blocked by`; every quality
claim names its baseline and metric. Split a critical-path ticket that blocks many
others into a contract-first slice that merges early and follow-ups.

One ticket = one cohesive responsibility (a crate, contract or user-facing feature)
under [structural admission](../engineering/standards.md#structural-admission).
Split a ticket that mixes responsibilities; merge pieces that cannot be tested or
reviewed apart. Format and queries: [issue tracker](issue-tracker.md).

Ask the coordinator for conflicting contracts, missing acceptance, absent hardware or
credentials, licence questions about new dependencies or assets, and destructive or
out-of-scope actions; keep working on independent parts meanwhile. Copied reference
code is settled by [copy imports](../engineering/workflows.md#copy-imports).

## Placement

- One Orca-created worktree and branch per worker; a pull loop starts each ticket on a fresh branch from `origin/main` in that worktree.
- `.scratch/` and `.spikes/` exist only in the main checkout: read and write them by absolute path.
- Build cache per slot: `CARGO_TARGET_DIR=$HOME/.cache/vibecut/target/<slot>`, slots `codex-1`, `codex-2`, `opencode-1`, `opencode-2`, `review-1`, `review-2`; one live worker per slot.
- At most three concurrent Rust builds; research and docs workers take no slot.

## Coordinator loop

1. Dispatch with a one-line spec; the rest lives in this file:
   - `Agent loop: <assignee> <slot> max <n> start: <milestone>/NN`
   - `Review: <milestone>/NN <branch>@<sha7> <slot>`
   Longer briefs go in a spec file (`--spec-file`). Reviewers get their own new worktree, never the implementer's.
2. End the turn; Orca wakes you for mail. Read the `worker_done` summary and the report TL;DR only.
3. Answer questions with one batched reply; send no acknowledgements.
4. Return work to the dispatch that already holds its context: review `changes` go to the implementer's dispatch, re-reviews to the same reviewer's dispatch (`orca orchestration send` reopens a finished worker). Start a new worker only when that dispatch is gone.
5. After a merge, clean up with `.scratch/bin/cleanup <dispatch-id> <worktree-name>...`. Keep the implementer and a `changes` reviewer (dispatch and worktree) until the ticket merges.
6. Dispatch what `.scratch/bin/frontier` lists, opencode first, critical path first.
7. Anything outward-facing (posting upstream, changing repo visibility, publishing) needs the maintainer's explicit yes first.

Load `orca skills get orchestration` for CLI details; use handles from receipts.

## Worker loop

1. Claim: re-read the ticket in the main checkout, skip it unless `open`, set `Status: claimed`.
2. Implement per the [development loop](../engineering/workflows.md#development-loop).
   Before handing to review, attack your own branch the way a reviewer will: hostile
   and boundary inputs, cancellation mid-operation, gesture reversal, and every
   quality metric against its recorded baseline (no language, format or backend may
   regress). Fix what you find first.
3. Finish the ticket on one of the two [merge paths](#review-and-merge). A ticket
   that needs review ends the turn right away with `worker_done`, so its review starts
   while you wait; the coordinator sends `continue` to resume the loop.
4. Claim the next frontier ticket with your `Assignee`: highest priority first, then
   lowest number (`.scratch/bin/frontier <assignee>`), up to the spec's maximum.
5. `worker_done` is at most three sentences (outcomes, report path, follow-ups) with `--report-path`. Then stop.

Talk to the coordinator only through `ask` (blocking question) and `worker_done`.
A blocked ticket gets `Status: blocked` plus the reason, then an escalation.

## Review and merge

**Self-merge** T1 tickets once every Verify item holds: rebase on `origin/main`, run
`cargo xtask ci`, push fast-forward to `main`. T2–T4 tickets go to review.

**Review** the frozen SHA in a separate Codex dispatch. Write the review to
`/Users/canh/Startup/VibeCut/.scratch/<milestone>/reviews/NN-<sha7>.md` using the
[review fields](../engineering/verification.md#reports).

- Small, clear finding (obvious fix, full context, no design change): fix it on the ticket branch.
- Larger or design-level finding: verdict `changes`; the coordinator returns it to the implementer, who fixes and resubmits a new frozen SHA.
- Dual versions, legacy shims, deprecated paths and backward-compatibility code are `changes` findings.
- Approved: merge immediately like a self-merge (regenerate `Cargo.lock` on conflict).
- A re-review checks the earlier findings and the fix diff, not the whole ticket again.

After either path the merger writes `## Answer` (SHA, evidence path), sets
`Status: resolved` and adds one decision line to the milestone `map.md`.
`main` only moves fast-forward.
