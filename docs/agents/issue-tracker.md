# Issue tracker

Tickets live in git-ignored `.scratch/` in the coordinator's main checkout: plain files, the single source of truth. `.spikes/` and `.samples/` sit beside it, also git-ignored. Workers read and edit only their assigned ticket files by absolute path and re-read a file before editing it. The coordinator's private tooling lists ready work, ranks unresolved work by what it holds up and serves a read-only board.

| Path under `.scratch/` | Owns |
|---|---|
| `<milestone>/spec.md` | Scope |
| `<milestone>/map.md` | One line per decision |
| `<milestone>/issues/NN-slug.md` | Ticket |
| `<milestone>/reviews/` | Review reports |
| `<milestone>/reports/` | Worker handbacks |

Ticket headers use these exact fields; omit `Blocked by` when empty:

```text
Status: open | claimed | blocked | resolved | wontfix
Type: task | research | bug
Priority: P0 | P1 | P2 | P3 | P4
Assignee: <model id> | human | -
Blocked by: - | NN, <milestone>/NN
```

`Assignee` is the worker model id, for example `claude-haiku-5-5` or `claude-sonnet-5-5`; resolved tickets keep their historical values. `Blocked by` holds ticket ids only; put any condition or note in the body.

`claimed` means a live worker holds the ticket. A queued but ready ticket is `open`; one that needs intervention is `blocked` with the condition in the body. Keep a broad parent `open` while a leaf is active and resolve it only after all its acceptance. Record unrun checks as unverified.

Record the delivery stage in the body: **merged unbuilt** (main SHA and what the combined build must check), then **resolved** once the combined build is green.

Priority follows the [delivery order](../product/requirements.md#scope): P0 foundation, P1 core editor, P2 AI editing and agent control, P3 Home, P4 long video to shorts; a defect takes the priority of the feature it breaks. Bare numbers mean the same milestone. A ticket is unblocked when every blocker is `resolved`; `wontfix` records its reason. Put the outcome under `## Answer`.

Ready order: highest priority, then most open tickets transitively blocked, then lowest number; a blocking view ranks unresolved tickets by unresolved transitive dependents. Claiming, resolving and routing follow [orchestration](orchestration.md).
