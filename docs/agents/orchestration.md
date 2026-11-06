# Agent orchestration

Rules for the coordinator and workers. Each rule lives here once; hosts, commands and quirks are maintainer-private (`.scratch/process/orchestration.md`), per-work-type tiers and verdicts are in `.scratch/agents.md`. Before any action that dispatches, merges, deletes or pushes, re-read the section that governs it; do not act from memory.

## Roles

| Role | Does |
|---|---|
| Coordinator (Claude) | Plans with the maintainer, writes tickets, dispatches, answers `ask`, verifies fixes, cleans up. Never implements or reviews |
| opencode | Implements up to its tier for the ticket's work type |
| Codex | Overflow for work above opencode's tier when Sonnet workers are saturated; reviews Sonnet's branches |
| Claude Sonnet | Default for work above opencode's tier (new contracts, file publication, T4, UI fidelity); also reviews opencode and Codex branches |
| Maintainer | Product decisions, credentials, hardware, assets, outward-facing actions |

| Tier | Work |
|---|---|
| T1 | Mechanical edits, docs, research, benchmarks, fixtures |
| T2 | Features/adapters on existing patterns; UI on settled contracts |
| T3 | New contracts, pipelines, persistence formats |
| T4 | unsafe/FFI, concurrency, job engine, render and A/V sync core |

## Routing

1. Cost order: opencode first where its work type's tier covers the ticket (`.scratch/agents.md`); above that Claude Sonnet is the default implementer, and Codex takes the overflow when Sonnet workers are saturated or the host's account is busy. The reviewer is never the implementer's agent (Sonnet's branches are reviewed by Codex, opencode's and Codex's by Sonnet). Live Codex implementers never outnumber opencode ones.
2. opencode holds a tier per work type. Promote one tier after two consecutive reviewed tickets of that type merge without design-level `changes`; demote one tier after that verdict or a reverted merge; update `.scratch/agents.md` on every verdict. Work types with design-level or robustness findings route to Codex until their profile changes. A type with two clean T3 merges may trial T4, one ticket at a time, with a close Codex review (race, cancel, hostile-input tests); unsafe/FFI stays Codex.
3. Keep one or two low-risk opencode trials live on untested work types: never self-merged, close independent review, more than five contained reviewer fixes earns no credit.
4. Route by work type, not size. A ticket mixing a contract/lifecycle/algorithm part with UI or wiring is split into a Codex contract slice and an opencode UI slice blocked by it. Split by size only above about 2,000 inserted lines (opencode) or 3,000 (Codex), into 2–3 slices of at least about 800 lines that each ship working behaviour.

## Tickets

