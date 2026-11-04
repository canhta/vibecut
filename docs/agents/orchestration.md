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
   Longer briefs go in a spec file (`--spec-file`). Reviewers get a fresh worktree.
2. End the turn; Orca wakes you for mail. Read the `worker_done` summary and the report TL;DR only.
3. Answer questions with one batched reply; send no acknowledgements.
4. After a merge, clean up: `orca orchestration worker-release --dispatch <id>`, then `orca worktree rm --worktree path:<worktree> --delete-branch`. Keep a worktree only while its ticket awaits review or fixes.
5. Dispatch whatever the merge unblocked, opencode first.

Load `orca skills get orchestration` for CLI details; use handles from receipts.

## Worker loop

1. Claim: re-read the ticket in the main checkout, skip it unless `open`, set `Status: claimed`.
2. Implement per the [development loop](../engineering/workflows.md#development-loop).
3. Finish the ticket on one of the two [merge paths](#review-and-merge).
4. Claim the next frontier ticket with your `Assignee`, up to the spec's maximum.
5. Send one `worker_done` for the whole loop: at most three sentences (outcomes, report path, follow-ups) with `--report-path`. Then stop.

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

After either path the merger writes `## Answer` (SHA, evidence path), sets
`Status: resolved` and adds one decision line to the milestone `map.md`.
`main` only moves fast-forward.
