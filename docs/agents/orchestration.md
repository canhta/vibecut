# Agent orchestration

Rules for the coordinator and workers. Private host facts and tooling pointers live in `.scratch/process/orchestration.md`. Retain already-read unchanged context; reopen changed owner sections and actual callers before acting.

## Roles

| Role | Does |
|---|---|
| Coordinator | Plans with the maintainer, admits assignments, reviews and merges independent work, releases settled resources |
| opencode | Implements slices whose contracts, errors and limits are decided |
| Codex | Decides unresolved technical contracts and implements work requiring design judgement |
| Maintainer | Product decisions, credentials, hardware, assets, outward-facing actions |

## Routing

1. Admit a source leaf that delivers working behaviour: name its actual main or merged-dependency base, exact owners/symbols, types/errors/limits, failure and boundary acceptance, check commands and independent merge boundary. Keep broad parent acceptance retained while one leaf is active. A review boundary alone is not a merge boundary; unused API scaffolding does not ship a feature.
2. Keep changed behaviour within two owner modules and about 1,000 inserted lines including tests. Census every public-contract caller on current main before deciding scope. Pure forwarding, constructor fields and exhaustive-call propagation belong atomically with that contract, including callers in other crates; enumerate and review them in the same slice instead of creating preparatory handoffs. Count and disclose these edits. Design any changed caller semantics explicitly; they are behaviour, not mechanical propagation.
3. Before dispatch compare the actual files/functions/types with every current assignment's committed and uncommitted changes. Serialize overlapping writers and area moves. Shared registries may take independent additive entries inside their feature sections. Same-file work otherwise runs sequentially; only unmerged code dependencies belong in `Blocked by` ([tracker](issue-tracker.md)).
4. Route fully decided work to opencode and undecided contracts, public types spanning three or more crates, or reference-matched UI judgement to Codex. A design finding or two hours without handoff triggers coordinator reassessment with the same checkout/context, not an automatic alternate-model retry.
5. Queue compatible work on the same worker and warm checkout. For a separately consumable correction trapped in unfinished WIP, preserve all published refs and receipts, create a clean main-based integration branch and bring only the working behaviour with its real consumers and proof. Merge that boundary independently, then reconcile the retained WIP in a new integration branch. Append to published history; never squash or force-rewrite it.
6. A brief carries the changed behaviour, contract, ownership, failures, commands and pointers to unchanged policy. Cite the [upgrade policy](../engineering/workflows.md#upgrade-policy) instead of copying it. Verify named paths exist. Existing context and receipts travel with a compatible queued assignment.

## Tickets

- Each source leaf has an independent working outcome; broad parents retain all remaining acceptance without being claimed as concurrent work. [Issue tracker](issue-tracker.md) owns states and the delivery record.
- A separate defect becomes its own ticket. `.spikes/` and operational evidence stay in the coordinator's checkout.

## Placement and builds

- Reuse a compatible warm checkout; start each independent integration branch from actual `origin/main`. OS-specific work needs its OS host; report other platforms unverified. Tracked instructions and production code must work for a fresh contributor without private host paths.
- Reserve ownership and capacity before dispatch, counting admitted assignments even before their targets exist. Discover measured capacity through `cargo xtask build-cap --json` and the [build owner](../../xtask/README.md#build-slots); retain at least 30% free memory and load below twice the core count, accounting for live pressure and reported waits. Reuse valid warm targets and safely release settled resources before admitting another directory. Capacity is not evidence of compiler utilization.
- Compile through `cargo xtask build -- <command>`; keep the checkout's target, without a `CARGO_TARGET_DIR` override. Generated catalog conflicts take one side and regenerate with `cargo xtask catalog`.
- Use the session's version-matched Orca orchestration guide for dispatch, mail, replies, recovery and cleanup commands. Remote placement loads its named reference and transfers reports before cleanup; command syntax is not cached here.

## Coordinator loop

On Orca mail, process the received messages, then end the turn when no action remains; Orca wakes the coordinator. A wake naming `check` means run that command. No watch or polling loop.

1. Answer actual blockers first, reading each question whole and deciding technical details through their owners. Ask the maintainer only for unresolved product decisions, credentials, hardware, assets or outward-facing actions.
2. Review completed independent work before fresh planning. Read the changed diff and named evidence, not only the summary. Return a real finding to the worker holding the context; silence alone is not a reason to nudge or reopen a settled assignment.
3. Record implemented, reviewed and merged separately. Accepting a staged correction on unfinished WIP does not resolve the parent or prove it shipped. Preserve its refs and exact-source receipts for the integration boundary.
4. Reconcile current assignments, paused/uncertain assignments and settled history before capacity decisions. Follow Orca receipt `next` and recovery gates; `unverifiable`, timeout or silence does not prove exit. Release workers/checkouts/targets only through verified settled-resource cleanup, with reports and published refs retained. Never mutate Orca's database or treat an old dispatch ID as cleanup authority.
5. Reserve the next compatible assignment, set its current ticket `claimed`, then dispatch from ready work, critical path first. Prefer a completed behaviour reaching main over another generic prerequisite accumulating on an unfinished branch. Tracker suggestions are not reservations.
6. On every dispatch, handback, pause, block, queue, merge or release, update the ticket and its checkout's Orca workspace status and comment in the same step. The comment names only the current ticket, stage and dispatch or waiting reason; the ticket keeps the detail. A ticket change without the matching workspace change leaves the transition incomplete.

## Worker loop

1. Work only on the current admitted assignment, using the [development loop](../engineering/workflows.md#development-loop) and [verification](../engineering/verification.md#checks). Reuse unchanged context and evidence; exercise the relevant hostile inputs, failure, cancellation and metric boundaries.
2. Finish owned commands before handoff. Push the branch normally, write the compact exact-SHA receipt, send `worker_done` with truthful outcome/report path/remaining gates (at most three sentences), and end the turn. The receipt settles that task, not an unaccepted parent; further work needs a new assignment.
3. For conflicting contracts, missing acceptance, hardware, credentials, dependency/licence decisions or destructive/out-of-scope work, ask the coordinator through Orca, then end the turn as its guide requires. A precise blocker names the unavailable seam and proposed decision. The coordinator maintains ticket state; never start another agent unless explicitly assigned.
4. Report out-of-scope defects with a reproduction to the coordinator for admission; stay within the current owners.

## Review and merge

1. The coordinator reviews each independent boundary against its acceptance, actual diff, callers and failure evidence. Reuse valid exact-source receipts; reproduce only a missing, contradictory or changed behaviour ([verification](../engineering/verification.md#checks)). Named test evidence must select a nonzero number of tests; docs-only changes use their document check.
2. Use one real review/fix round: small findings can be fixed on the branch, design findings return to its worker. Review the correction and affected evidence, rather than starting another broad review. Safety, data, native, privacy and product acceptance remain gates.
3. Apply the [single-version policy](../engineering/workflows.md#upgrade-policy). Never skip, mask or loosen a test to get green; record a genuine existing failure with its blocker. Invalid comparison inputs prove no UI acceptance; [UI evidence](../engineering/verification.md#ui-evidence) retains its thresholds.
4. Integrate on current `origin/main` without rewriting published history, preserving prior refs. Reuse conflict-free receipts; changed source/conflicts get the affected checks. The coordinator performs the final public-signature workspace gate once on the rebased branch, then merges normally. Every main commit builds; whoever breaks main fixes or reverts it at once.
5. After merge write `## Answer` with SHA and evidence, set the delivered leaf `resolved`, and add its decision to the milestone map. Resolve a parent only when all its retained acceptance is met. Outward-facing actions require maintainer approval.