- Write tickets whose owners are on `main` or listed in `Blocked by`. `Blocked by` holds hard dependencies only (code or contracts not on `main`), never ordering preferences or transitive blockers.
- Each ticket ships working behaviour; research, audits and benchmarks serve a pending decision. Apply [structural admission](../engineering/standards.md#structural-admission). Metrics follow [verification](../engineering/verification.md), each with a baseline.
- opencode acceptance names failure paths and boundary inputs: failed publication, hostile sizes, every entry to cover.
- A separate defect that blocks acceptance becomes its own ticket; split the acceptance, do not grow the ticket.
- Tracker access: [issue tracker](issue-tracker.md). `.spikes/` lives only in the main checkout; use absolute paths.
- A resolved spike keeps reports, outputs, labels and eval corpus; delete venvs, package/model caches and weights unless an open ticket names them; record weight revision and hash.

## Placement and builds

- One worktree per ticket on `<type>/<milestone>-<NN>-<slug>` from `origin/main` (lowercase, hyphens; `<type>` is the Conventional Commits type of the main change: `feat fix perf refactor test docs build ci chore`; [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/), [Conventional Branch](https://conventionalbranch.org/)). The reviewer runs in the same worktree.
- A ticket's implementer, reviewer and builds stay on one host. OS-specific work (OS `cfg`, native/FFI, file-system semantics, packaging) goes to that OS's host; everything else goes to the host with the most headroom.
- Dispatch another worker only while builds barely queue (at most 1 waiting), free memory stays at or above 30%, load is under twice the core count, and no worker reports waiting on a build, conflict or ticket. Otherwise let workers finish. Never run two live tickets on the same crate or file area.
- Every command that compiles Rust on a shared host runs through `cargo xtask build -- <command...>`: FIFO build slots sized to the host, warm per-slot `CARGO_TARGET_DIR`, provisioned FFmpeg environment. Never set `CARGO_TARGET_DIR` yourself. Contract and limits: [xtask](../../xtask/README.md#build-slots). Hosted CI runners run Cargo directly.
- A worker never runs an unbounded load generator; stress is bounded (`timeout`, explicit pids, no bare `wait`) and announced before load above 8.
- Remote hosts: the worker gets the ticket inline in `--spec-file` and has no tracker; the coordinator sets `Status: claimed` at dispatch, the worker writes its report to its worktree's `.scratch/<milestone>/` and ends with `worker_done`, and the coordinator pulls it and records `Status: resolved`, `## Answer` and the map line. Dispatch `max 1` there: a `worker_done` ends the dispatch, so a finished remote worker takes no follow-up (start a new one in the same worktree with the context inline). The coordinator's machine holds the tracker and the Run that receives every worker's mail; it must stay awake. Provision the pinned toolchain and FFmpeg once before sending several workers to a new host.
- A ticket whose only blocker is an approved or in-review branch starts from that branch and rebases when it lands; its feature is reviewed at once, and the later integration diff (rebase, catalog, shared registries) is checked by the coordinator, not reviewed again.
- `cargo xtask cleanup` prunes incremental sessions and slots untouched for 2+ days.

## Coordinator loop

On every wake, in this order:

1. Run the sweep; drain worker mail to empty; read each worker's summary and report TL;DR only.
2. Check every live worker, including ones that ended a turn without mail; nudge stuck ones; create a replacement only when the worker is gone, and return fixes to the worker holding the context. About a minute after a start, read the terminal and resend the spec if the agent sits at an empty prompt.
3. Answer questions in one batched reply. Never message a settled worker just to inform it: attach the note to its next task. The coordinator decides technical questions; ask the maintainer only product, credential, hardware, asset or outward-facing questions.
4. Verify and record finished work (review and merge below); then run `cargo xtask cleanup` for the ticket's workers and worktrees. Save untracked outputs, stop and clean workers whose ticket merged or was dropped and idle workers with neither mail nor an open ticket; retain implementer and reviewer until merge or drop. On a remote host, pull the ticket's reports and reviews into the tracker before cleanup: they exist only in that worktree and cleanup deletes it.
5. Dispatch what `cargo xtask frontier` lists, critical path first, within the routing and placement rules; when a host has spare capacity, fill it. Spec first line: `Agent loop: <assignee> max <n> start: <milestone>/NN` or `Review: <milestone>/NN <branch>@<sha7>`; the reviewer starts in the implementer's worktree.
6. Pause or resume workers only by explicit message from the coordinator; a pause also kills apps and builds the worker started.

## Worker loop

1. Re-read the ticket; claim only `open` tickets with `Status: claimed`.
2. Follow the [development loop](../engineering/workflows.md#development-loop) and [checks](../engineering/verification.md#checks). Before handoff attack boundary and hostile inputs, mid-operation cancellation, gesture reversal and every metric against its baseline; every language, format and backend keeps its behaviour.
3. Push the frozen SHA to the ticket branch with its check receipt, then follow review and merge. Ticket-branch and main pushes are authorized by this loop. Send `worker_done` (at most three sentences: outcome, report path, follow-ups; include `--report-path`) and end the turn; when a review is needed the coordinator sends `continue` to resume. Never end a turn while a build, CI run or `ask` answer is pending; give long commands a timeout of at least 60 minutes and wait.
4. Claim the next assigned ticket up to the dispatch maximum, in a new worktree on the same host; the finished worktree stays untouched for its reviewer.
5. Use `ask` only for conflicting contracts, missing acceptance, hardware or credentials, new dependency or asset licensing, and destructive or out-of-scope actions; continue independent work. A blocker sets `Status: blocked` with its reason first. Communicate only through `ask`, blocker escalation and `worker_done`. Never start another agent.
6. Bugs outside the ticket: check open bug tickets and recent `main` first. Fix small defects with a clear cause, one owner and no design change in a separate commit with a regression test and list them in the report. File large ones (data loss, crash, security, cross-owner, design) as prioritized bug tickets with a reproduction; ask at once if one blocks the ticket or corrupts user data. A failure that does not reproduce in two local attempts is recorded unreproduced with logs. Defects in the ticket under review are review findings.

## Review and merge

1. T1 and confident Sonnet or Codex implementations merge themselves once every Verify item and check holds. Everything else gets one review of the frozen SHA by a different agent, in the implementer's worktree on the same host, written to `.scratch/<milestone>/reviews/NN-<sha7>.md` with [review fields](../engineering/verification.md#reports). Review specs are one line plus ticket notes; shared rules are in `.scratch/templates/review.md`. Read the receipt; run targeted reproductions only.
2. **One review round per ticket, never a second.** The reviewer fixes every finding it is confident about on the ticket branch, with a regression test, then runs checks and merges. A Sonnet reviewer also fixes design-level findings it is confident about and returns `changes` only when it is not. Findings that need a design change, a large diff or context the reviewer lacks get `changes`: the implementer fixes them, runs checks and merges; the coordinator verifies each finding against the fix diff and its test and dispatches no re-review.
3. `changes` also for dual versions, legacy shims, deprecated paths and compatibility code. Unsettled product or policy questions go to the coordinator as questions, not `changes`, and do not affect tiers.
4. Never skip, mask, loosen or special-case a test to get green. A failure called known cites a bug ticket (file one first if absent); a failure matching an open bug ticket never blocks the merge. Stop only for a failure no ticket explains.
5. Merge immediately when approved: rebase onto `origin/main`, regenerate `Cargo.lock` on conflict, apply [checks](../engineering/verification.md#checks), push fast-forward to `main`, repeat on rejection. Local checks are the only gate; nobody waits for hosted CI. The last merger fixes or reverts red `main` at once.
6. Every commit on `main` builds: resolve a rebase conflict in the commit that causes it or squash first; run `git grep -n '^<<<<<<< '` over the range before pushing. `aca291f` and `5bb3179` broke this; bisect across them needs `git bisect skip`.
7. After merge: write `## Answer` with SHA and evidence, set `Status: resolved`, add one decision line to the milestone map.
8. Outward-facing actions (upstream posts, visibility changes, publishing) need the maintainer's explicit approval.
