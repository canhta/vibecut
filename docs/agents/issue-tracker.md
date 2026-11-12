# Issue tracker

Canonical tickets live in git-ignored `.scratch/` in the master coordinator's main checkout: plain files, the single source of truth, no tracker repository. `.spikes/` and `.samples/` sit beside it, also git-ignored. Workers read and edit assigned ticket files by absolute path (never stage, delete or overwrite another ticket's files; re-read a file before editing it). The coordinator's private tooling lists ready work, ranks unresolved work by what it holds up, serves the read-only board on localhost from any linked worktree, and cleans up settled workers; it lives outside this repository.

Delegated hosts maintain working copies of their assigned contracts and tickets. At source, review and merge handbacks, transfer changed tickets, decisions and their complete linked evidence to the master: reports, raw results, failures, hashes and required visual artifacts. Preserve host and revision provenance; reconcile each ticket into one canonical record and retain incoming copies separately when they differ. The master's verified local files must support audit and continuation with the delegated host offline. A transfer is complete when its manifest matches local file hashes and all required linked artifacts are present; record missing evidence explicitly. This custody check reuses the host's technical review and proof. The private host inventory names delegation, transfer locations and current assignments.

| Path under `.scratch/` | Owns |
|---|---|
| `<milestone>/spec.md` | Scope |
| `<milestone>/map.md` | One line per decision |
| `<milestone>/issues/NN-slug.md` | Ticket |
| `<milestone>/reviews/` | Review reports |
| `agents.md` | opencode track record per work type |

Ticket headers use these exact fields; omit `Blocked by` when empty:

```text
Status: open | claimed | blocked | resolved | wontfix
Type: task | research | bug
Priority: P0 | P1 | P2 | P3 | P4
Assignee: <ide>/<model> | human | -
Blocked by: - | NN, <milestone>/NN
```

`Assignee` is the exact model id the coordinator passes to the worker's `--model`, for example `opencode-go/deepseek-v4.1-flash` (run by opencode) or `claude-sonnet-5-5` (run by Claude Code). Older resolved tickets keep their historical values. `Blocked by` holds ticket ids only; put any condition or note in the body.

`claimed` means the current active assignment, including explicitly admitted planning or source work whose checks await build capacity; it is not a lifetime claim on a worker, checkout or parent feature. Keep a broad parent `open` with its acceptance retained while a source leaf is active. A paused assignment records its precise condition in the body and is `blocked` when intervention is needed; a queued but otherwise ready leaf is `open`. Only unmerged code/contract tickets go in `Blocked by`; resource admission, scheduling and human evidence conditions stay in the body. Record unrun checks as unverified, independently of source progress.

Use the body to record delivery stage without new status values: **planning** (no source admission), **implemented** (published SHA and receipt), **reviewed** (verdict and remaining acceptance), **merged** (main SHA). A staged approval on unfinished WIP remains explicitly accepted-unmerged, not resolved or shipping. Retain its refs, evidence and next integration boundary. Resolve only the delivered leaf after merge; resolve its parent only after all acceptance. Keep one current task/dispatch identity distinct from settled history and paused/uncertain assignments; uncertain liveness never proves exit.

Priority follows the [delivery order](../product/requirements.md#scope): P0 foundation, P1 core editor, P2 AI editing and agent control, P3 Home, P4 long video to shorts; a defect takes the priority of the feature it breaks. Bare numbers mean the same milestone. A ticket is unblocked when every blocker is `resolved`; `wontfix` records its reason. Put the outcome under `## Answer`.

Ready order: highest priority, then most open tickets transitively blocked, then lowest number; a blocking view ranks unresolved tickets by unresolved transitive dependents. Claiming, resolving and routing follow [orchestration](orchestration.md).
