# Agent orchestration

## Roles and routing

| Role | Responsibility |
|---|---|
| Claude coordinator | Plans with the maintainer, writes tickets, dispatches, answers questions, cleans up; implementation, review and merge belong to workers |
| opencode | Default implementer up to its tier for the ticket's work type |
| Codex | Implements above opencode's tier where no Claude Sonnet route applies; reviews code branches it did not implement |
| Claude Sonnet worker | Implements or reviews complex work: new contracts, file publication, T4 (concurrency, job engine, render and A/V sync); its own implementations self-merge after their checks when it is confident, and request a separate review when it is not (it states which and why in the report) |
| Maintainer | Product decisions, credentials, hardware, assets |

| Tier | Work |
|---|---|
| T1 | Mechanical edits, docs, research, benchmarks, fixtures |
| T2 | Features/adapters on existing patterns; UI on settled contracts |
| T3 | New contracts, pipelines, persistence formats |
| T4 | unsafe/FFI, concurrency, job engine, render and A/V sync core |

opencode holds one tier per work type, not an overall tier. Promote a work type one tier after two consecutive reviewed tickets of that type merge without design-level `changes`; demote it one tier after that verdict or a reverted merge. The coordinator updates `.scratch/agents.md` on every verdict with the per-work-type tier, evidence and routing. Work types with design-level or robustness findings route to Codex until their profile changes. A work type with two consecutive clean T3 merges may trial T4 one ticket at a time under a close Codex review whose acceptance includes race, mid-operation cancellation and hostile-input tests; unsafe/FFI stays Codex until a concurrency or job-engine trial merges clean.

## Tickets and placement

