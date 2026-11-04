# Issue tracker: local markdown

Issues live as markdown files in `.scratch/` (git-ignored, never pushed). No GitHub Issues.

## Layout

- One feature per directory: `.scratch/<feature-slug>/`
- Spec: `.scratch/<feature-slug>/spec.md`
- One ticket per file: `.scratch/<feature-slug>/issues/NN-<slug>.md`, numbered from `01`

## Ticket header

The first lines of every ticket, one per line, exact spelling, so agents can grep them:

```
Status: open | claimed | blocked | resolved | wontfix
Type: task | research | bug
Priority: P0 | P1 | P2
Blocked by: NN, NN        # omit when none
```

Body follows. Discussion appends under `## Comments`; the outcome goes under `## Answer`.

## Queries

- Open work: `grep -l '^Status: open' .scratch/*/issues/*.md`
- P0 open: `grep -l '^Priority: P0' $(grep -l '^Status: open' .scratch/*/issues/*.md)`
- What blocks NN: `grep -H '^Blocked by:' .scratch/<feature>/issues/*.md`

A ticket is unblocked when every ticket in its `Blocked by:` line is `resolved`. Frontier: open, unblocked, lowest number first.

## Operations

- **Publish**: create the file under `.scratch/<feature-slug>/issues/` with the header above.
- **Fetch**: read the path or number the user gives.
- **Claim**: set `Status: claimed` and save before starting.
- **Resolve**: write `## Answer`, set `Status: resolved`, and add a one-line pointer in `.scratch/<feature-slug>/map.md` under `Decisions`.
