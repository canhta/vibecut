# Agent orchestration

Rules for the coordinator and workers. Hosts and commands: `.scratch/process/orchestration.md`. Re-read the section before acting.

## Roles

| Role | Does |
|---|---|
| Coordinator (Claude) | Plans with the maintainer, splits tickets into slices, dispatches, reviews and merges every branch, cleans up |
| opencode | Implements every slice |
| Codex | Implements slices that need design judgement, and takes over a slice opencode could not finish |
| Maintainer | Product decisions, credentials, hardware, assets, outward-facing actions |

## Routing

1. The coordinator splits each ticket into slices opencode can finish: one behaviour, its owner module and files, acceptance naming failure paths and boundary inputs, and the exact check commands. A slice that would exceed about 1,000 inserted lines (tests included) or two owner modules splits again by behaviour. Every spec repeats the [upgrade policy](../engineering/workflows.md#upgrade-policy) line. A slice that needs a new contract states the contract (types, errors, limits) in the spec.
2. opencode implements mechanical slices: moves, call-site updates and slices whose `## Design` decides every type, error and limit. Codex implements a slice that still needs judgement: an undecided contract, a change to a public type used by three or more crates, or UI behaviour matched against the reference product. A slice goes to Codex in the same worktree once its review returned a design finding or its worker has run two hours without handing off.
3. Before dispatch the coordinator greps current `main` for every file and symbol in the ticket's Ownership line, fixes the line, and splits the ticket when it would exceed the bound above. It then compares those files with what every live branch changes, committed or not: a ticket that edits a function a live branch edits waits for that merge, and a refactor that moves or renames an area's files runs while no other branch edits that area. Shared registries are exempt: entries go inside the feature's own section, never at the end of the list.
4. Slices of one ticket that touch the same files run in sequence; `Blocked by` orders them.

## Tickets

- Each ticket ships working behaviour. `Blocked by` lists only code or contracts not yet on `main`.
- A separate defect becomes its own ticket.
- Tracker: [issue tracker](issue-tracker.md). `.spikes/` lives only in the main checkout.

## Placement and builds

- One worktree per ticket, branch `<type>/<milestone>-<NN>-<slug>` from `origin/main` (`<type>` is the Conventional Commits type). Implementer, reviewer and builds stay on one host.
- OS-specific work goes to that OS's host; everything else to the host with the most headroom.
- Dispatch more only while at most one build waits, free memory is at least 30%, load is under twice the core count and no worker reports waiting. Never two live tickets changing the same function, type or doc section. Two live tickets may share a file where each only adds to it (a command declaration line, a test table row or test function, an i18n key, a module list entry, a README table row); the later merge rebases. Generated files (`crates/engine/catalog.digest`, `docs/product/agent-control/commands.md`) are never merged by hand: on conflict, take either side and rerun `cargo xtask catalog`.
- Compile only through `cargo xtask build -- <command>`; never set `CARGO_TARGET_DIR` ([xtask](../../xtask/README.md#build-slots)).
- Remote workers get the ticket inline (`--spec-file`), write reports to their worktree's `.scratch/<milestone>/`, and are dispatched `max 1`. The coordinator pulls reports before cleanup.

## Coordinator loop

On every wake:

1. Drain worker mail; read summaries and TL;DRs only.
2. Check every live worker; nudge stuck ones; return fixes to the worker that holds the context.
3. Answer one message per `reply`, after reading it whole. Decide technical questions yourself, and product questions by looking at the reference product. Ask the maintainer only what it cannot answer, plus credentials, hardware, assets and outward-facing actions.
4. Verify finished work (below). A receipt counts only if it ran a non-zero number of named tests. Then clean up the ticket's workers and worktrees with the tracker tooling.
5. Dispatch from the tracker's ready list, critical path first, filling spare host capacity. Set `Status: claimed` first. Spec: first line `Agent loop: <assignee> max <n> start: <milestone>/NN` then the paths and ticket-specific notes; shared rules stay in `.scratch/templates/`. `ls` every path the spec names before sending.

## Worker loop

1. Work only on the ticket your spec names.
2. Follow the [development loop](../engineering/workflows.md#development-loop) and [checks](../engineering/verification.md#checks). Before handoff, try hostile inputs, cancellation and every metric against its baseline.
3. Push the branch with its receipt; never merge to `main` yourself. Send `worker_done` (outcome, report path, follow-ups; at most three sentences) and end the turn. Never end a turn while a build or `ask` is pending.
4. `ask` only for conflicting contracts, missing acceptance, hardware, credentials, new dependencies or licences, and destructive or out-of-scope actions. A blocker sets `Status: blocked` first. Never start another agent.
5. A small defect outside the ticket with a clear cause: fix it in a separate commit and list it. Anything larger becomes a bug ticket with a reproduction.

## Review and merge

1. The coordinator reviews every branch against its slice spec: reads the diff, runs the checks, tries the failure paths.
2. One round. The coordinator fixes small findings on the branch itself and merges. A design-level finding goes back to the same worker as feedback; the worker fixes and the coordinator merges after checking the fix.
3. Dual versions, legacy shims, deprecated paths, migrations and compatibility code are always findings, however small; the coordinator deletes them before merging.
4. Never skip, mask or loosen a test to get green. A known failure cites its bug ticket.
5. Merge: rebase on `origin/main`, run [checks](../engineering/verification.md#checks), push fast-forward to `main`. Local checks are the only gate. Whoever breaks `main` fixes or reverts it at once. Every commit on `main` builds.
6. After merge: write `## Answer` with SHA and evidence, set `Status: resolved`, add one line to the milestone map.
7. Outward-facing actions need the maintainer's explicit approval.
