# Workflows

## Development loop

Read `AGENTS.md`, `GLOSSARY.md`, relevant ADRs, then open tasks using the queries
in [the local issue tracker](../agents/issue-tracker.md). Read the README of the
crate you touch when present. Keep tasks and progress in `.scratch/`.

Research actively before deciding: read actual source and official documentation
(use Context7 when available), and search the web for current API/library facts.
Never guess or invent APIs, versions, limits or results. Cite what you checked;
if a needed fact cannot be verified, label it unverified and ask.

Work one focused slice: inspect current source, change its owner, run relevant
checks, obtain independent substantive review and resolve findings before delivery.
Preserve concurrent work. Parallel agents use separate git worktrees and
`CARGO_TARGET_DIR=target/agent-<name>`. Keep every Cargo manifest valid: the
`crates/*` workspace glob exposes broken manifests to every workspace command.

## Delivery

While private, the maintainer pushes reviewed slices directly to `main` after
local `cargo xtask ci` and independent review. Use conventional commit messages
and DCO sign-off (`git commit -s`). Hosted CI is optional and manual-only.

Before making the repository public, add a `pull_request` trigger to CI for
external contributions. External contributors then submit PRs with DCO sign-off.
This policy does not authorize changing repository visibility.

## Upgrade policy

Choose the best current design and replace in place. Remove superseded paths;
keep no dual versions, legacy shims, deprecated paths or backward compatibility.
For pre-release persisted-format changes, bump the schema version and refuse
older stores with a typed error. Never delete user data to resolve incompatibility.
