# Agent delivery with Orca

## Roles and placement

| Role | Responsibility |
|---|---|
| opencode implementer | Clear/mechanical work and resource-heavy inventories, builds, benchmarks and fixtures |
| Codex implementer/reviewer | Hard design, FFI, concurrency, jobs, render/sync, model/provider contracts; review every opencode branch |
| Claude coordinator | Plan, ticket, dispatch, review Codex branches, resolve gates, merge and record decisions |
| Human | Product decisions, credentials, Windows hardware and license decisions |

When unsure, route ambiguous correctness work to Codex; explicit recipes to opencode.
One ticket owns one feature or crate, about 600 changed lines maximum excluding a
mechanical import. Split larger work before starting unless explicitly authorized.
Stop and ask for conflicting contracts, missing acceptance, unavailable required
hardware/credentials, license ambiguity or destructive/out-of-scope action. Continue
independent authorized work while a nonblocking answer is pending.

Use one worktree per ticket, created by Orca. Shared placement needs explicit
coordinator assignment of disjoint files; never switch another worker's branch.
Branch: `t/<milestone>-NN-<slug>`. For a fresh worktree starting on its base branch:
`git checkout -b t/p0-core-01-time`. If Orca already created the ticket branch, use it.
Set `CARGO_TARGET_DIR=target/agent-<worker-name>` for every Rust command. At most
three concurrent Rust builds on a 16 GB machine: two implementers plus one reviewer.

## Coordinator loop

Load `orca skills get orchestration` and command `--help` before using new flags;
use the session's configured CLI executable instead of `orca` when applicable.
Use runtime-issued handles/IDs from receipts, not guessed values. An open, unblocked
[ticket](issue-tracker.md#ticket-header) may be dispatched with a complete spec:

```sh
orca orchestration worker-start --agent codex --worktree new-top-level --repo "path:$PWD" --name p0-core-01-time --base-branch main --spec 'Follow .scratch/p0-core/issues/01-time-crate.md and docs/agents/orchestration.md; use branch t/p0-core-01-time; commit -s; report evidence; do not push main'
orca orchestration worker-list --json
```

For existing placement use `--worktree path:<absolute-worktree-path>`; for explicitly
assigned shared files use `--worktree current` without creation flags. Dispatch
independent ready tickets, then end the turn. Orca wakes the coordinator for mail:

```sh
orca orchestration check
orca orchestration reply --id <message-id> --body '<answer>'
orca orchestration send --to dispatch:<dispatch-id> --subject 'Review changes' --body '<concrete findings and required checks>'
```

Follow receipt `next` actions. A failed start must not be retried blindly: load
`orca skills get orchestration --reference references/recovery-and-cleanup.md`.
A worker list is resource accounting, not evidence that a task succeeded. Never
steer workers through terminal input or poll mail in a background script.

## Worker loop

1. Set ticket `Status: claimed` before implementation; confirm branch and file scope.
2. Follow the [development loop](../engineering/workflows.md#development-loop),
   [copy protocol](../engineering/workflows.md#copy-imports) where applicable, and
   [verification](../engineering/verification.md#checks). Commit with `git commit -s`.
3. Ask only through the assigned coordinator channel, using the preamble's handle:

```sh
orca orchestration ask --from <worker-handle> --question '<blocking question and evidence>'
```

After asking, end the turn; Orca wakes the worker with the answer. If blocked,
set ticket `Status: blocked`, record the reason locally and send an escalation:

```sh
orca orchestration send --from <worker-handle> --type escalation --subject 'Blocked: required input' --body '<reason and required action>' --task-id <task-id> --dispatch-id <dispatch-id>
```

Finish exactly once, using the active preamble IDs and [done report](../engineering/verification.md#reports):

```sh
orca orchestration send --from <worker-handle> --type worker_done --subject 'Ticket ready for review' --body '<three sentences: work; findings/evidence; remaining work>' --task-id <task-id> --dispatch-id <dispatch-id> --outcome succeeded --report-path <absolute-local-report-path>
```

The body/report contains branch, SHAs, diff stat, exact commands/results, CI tail,
each Verify item, observed/unverified limits and proposed follow-ups. Use
`--outcome failed` for incomplete work; omit optional file/report flags unless real.
Once confirmed, end the turn and stop edits. Do not poll or start other work.
Send no status or acknowledgement messages: only `ask` for blocking questions and
one `worker_done`. The coordinator batches instructions into few `send` messages.
`worker_done` hands off for review; it does not resolve the markdown ticket.

## Review and merge

Two paths; the implementer states which one it took in `worker_done`.

**Self-merge.** For a simple ticket (mechanical, docs, tests, small isolated
change) where the implementer is confident it meets every Verify item, the
implementer merges itself: rebase on `origin/main`, rerun `cargo xtask ci` with
local name patterns, `git push origin HEAD:main` (fast-forward only; on rejection
fetch, rebase, rerun checks, retry). Hard tickets (FFI, concurrency, pipelines,
contracts, persistence formats, AI providers) never self-merge.

**Reviewed.** Otherwise review the frozen SHA independently: Codex reviews opencode
work, the coordinator reviews Codex work. The reviewer writes
`.scratch/<milestone>/reviews/NN-<sha7>.md` using the
[review template](../engineering/verification.md#reports) and fixes findings
directly on the ticket branch (signed-off commits) instead of bouncing them back;
only findings needing a product decision go back to the coordinator. When approved,
the reviewer merges exactly as in self-merge.

After either path: record the merged SHA/evidence under `## Answer`, set
`Status: resolved`, add one decision pointer in `.scratch/<milestone>/map.md`, and
preserve other workers' worktrees. `wontfix` needs a reason; `blocked` returns to
`claimed` only after its blocker is removed. Never force-push `main`. Keep reports
local; publishing requires explicit direction.
