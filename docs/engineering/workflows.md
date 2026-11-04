# Workflows

## Development loop

1. Read `AGENTS.md`, the glossary, relevant ADRs, ticket/spec and owning crate README.
   Claim the ticket under the [tracker](../agents/issue-tracker.md); follow
   [orchestration](../agents/orchestration.md) for placement, branching and review.
2. Research actual source and official documentation before deciding. Use Context7
   when available and web lookup for current APIs. Cite checked sources in local
   evidence; label unavailable facts unverified. Ask the coordinator when a missing
   fact blocks correctness rather than inventing APIs, versions, limits or results.
3. Learn core/structure patterns and UX/feature behavior from the maintainer-provided
   local references listed in the ticket. Inspect consumers, implement one authorized slice and reconcile its owner docs.
   Freshly read shared files before editing; preserve unrelated changes and keep
   manifests valid throughout. Admit structure under [standards](standards.md#structural-admission).
4. Run focused checks, `cargo fmt --all`, then `cargo xtask ci`. Record evidence using
   [verification](verification.md#reports); obtain independent substantive review
   and fix findings before delivery. A green build is not runtime acceptance.

## Delivery

[Orchestration](../agents/orchestration.md) owns slice limits, review and merge steps.
Use conventional commit subjects and DCO sign-off (`git commit -s`). Implementers
hand over branch/SHAs; only the maintainer merges and pushes `main`. Hosted CI is
optional and manual-only while private. Before public contributions, add a
`pull_request` trigger and require signed-off PRs; this grants no visibility change.

## Copy imports

Copy only the ticket's required files from a read-only source. The import commit
is mechanical: rename our identifiers, remove unused features and provenance/history
comments, preserve logic. Adapt behavior in separate commits. Both stages pass
[verification](verification.md#checks); retain license notices required by
[policy](open-source.md#licensing), without source-project commentary in code.

## Upgrade policy

Use the latest stable Rust, direct crates, Actions, APIs/models and OS SDK bindings.
When adding or touching one, look up the current stable release through its official
registry/releases/docs (`cargo search`, `cargo info`, crates.io, GitHub releases,
`rustup check`); upgrade copied dependencies too. Record versions in manifests,
lockfiles, toolchain/workflow configuration, not prose. Report source and result
under [completion evidence](verification.md#reports). Offline builds cannot prove
freshness; a lockfile-only `cargo update --verbose` cannot prove direct requirements
are current. Do not install an extra freshness tool just for CI.

Choose the best current design and replace in place. Remove superseded paths;
keep no dual versions, legacy shims, deprecated paths or backward compatibility.
For pre-release persisted-format changes, bump the schema version and refuse
older stores with a typed error. Never delete user data to resolve incompatibility.
