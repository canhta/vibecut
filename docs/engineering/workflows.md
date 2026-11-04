# Workflows

## Development loop

1. **Read** the ticket, its milestone `map.md`, the glossary, ADRs for the area and
   the README of every crate you touch. Done when you can name the owner of each
   piece you will change.
2. **Research** every API, crate, codec or provider fact you rely on: Context7 first,
   then official docs, then upstream source. Cite sources in your report; ask the
   coordinator when a missing fact blocks correctness.
3. **Learn from references** in `.spikes/repos/` (main checkout): structure and core
   from the core reference, UI/UX and features from the editor references the ticket
   names. Copy useful code under [copy imports](#copy-imports).
4. **Implement** one ticket. Reuse the existing owner of any logic; re-read shared
   files right before editing them and keep other workers' changes intact. Update the
   owning docs in the same change.
5. **Verify** with the [smallest covering check](verification.md#checks), then the evidence
   [verification](verification.md) asks for. Done when every Verify item has a
   command and a result.

Commits: conventional subject, `git commit -s`.

## Copy imports

Copy only the files the ticket needs from a read-only reference. First commit:
mechanical import with our names, unused features removed and provenance comments
dropped. Later commits: adaptations. Both pass `cargo xtask ci`, including the name
gate. Copied code carries no source names or notices; the maintainer settles
licensing with the owners directly.

## Upgrade policy

Use the latest stable Rust, crates, Actions, APIs and models. When adding or
touching a dependency, look up its current release (`cargo info`, crates.io,
official releases) and record the lookup in the report; versions live in manifests
and lockfiles.

Replace in place: one version of every path, superseded code deleted in the same
change. A persisted-format change bumps the schema version and refuses older files
with a typed error; user data is never deleted.

Accepted third-party limitations are documented, not patched. The one exception is
an upstream crash or security fix not yet released: backport it as a removable
patch per [ADR 0002](../adr/0002-desktop-media-and-time.md).

## FFmpeg provisioning

`cargo xtask ffmpeg` builds the latest stable LGPL FFmpeg from verified source and
logs the resolved version; keep the sources and receipt with every distributed
binary. Windows builds need MSYS2 CLANG64 (x64) or CLANGARM64 (arm64) with Bash,
Make, CMake, Ninja, pkg-config, NASM and matching Clang/LLVM, run on the matching
architecture; Rust keeps its MSVC toolchain.

## Hosted CI

Manual-only while the repo is private. Before accepting public contributions, add a
`pull_request` trigger and require signed-off PRs.
