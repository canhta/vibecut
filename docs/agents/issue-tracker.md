# Issue tracker: local markdown

Tickets live in git-ignored `.scratch/` in the main checkout.

## Layout

```
.scratch/<milestone>/spec.md            scope
.scratch/<milestone>/map.md             one line per decision
.scratch/<milestone>/issues/NN-slug.md  one ticket
.scratch/<milestone>/reviews/           review reports
.scratch/agents.md                      implementer tiers and track record
```

## Ticket header

First lines, exact spelling:

```
Status: open | claimed | blocked | resolved | wontfix
Type: task | research | bug
Priority: P0 | P1 | P2
Assignee: opencode | codex | human | -
Blocked by: NN, <milestone>/NN
```

Bare numbers mean the same milestone. Omit `Blocked by` when empty. The body follows;
the outcome goes under `## Answer`. A ticket is **unblocked** when every blocker is
`resolved`. The **frontier** is open, unblocked tickets, highest priority then lowest number
(`.scratch/bin/frontier`).
`wontfix` carries its reason.

## Queries

```sh
grep -l '^Status: open' .scratch/*/issues/*.md
grep -H '^Blocked by:' .scratch/<milestone>/issues/*.md
```

Claiming, resolving and routing follow [orchestration](orchestration.md).
