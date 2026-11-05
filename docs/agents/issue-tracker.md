# Issue tracker

Tickets live in git-ignored `.scratch/` in the main checkout; `cargo xtask frontier [assignee]` lists ready work and `cargo xtask board [port]` serves the read-only live board on localhost from any linked worktree.

| Path under `.scratch/` | Owns |
|---|---|
| `<milestone>/spec.md` | Scope |
| `<milestone>/map.md` | One line per decision |
| `<milestone>/issues/NN-slug.md` | Ticket |
| `<milestone>/reviews/` | Review reports |
| `agents.md` | Implementer tiers and track record |

Ticket headers use these exact fields; omit `Blocked by` when empty:

```text
Status: open | claimed | blocked | resolved | wontfix
Type: task | research | bug
Priority: P0 | P1 | P2
Assignee: opencode | codex | human | -
Blocked by: NN, <milestone>/NN
```

Bare numbers mean the same milestone. A ticket is unblocked when every blocker is `resolved`; `wontfix` records its reason. Put the outcome under `## Answer`.

`cargo xtask frontier [assignee]` lists open, unblocked tickets by highest priority, then lowest number. Claiming, resolving and routing follow [orchestration](orchestration.md).
