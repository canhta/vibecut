# Agent orchestration

Private host facts and tooling pointers live in `.scratch/process/orchestration.md`. Read changed owners and actual callers; reuse unchanged context.

## Roles

The master sets cross-host priorities and keeps the canonical tickets, decisions and final evidence under the [tracker's custody rules](issue-tracker.md). Each host coordinator owns its queue, dispatch, technical decisions, review, fixes, integration and normal pushes; send compact stage updates and transfer complete handback artifacts to the master. Source work continues during transfer. The maintainer decides unresolved product choices, credentials, hardware, assets and outward-facing actions.

Use opencode for decided implementation and Claude Sonnet for unresolved technical or UI judgement and reviews. Host coordinators run Claude Opus at medium effort. Codex is not used.

On dispatch, set the ticket `claimed`. The worker ends it `resolved` after merge, or `open` with an exact remaining list. Reuse compatible worker context and warm checkouts.

## Routing

- Assign an independently usable behaviour with its base, owners, contract, failure acceptance and checks. Include necessary callers, tests and docs atomically. Use existing plans directly; separate planning only for a decision that actually blocks source work. Broad parents retain remaining acceptance ([tracker](issue-tracker.md)).
- Size follows the working boundary. File, owner and line counts describe scope; they are not approval limits. Workers update the manifest as necessary code/callers emerge and continue. Ask the host coordinator for materially new responsibilities, dependencies or changed acceptance/public/security contracts, not another path or more lines.
- Keep one writer per physical checkout. Independent checkouts may edit the same repository file. Serialize competing changes to the same behaviour or public contract; name who integrates independent edits. Shared model fields and registry entries can proceed independently with ordered integration; regenerate derived catalogs then. Only actual unmerged code dependencies belong in `Blocked by`.
- Keep useful independent source work running while another task waits for review or a compiler. Dispatch ready work by product priority and dependencies; a busy host does not block another host. A ticket or source brief needs only assignment-specific decisions and owner pointers.

## Placement and builds

- Reuse a warm checkout; independent branches start at actual `origin/main`. Use another top-level checkout for another writer. Read-only reviews can share a checkout using immutable Git revisions. Keep implementation, review and corrections on the assigned host; move only for a concrete blocker or maintainer reassignment.
- Agent count, product targets and active compilers are separate resources. Schedule workers from measured host pressure and ready work, without a fixed per-host count. Use `cargo xtask build-cap --json` and the [build owner](../../xtask/README.md#build-slots) for target admission and compiler slots. Retain at least 30% free memory and load below twice the core count. A source worker can use an isolated checkout before a build target is available; queue its checks for admitted capacity and report that wait truthfully. At target capacity a build evicts an idle target or waits by itself; workers do not prune other checkouts. Source progress does not justify bypassing build admission or claiming unrun proof.
- Compile through `cargo xtask build -- <command>` using the checkout's managed target, without `CARGO_TARGET_DIR` overrides. Reuse valid artifacts. OS-specific proof needs that OS; name other platforms unverified. Tracked code and instructions must work without private host paths.

## Coordinator loop

On mail, answer actual blockers, return actionable findings, review deliveries and dispatch compatible ready source. End the turn when no action remains so Orca can wake you; run a requested `check`. Use the session's version-matched Orca guide for dispatch, messaging, remote placement and recovery; no watch or polling loop.

- Return a verified finding immediately while remaining reviewers inspect the pinned Git revision. Keep source fixed for any running proof. Silence alone is not a reason to nudge or restart an assignment.
- Keep tickets truthful about planning, implemented, reviewed and merged stages; update the workspace comment with its current task at meaningful transitions. A sidebar update is bookkeeping, not a source/review gate.
- Preserve published refs and receipts. Extract an independently working correction from unfinished WIP onto a clean main-based integration branch when useful; retain the unfinished branch and its remaining acceptance. Append to published history.
- Reconcile live assignments with settled history before resource decisions. Follow receipt `next` and the Orca recovery/cleanup guide; timeout, silence and `unverifiable` do not prove exit. Preserve evidence and refs before verified cleanup; never mutate Orca's database.

## Worker loop

Implement the assignment through its owners using the [development loop](../engineering/workflows.md#development-loop) and [verification](../engineering/verification.md#checks). Exercise relevant failure, boundary, cancellation and metric cases. Resolve routine implementation choices locally; send actual contract/resource blockers or out-of-scope defects to the host coordinator. Follow the Orca guide when asking; start another agent only when assigned.

Finish owned commands, push normally and hand back a compact exact-SHA receipt with evidence and remaining gates, then `worker_done` and end the turn. Further assignments reuse that context. A handback does not accept an unfinished parent.

## Review and merge

1. The host coordinator is accountable for acceptance, standards, the actual diff, callers and evidence. Choose review depth for concrete risk: persistence/deletion, security/privacy, native boundaries and cross-crate public contracts warrant a separate reviewer; routine local fixes and docs can use coordinator review. Two review axes are not a default gate.
2. Reuse valid exact-source receipts. Reproduce only missing, contradictory or changed behaviour. Named test evidence selects nonzero tests; docs use their document check. Return design findings to the author, then inspect the correction and affected evidence instead of restarting broad reviews.
3. Preserve safety, data, native, privacy and product acceptance. Apply the [single-version policy](../engineering/workflows.md#upgrade-policy); never weaken a check to get green. Invalid [UI comparisons](../engineering/verification.md#ui-evidence) prove no acceptance.
4. Integrate on current `origin/main`, retaining published history. Reuse conflict-free proof; changed source/conflicts get affected checks. The host coordinator owns the once-only final public-signature workspace gate under [verification](../engineering/verification.md#checks), then normal merge/push. Whoever breaks main fixes or reverts it immediately.
5. Record the delivered leaf's `## Answer`, SHA and evidence, resolve it and add the milestone decision. Resolve a parent only after its retained acceptance is met. Outward-facing actions need maintainer approval.
