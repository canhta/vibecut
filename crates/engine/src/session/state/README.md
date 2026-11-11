# Session state

Interface: `Session`, `JobTarget` (re-exported by `session`). `checkpoint` and `session_control` are private.

## Rollback

Every command entry (`apply`, `apply_merged`, `apply_with_ripple`, `execute`, `execute_with_ripple`, control) runs inside one boundary. A refusal, an unwind, or a revision that cannot advance after a successful body all restore the same `Checkpoint`:

| Restored | Kept as the command left it |
|---|---|
| project, undo and redo, selection, merge key, trim sources, ID counter, pending change | `generation`: lifetimes opened by the refused command stay retired |
| gesture entry and the timeline-preview flag | `protected` paths: set only by `protect_paths` |
| ripple policy | `identity`, history `limit`: fixed at construction |
| metadata (view state, scene views) | `grouping`: owned by `transaction` |
| revision, ledger sequence and events | ledger receipts and jobs: written by `control` after the boundary |

`Checkpoint` destructures `Session` exhaustively, so a new field does not compile until it is placed in one column.

A gesture cancel, a replaced timeline preview and a failed `transaction` return to their entry state but keep the ID counter: their children already returned IDs to the caller, so those IDs are never reused. A boundary restores the counter because a refused command returned nothing.

## Session control

Commands declared `SessionControl` (ripple policy, timeline preview, scene switch, view state, gestures, undo and redo) execute in `session_control`. `commit` reserves the generation, refuses lifecycle commands, which their library or persistence owner executes, then dispatches by the declared effect; everything else is an edit candidate.

`commit` keeps three admission steps because they choose the input of the edit pipeline: `timeline.edit` unwraps into a batch with its own ripple policy, a reviewed span cut (`remove_pauses`, `transcript.delete_range`, `remove_fillers`, also nested in a batch) forces ripple, and `sounds.add_to_timeline` runs as a transaction of two edits.

The Session clipboard snapshot is restored by `Checkpoint` alongside the revision,
but deliberately excluded from `State` and history snapshots. Committed identity
changes clear it under the [clipboard contract](../README.md#keyframe-clipboard).

Pure state, selection, history and command operations return the edit owner's Result.
The existing `transaction` seam is generic over the actual caller error with
`From<EditError>`: Kernel refusals convert once and callback errors return unchanged.
Rollback and ID retirement follow the same rules regardless of that caller's error type.
