# Issue tracker

Tickets live in git-ignored `.scratch/` in the coordinator's main checkout: plain files, the single source of truth, no tracker repository. `.spikes/` and `.samples/` sit beside it, also git-ignored; large media stay local. Workers on the coordinator's machine read and edit these files by absolute path (never stage, delete or overwrite another ticket's files; re-read a file before editing it). Workers on a remote host get the ticket inline and write reports and reviews under their own worktree's `.scratch/<milestone>/`; the coordinator transfers them into the main checkout and applies status, answer and map changes. The coordinator's private tooling lists ready work, ranks unresolved work by what it holds up, serves the read-only board on localhost from any linked worktree, and cleans up settled workers; it lives outside this repository.

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

`Assignee` names the agent tool and model that implement the ticket as `ide/model`, for example `opencode/deepseek-v4.1-flash`, `codex/gpt-6.1-sol` or `claudecode/claude-sonnet-5-5`. `Blocked by` holds ticket ids only; put any condition or note in the body.

Priority follows the [delivery order](../product/requirements.md#scope): P0 foundation, P1 core editor, P2 AI editing and agent control, P3 Home, P4 long video to shorts; a defect takes the priority of the feature it breaks. Bare numbers mean the same milestone. A ticket is unblocked when every blocker is `resolved`; `wontfix` records its reason. Put the outcome under `## Answer`.

Ready order: highest priority, then most open tickets transitively blocked, then lowest number; a blocking view ranks unresolved tickets by unresolved transitive dependents. Claiming, resolving and routing follow [orchestration](orchestration.md).