Write tickets with owners already on `main` or explicit `Blocked by` dependencies so they can finish without ownership questions. `Blocked by` lists only hard dependencies (code or contracts the ticket needs that are not on `main`); ordering preferences, shared areas and transitively implied blockers stay out of it. Each ships working behaviour; research, audits and benchmarks serve a pending decision. Apply [structural admission](../engineering/standards.md#structural-admission): one cohesive responsibility; split mixed owners and critical-path contracts into early contract slices plus follow-ups, but keep inseparable test/review pieces together. Metrics follow [verification](../engineering/verification.md), each with a baseline. Opencode acceptance names failure paths and boundary inputs, including failed publication, hostile sizes and every entry to cover. Route by work type, not size: when a ticket mixes a contract, lifecycle or algorithm part (publication, caches, job cancel/failure/bounds, media timing, new commands or formats) with UI or wiring on it, split a Codex contract slice and an opencode UI slice blocked by it. Split by size only above about 2,000 inserted lines for opencode or 3,000 for Codex, into two (at most three) slices of at least about 800 lines that each ship working behaviour.

Use one worktree per ticket on branch `<type>/<milestone>-<NN>-<slug>`; its reviewer runs in a second terminal there while the implementer waits, reusing warm artifacts without another checkout. Start each pull-loop ticket from `origin/main` on `<type>/<milestone>-<NN>-<slug>`, lowercase with hyphens; `<type>` matches the main change's Conventional Commits type (`feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci`, `chore`). [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) and [Conventional Branch](https://conventionalbranch.org/) define the conventions. Tracker access: [issue tracker](issue-tracker.md); `.spikes/` also lives only in the main checkout and uses absolute paths.

- On shared development hosts, every command that compiles Rust (`cargo xtask ci`, `cargo test`, `cargo clippy`, `cargo build`) runs through `cargo xtask build -- <command...>`: it waits in FIFO order for a build slot, whose count follows host capacity (logical CPUs, total memory, the 30% free-memory threshold after a 20-second ramp, and a documented load bound); `VIBECUT_BUILD_SLOTS` overrides for tests and emergencies. It sets `CARGO_TARGET_DIR` to that slot's warm cache and runs the command. It also applies the provisioned LGPL FFmpeg compile- and run-time environment, so wrapped commands need no per-command SDK paths. Never set `CARGO_TARGET_DIR` yourself. The xtask launcher builds separately under `target/xtask`; waiter locks provide liveness and FIFO admission, and the slot lease is held until the command exits. Windows launcher-death limitations live in [xtask](../../xtask/README.md#build-slots). Admitted commands default to `max(2, logical CPUs / slot count)` Cargo jobs unless `CARGO_BUILD_JOBS` is set; user-level job caps are unnecessary. Dedicated hosted CI runners run Cargo directly.
- Size the live worker count by what you observe, not a fixed number: dispatch another worker while builds barely queue (`ls ~/.cache/vibecut/locks/build-queue | wc -l` at most 1), free memory stays high (`sysctl -n kern.memorystatus_level` at least 30; on Windows `Win32_OperatingSystem.FreePhysicalMemory` at least 30%), load stays under twice the core count (`sysctl -n vm.loadavg`) and no worker reports waiting on a build, a merge conflict or another ticket; when any of these slips, let workers finish before dispatching. Never run two live tickets that own the same crate or file area (check `Ownership`; serialize the overlap).
- A ticket whose only blocker is an approved or in-review branch starts from that branch and rebases when it lands; its feature is reviewed at once and the later integration (rebase, catalog, shared registries) is re-reviewed as a separate diff.
`cargo xtask cleanup` prunes incremental sessions and slots untouched for 2+ days, retaining active slots.

A resolved spike retains reports, outputs, labels and eval corpus; remove venvs, package/model caches and weights unless an open ticket names them. Record weight revision and hash for re-fetching.

## Coordinator loop

1. Dispatch `Agent loop: <assignee> max <n> start: <milestone>/NN` or `Review: <milestone>/NN <branch>@<sha7>`; the reviewer works in the implementer's worktree.
2. On each wake, drain worker mail, read only each worker's summary and report TL;DR, check every live worker (including ones that ended a turn without reporting) and clean up settled work.
3. Answer questions in one batched reply; send only substantive replies. Never message a settled worker just to inform it: attach the note to its next task. When a ticket's acceptance is blocked by a separate defect, file that defect and split the acceptance instead of growing the ticket. The coordinator owns technical decisions; dispatch research when evidence is missing, and bring the maintainer only decisions in its role.
4. Return fixes and re-reviews to the worker holding the context; create a replacement only when it is gone.
5. After merge/drop, run `cargo xtask cleanup` for the ticket's workers and worktrees. Retain implementer and reviewer until then.
6. Dispatch what `cargo xtask frontier` lists, critical path first. Implementers are opencode when the ticket's work type has a tier covering the ticket in `.scratch/agents.md`, otherwise the work type's route there (Codex or Claude Sonnet); place each ticket on a host with spare capacity.
7. Only the coordinator pauses/resumes by explicit message; a pause also kills apps and builds the worker started.

Tooling, hosts and their quirks are maintainer-private: `.scratch/process/orchestration.md`.



## Worker loop

1. Re-read the main-checkout ticket; claim only `open` tickets by setting `Status: claimed`.
2. Implement the [development loop](../engineering/workflows.md#development-loop) and its [checks](../engineering/verification.md#checks). Before handoff, attack boundary/hostile inputs, mid-operation cancellation, gesture reversal and every ticket metric against its baseline; every language, format and backend must retain its behaviour.
3. Push the frozen SHA to the ticket branch with its check receipt, then follow a merge path below. Ticket-branch and main pushes are authorized by this loop. When review is needed, send `worker_done` and end the turn immediately; the coordinator sends `continue` to resume. Never end a turn while a build, CI run or `ask` answer is pending: give long commands a timeout of at least 60 minutes and wait.
4. Claim the next assigned frontier ticket up to the dispatch maximum, in a new worktree on the same host: the finished ticket's worktree stays untouched for its reviewer. Finish with `worker_done` (at most three sentences: outcome, report path, follow-ups; include `--report-path`), then stop.

Ask the coordinator through `ask` for conflicting contracts, missing acceptance, hardware/credentials, new dependency/asset licensing, or destructive/out-of-scope actions; continue independent work. A blocker sets `Status: blocked` with its reason before escalation. Worker communication uses only `ask`, blocker escalation and `worker_done`.

Outside-ticket bugs: first check open bug tickets and recent `main` for the same fix (parallel tickets hit shared fixtures); fix small/medium defects with a clear cause, one owner and no design change now in a separate commit with a regression test, listing them in the report. File large defects (data loss, crash, security, cross-owner or design changes) immediately as prioritized bug tickets with reproduction; ask immediately if they block this ticket or corrupt user data. Close failures unreproduced on two local attempts as unreproduced with logs; reopen on recurrence. Defects in the ticket under review remain review findings.

## Review and merge

T1 self-merges after every Verify item holds. Claude Sonnet implementations self-merge like T1 once every Verify item and check holds, unless Sonnet judges its confidence insufficient and requests a review. Other T2–T4 work uses a separate review of the frozen SHA by a different agent than the implementer, in the implementer's worktree on the same host, written to `.scratch/<milestone>/reviews/NN-<sha7>.md` in the main checkout with [review fields](../engineering/verification.md#reports). Review specs are one line plus ticket-specific notes; the shared rules live in `.scratch/templates/review.md`. Read the implementer's receipt; run targeted reproductions only, following [checks](../engineering/verification.md#checks) for reviewer changes.

The reviewer fixes every finding it is confident it can fix within the review context on the ticket branch, with a regression test, then applies checks and merges; this avoids a return loop. Only findings needing a design change, a large diff or context the reviewer lacks get `changes` and return to the implementer for a new frozen SHA; re-review covers previous findings and the fix diff. Dual versions, legacy shims, deprecated paths and compatibility code get `changes`. Ask about unsettled product/policy questions rather than issuing `changes`; they do not affect implementer tiers. A failure called known must cite a bug ticket; file one first if absent. Never skip, mask, loosen or special-case a test to get a green receipt; a known failure is cited, not hidden. A failure matching an open bug ticket never blocks the merge: cite it and merge without asking; stop only for a failure no ticket explains. A Claude Sonnet reviewer goes further: it fixes any finding it is confident about, design-level ones included, then checks and merges; it returns `changes` only when it is not confident of the fix.

Approved work merges immediately: local checks are the only gate; workers and reviewers never trigger or wait for hosted CI (it is informational). rebase onto `origin/main`, resolve conflicts (regenerate `Cargo.lock` on conflict), apply [checks](../engineering/verification.md#checks), and push fast-forward to `main`; repeat on rejected push. The last merger immediately fixes or reverts red main. Write `## Answer` with SHA/evidence, set `Status: resolved` and add one decision line to the milestone map.

Outward-facing actions (upstream posts, visibility changes, publishing) require the maintainer's explicit approval; push authorization is defined in the worker loop.
