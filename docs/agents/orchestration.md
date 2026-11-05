# Orchestration with Orca

## Roles and routing

| Role | Responsibility |
|---|---|
| Claude coordinator | Plans with the maintainer, writes tickets, dispatches, answers questions, cleans up; implementation, review and merge belong to workers |
| opencode | Default implementer up to its current tier |
| Codex | Implements above opencode's tier; reviews every code branch |
| Maintainer | Product decisions, credentials, hardware, assets |

| Tier | Work |
|---|---|
| T1 | Mechanical edits, docs, research, benchmarks, fixtures |
| T2 | Features/adapters on existing patterns; UI on settled contracts |
| T3 | New contracts, pipelines, persistence formats |
| T4 | unsafe/FFI, concurrency, job engine, render and A/V sync core; Codex only |

Promote one tier after two consecutive reviewed code tickets at its tier merge without design-level `changes`; demote one tier after that verdict or a reverted merge. The coordinator updates `.scratch/agents.md` on every verdict with tier, track record and per-work-type evidence/routing. Work types with design-level or robustness findings route to Codex until the profile changes.

## Tickets and placement

Write tickets with owners already on `main` or explicit `Blocked by` dependencies so they can finish without ownership questions. Each ships working behaviour; research, audits and benchmarks serve a pending decision. Apply [structural admission](../engineering/standards.md#structural-admission): one cohesive responsibility; split mixed owners and critical-path contracts into early contract slices plus follow-ups, but keep inseparable test/review pieces together. Metrics follow [verification](../engineering/verification.md), each with a baseline. Opencode acceptance names failure paths and boundary inputs, including failed publication, hostile sizes and every entry to cover.

Use one Orca-created worktree per ticket; its reviewer runs in a second terminal there while the implementer waits, reusing warm artifacts without another checkout. Start each pull-loop ticket from `origin/main` on `<type>/<milestone>-<NN>-<slug>`, lowercase with hyphens; `<type>` matches the main change's Conventional Commits type (`feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci`, `chore`). [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) and [Conventional Branch](https://conventionalbranch.org/) define the conventions. Tracker access: [issue tracker](issue-tracker.md); `.spikes/` also lives only in the main checkout and uses absolute paths.

- On shared Unix maintainer machines, every command that compiles Rust (`cargo xtask ci`, `cargo test`, `cargo clippy`, `cargo build`) runs through `cargo xtask build -- <command...>`: it waits in FIFO order for a build slot (up to three, on macOS, a second or third admitted only at 30% free memory after a 20-second ramp), sets `CARGO_TARGET_DIR` to that slot's warm cache and runs the command. Never set `CARGO_TARGET_DIR` yourself. The xtask launcher builds separately under `target/xtask`; waiter locks provide liveness and FIFO admission, and the command inherits its slot lease until it exits. This wrapper is Unix-only; Windows and dedicated hosted CI runners run Cargo directly.
- Size the live worker count by what you observe, not a fixed number: dispatch another worker while builds barely queue (`ls ~/.cache/vibecut/locks/build-queue | wc -l` at most 1), free memory stays high (`sysctl -n kern.memorystatus_level` at least 30) and no worker reports waiting on a build, a merge conflict or another ticket; when any of these slips, let workers finish before dispatching. Never run two live tickets that own the same crate or file area (check `Ownership`; serialize the overlap).
- A connected Windows Orca server (`worker-start --on <environment>`) runs tickets for any agent, self-contained so work continues while the coordinator's machine is off: the tickets go inline in `--spec-file`, cargo runs directly, and reports land in that worktree's `.scratch/` for the coordinator to copy back. Connection details are maintainer-private.

`.scratch/bin/cleanup` prunes incremental sessions and slots untouched for 2+ days, retaining active slots.

A resolved spike retains reports, outputs, labels and eval corpus; remove venvs, package/model caches and weights unless an open ticket names them. Record weight revision and hash for re-fetching.

## Coordinator loop

1. Dispatch `Agent loop: <assignee> max <n> start: <milestone>/NN` or `Review: <milestone>/NN <branch>@<sha7>`; put longer briefs in `--spec-file`. Start the reviewer with `--worktree` set to the implementer's worktree.
2. End the turn for Orca mail. On wake, drain `check` to `No messages` (or read every `[unhandled]` inbox line); read only each worker's summary and report TL;DR. Also run `orca orchestration worker-list` and `git worktree list`. Save untracked outputs and stop/clean workers whose ticket merged or was dropped, or idle workers with neither mail nor an open ticket; retain workers awaiting review. Confirm terminal release after `worker-stop`, using `orca terminal close --terminal <id>` if needed.
3. Answer questions in one batched reply; send only substantive replies. The coordinator owns technical decisions; dispatch research when evidence is missing, and bring the maintainer only decisions in its role.
4. Return fixes and re-reviews to the dispatch holding context; create a replacement only when it is gone. `send` may reopen a finished worker under a new dispatch ID: use the current `worker-list` ID.
5. After merge/drop, run `.scratch/bin/cleanup <dispatch-id>... <worktree-name>...` to release all ticket dispatches, terminals and worktrees. Retain implementer and reviewer until then.
6. Dispatch what `cargo xtask frontier` lists, critical path first. Implementers are opencode unless the ticket is T3+ or its work type routes to Codex in `.scratch/agents.md`; live Codex implementers never outnumber opencode ones.
7. Only the coordinator pauses/resumes by explicit message; a pause also kills apps, `osascript` and builds the worker started.

## Worker loop

1. Re-read the main-checkout ticket; claim only `open` tickets by setting `Status: claimed`.
2. Implement the [development loop](../engineering/workflows.md#development-loop) and its [checks](../engineering/verification.md#checks). Before handoff, attack boundary/hostile inputs, mid-operation cancellation, gesture reversal and every ticket metric against its baseline; every language, format and backend must retain its behaviour.
3. Push the frozen SHA to the ticket branch with its check receipt, then follow a merge path below. Ticket-branch and main pushes are authorized by this loop. When review is needed, send `worker_done` and end the turn immediately; the coordinator sends `continue` to resume.
4. Claim the next assigned frontier ticket up to the dispatch maximum. Finish with `worker_done` (at most three sentences: outcome, report path, follow-ups; include `--report-path`), then stop.

Ask the coordinator through `ask` for conflicting contracts, missing acceptance, hardware/credentials, new dependency/asset licensing, or destructive/out-of-scope actions; continue independent work. A blocker sets `Status: blocked` with its reason before escalation. Worker communication uses only `ask`, blocker escalation and `worker_done`; load `orca skills get orchestration` for CLI syntax and use receipt handles.

Outside-ticket bugs: fix small/medium defects with a clear cause, one owner and no design change now in a separate commit with a regression test, listing them in the report. File large defects (data loss, crash, security, cross-owner or design changes) immediately as prioritized bug tickets with reproduction; ask immediately if they block this ticket or corrupt user data. Close failures unreproduced on both a local and hosted attempt as unreproduced with run IDs; reopen on recurrence. Defects in the ticket under review remain review findings.

## Review and merge

T1 self-merges after every Verify item holds. T2–T4 use a separate Codex review of the frozen SHA, written to `.scratch/<milestone>/reviews/NN-<sha7>.md` in the main checkout with [review fields](../engineering/verification.md#reports). Read the implementer's receipt; run targeted reproductions only, following [checks](../engineering/verification.md#checks) for reviewer changes.

Fix small, obvious findings with full context on the ticket branch. Larger/design findings get `changes` and return to the implementer for a new frozen SHA; re-review covers previous findings and the fix diff. Dual versions, legacy shims, deprecated paths and compatibility code get `changes`. Ask about unsettled product/policy questions rather than issuing `changes`; they do not affect implementer tiers. A failure called known must cite a bug ticket; file one first if absent.

Approved work merges immediately: rebase onto `origin/main`, resolve conflicts (regenerate `Cargo.lock` on conflict), apply [checks](../engineering/verification.md#checks), and push fast-forward to `main`; repeat on rejected push. The last merger immediately fixes or reverts red main. Write `## Answer` with SHA/evidence, set `Status: resolved` and add one decision line to the milestone map.

Outward-facing actions (upstream posts, visibility changes, publishing) require the maintainer's explicit approval; push authorization is defined in the worker loop.
