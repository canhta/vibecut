# Agent orchestration

Rules for the coordinator and workers. Hosts and commands: `.scratch/process/orchestration.md`. Per-work-type tiers: `.scratch/agents.md`. Re-read the section before acting.

## Roles

| Role | Does |
|---|---|
| Coordinator (Claude) | Plans with the maintainer, writes tickets, dispatches, answers `ask`, verifies, cleans up |
| opencode | Implements up to its tier for the ticket's work type |
| Claude Sonnet, Codex | Implement above opencode's tier; review each other's and opencode's branches |
| Maintainer | Product decisions, credentials, hardware, assets, outward-facing actions |

| Tier | Work |
|---|---|
| T1 | Mechanical edits, docs, research, benchmarks, fixtures |
| T2 | Features on existing patterns; UI on settled contracts |
| T3 | New contracts, pipelines, persistence formats |
| T4 | unsafe/FFI, concurrency, job engine, render and A/V sync |

## Routing

1. opencode first when its tier covers the ticket. Above that, Sonnet or Codex, whichever has fewer live implementers. Opus only after both fail.
2. Promote opencode one tier for a work type after two consecutive clean reviewed merges; demote after a design-level `changes` or a revert. Record every verdict in `.scratch/agents.md`.
3. Route by work type. Split a ticket that mixes a contract with UI into a contract slice and a UI slice. Split by size only above about 2,000 inserted lines (opencode) or 3,000 (others).
4. The reviewer is never the implementer's agent.

## Tickets

- Each ticket ships working behaviour. `Blocked by` lists only code or contracts not yet on `main`.
- opencode acceptance names failure paths and boundary inputs.
- A separate defect becomes its own ticket.
- Tracker: [issue tracker](issue-tracker.md). `.spikes/` lives only in the main checkout.

## Placement and builds

- One worktree per ticket, branch `<type>/<milestone>-<NN>-<slug>` from `origin/main` (`<type>` is the Conventional Commits type). Implementer, reviewer and builds stay on one host.
- OS-specific work goes to that OS's host; everything else to the host with the most headroom.
- Dispatch more only while at most one build waits, free memory is at least 30%, load is under twice the core count and no worker reports waiting. Never two live tickets on the same files.
- Compile only through `cargo xtask build -- <command>`; never set `CARGO_TARGET_DIR` ([xtask](../../xtask/README.md#build-slots)).
- Remote workers get the ticket inline (`--spec-file`), write reports to their worktree's `.scratch/<milestone>/`, and are dispatched `max 1`. The coordinator pulls reports before cleanup.

## Coordinator loop

On every wake:

1. Drain worker mail; read summaries and TL;DRs only.
2. Check every live worker; nudge stuck ones; return fixes to the worker that holds the context.
3. Answer one message per `reply`, after reading it whole. Decide technical questions yourself, and product questions by looking at the reference product. Ask the maintainer only what it cannot answer, plus credentials, hardware, assets and outward-facing actions.
4. Verify finished work (below). A receipt counts only if it ran a non-zero number of named tests. Then `cargo xtask cleanup` the ticket's workers and worktrees.
5. Dispatch from `cargo xtask frontier`, critical path first, filling spare host capacity. Set `Status: claimed` first. Spec: first line `Agent loop: <assignee> max <n> start: <milestone>/NN` or `Review: <milestone>/NN <branch>@<sha7>`, then the paths and ticket-specific notes; shared rules stay in `.scratch/templates/`. `ls` every path the spec names before sending.

## Worker loop

1. Work only on the ticket your spec names.
2. Follow the [development loop](../engineering/workflows.md#development-loop) and [checks](../engineering/verification.md#checks). Before handoff, try hostile inputs, cancellation and every metric against its baseline.
3. Push the branch with its receipt, then follow review and merge. Send `worker_done` (outcome, report path, follow-ups; at most three sentences) and end the turn. Never end a turn while a build or `ask` is pending.
4. `ask` only for conflicting contracts, missing acceptance, hardware, credentials, new dependencies or licences, and destructive or out-of-scope actions. A blocker sets `Status: blocked` first. Never start another agent.
5. A small defect outside the ticket with a clear cause: fix it in a separate commit and list it. Anything larger becomes a bug ticket with a reproduction.

## Review and merge

1. A Sonnet or Codex implementer that is confident (every Must and Verify item holds, handoff checks pass, no open question) merges itself. T1 work merges itself. Otherwise one review of the frozen SHA by another agent, in the same worktree, written to `.scratch/<milestone>/reviews/NN-<sha7>.md`.
2. One review round, never a second. The reviewer fixes the findings it is confident about and merges; it returns `changes` only for design changes, large diffs or missing context. The implementer then fixes and merges; the coordinator checks each fix and dispatches no re-review.
3. Dual versions, legacy shims, deprecated paths and compatibility code are `changes`.
4. Never skip, mask or loosen a test to get green. A known failure cites its bug ticket.
5. Merge: rebase on `origin/main`, run [checks](../engineering/verification.md#checks), push fast-forward to `main`. Local checks are the only gate. Whoever breaks `main` fixes or reverts it at once. Every commit on `main` builds.
6. After merge: write `## Answer` with SHA and evidence, set `Status: resolved`, add one line to the milestone map.
7. Outward-facing actions need the maintainer's explicit approval.
