# Issue tracker

Tickets live in git-ignored `.scratch/` in the main checkout of every host. `.scratch/`, `.spikes/` and `.samples/` are links into a clone of the maintainer's private workspace repository (`VibeCut-workspace/` beside the checkout); large media stay out of it (its `media.md`). Before reading tickets run `git pull --rebase` there; after changing any tracker file commit it with sign-off and push at once, so all hosts see one tracker; `cargo xtask frontier [assignee]` lists ready work and `cargo xtask board [port]` serves the read-only live board on localhost from any linked worktree.

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

Frontier order: highest priority, then most open tickets transitively blocked, then lowest number; `frontier --blocking` ranks unresolved tickets by unresolved transitive dependents. Claiming, resolving and routing follow [orchestration](orchestration.md).
