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

Route each ticket to opencode when its tier allows and its work type has no failing
record. opencode rises one tier after two consecutive reviewed code tickets at its
tier merge without a design-level `changes` verdict, and drops one tier after such a
verdict or a reverted merge. The coordinator records tier, track record and a
work-type profile (evidence and route per kind of work) in `.scratch/agents.md` on
every verdict.

An opencode ticket's acceptance names its failure paths and boundary inputs
(failed publish, hostile sizes, every entry to cover); work types where reviews
found design-level or robustness defects go to Codex until the profile changes.

## Tickets

Write each ticket so it can finish without a mid-ticket ownership question: every
piece it reuses either exists on `main` or is listed in `Blocked by`; metrics appear only where
[verification](../engineering/verification.md) asks for them (performance, our own AI
logic), each with its baseline. Every ticket ships working behaviour; research,
benchmarks and audits exist only when a decision waits on them. Split a critical-path ticket that blocks many
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
- Every command that compiles Rust (`cargo xtask ci`, `cargo test`, `cargo clippy`, `cargo build`) runs through `/Users/canh/Startup/VibeCut/.scratch/bin/build <command>`: it waits for one of three build slots (a lock), sets `CARGO_TARGET_DIR` to that slot's warm cache and runs the command. Never set `CARGO_TARGET_DIR` yourself.
- Workers are not limited by build slots: dispatch as many as the frontier allows, as long as no two live tickets own the same crate or file area (check `Ownership` in each ticket; serialize the overlap).
- Spike hygiene: a spike keeps its report, outputs, labels and eval corpus. Its venvs, package/model caches and model weights are deleted when its ticket resolves, unless an open ticket names the path; weights are re-fetched by recorded revision and hash.
- Build caches are kept only while used: `.scratch/bin/cleanup` prunes incremental sessions and slots untouched for 2+ days; slots in use stay warm.

## Coordinator loop

1. Dispatch with a one-line spec; the rest lives in this file:
   - `Agent loop: <assignee> max <n> start: <milestone>/NN`
   - `Review: <milestone>/NN <branch>@<sha7>`
   Longer briefs go in a spec file (`--spec-file`). Reviewers get their own new worktree, never the implementer's.
2. End the turn; Orca wakes you for mail. `check` returns one message per call: repeat it until `No messages` (or read every `[unhandled]` line of `orca orchestration inbox`). Read the `worker_done` summary and the report TL;DR only. On every wake also run `orca orchestration worker-list` and `git worktree list`: a worker whose ticket merged or was dropped, or one idle without mail and without an open ticket, is stopped and its worktree cleaned (save untracked outputs first). `worker-stop` can leave the agent running: confirm with `worker-list` that its terminal is released, else `orca terminal close --terminal <id>`. A worker waiting on its own ticket's review is not stale.
3. Answer questions with one batched reply; send no acknowledgements. Technical
   decisions (runtime, library, contract, approach) are the coordinator's: when the
   evidence is missing, dispatch research and decide from it. Bring the maintainer
   only what the Maintainer row owns.
4. Return work to the dispatch that already holds its context: re-reviews to the same reviewer's dispatch (`orca orchestration send` reopens a finished worker). Review `changes` go to the implementer's dispatch; start a new worker on the branch only when that dispatch is gone.
5. After a merge, clean up with `.scratch/bin/cleanup <dispatch-id> <worktree-name>...`. Keep the implementer and reviewer (dispatch and worktree) until the ticket merges or is dropped; only then are they removed.
6. Dispatch what `.scratch/bin/frontier` lists, opencode first, critical path first.
7. Anything outward-facing (posting upstream, changing repo visibility, publishing) needs the maintainer's explicit yes first.

Load `orca skills get orchestration` for CLI details; use handles from receipts.

## Worker loop

1. Claim: re-read the ticket in the main checkout, skip it unless `open`, set `Status: claimed`.
2. Implement per the [development loop](../engineering/workflows.md#development-loop).
   Before handing to review, attack your own branch the way a reviewer will: hostile
   and boundary inputs, cancellation mid-operation, gesture reversal, and every
   metric the ticket names against its recorded baseline (no language, format or backend may
   regress). Fix what you find first.
3. Finish the ticket on one of the two [merge paths](#review-and-merge). A ticket
   that needs review ends the turn right away with `worker_done`, so its review starts
   while you wait; the coordinator sends `continue` to resume the loop.
4. Claim the next frontier ticket with your `Assignee`: highest priority first, then
   lowest number (`.scratch/bin/frontier <assignee>`), up to the spec's maximum.
5. `worker_done` is at most three sentences (outcomes, report path, follow-ups) with `--report-path`. Then stop.

**Bugs found outside the ticket at hand** (any agent; a defect in the ticket under review stays a review finding):
- Small or medium and contained (clear cause, fix stays inside one owner, no design change): fix it now in its own commit with a regression test, and list it in your report.
- Large (data loss, crash, security, cross-owner or design-level): create a ticket right away (`Type: bug`, priority by severity, reproduction included) and name it in `worker_done`; `ask` at once if it blocks your ticket or corrupts user data.

Talk to the coordinator only through `ask` (blocking question) and `worker_done`.
A blocked ticket gets `Status: blocked` plus the reason, then an escalation.

## Review and merge

**Self-merge** T1 tickets once every Verify item holds: rebase on `origin/main`, run
the [smallest covering check](../engineering/verification.md#checks), push
fast-forward to `main`. T2–T4 tickets go to review.

**Review** the frozen SHA in a separate Codex dispatch. Write the review to
`/Users/canh/Startup/VibeCut/.scratch/<milestone>/reviews/NN-<sha7>.md` using the
[review fields](../engineering/verification.md#reports).

- Small, clear finding (obvious fix, full context, no design change): fix it on the ticket branch.
- Larger or design-level finding: verdict `changes`; the coordinator returns it to the implementer, who fixes and resubmits a new frozen SHA.
- Dual versions, legacy shims, deprecated paths and backward-compatibility code are `changes` findings.
- Approved: merge immediately like a self-merge (regenerate `Cargo.lock` on conflict).
- A re-review checks the earlier findings and the fix diff, not the whole ticket again.
- A finding that turns on product scope or policy the docs do not settle is an `ask` to the coordinator, not a `changes` verdict; it never counts against the implementer's tier.
- A failure called "known" cites its ticket; with none, the reviewer files a `Type: bug` ticket first.

After either path the merger writes `## Answer` (SHA, evidence path), sets
`Status: resolved` and adds one decision line to the milestone `map.md`.
`main` only moves fast-forward.
