# Structure

What is inside a crate or app and which way imports point. Crates and their contracts: [architecture](architecture.md). File length is never a rule.

## Rules

1. One responsibility per module, stated in its `//!` line in one sentence without "and". Name it after the concept (`timeline`, `recovery`); never `utils`, `common`, `manager`, `helpers`, `misc`, `core`, `types`.
2. A folder holds one layer or one feature. Children with a shared prefix (`clip_*`) or two kinds become a folder. More than 12 direct children means regroup.
3. Layout is `foo.rs` plus `foo/`, never `mod.rs`. `foo.rs` declares private children and re-exports only the interface; nothing imports another module's internals. Visibility: private, then `pub(super)`, `pub(crate)`, `pub` only for the crate interface. Fields private by default.
4. Imports point down the layer table. Features never import each other; shared logic moves to the lowest layer both can use.
5. Tests call the module's interface. Needing internals means the boundary is wrong.
6. New code goes, in order: into the existing owner; a new module in the owning folder; a new folder when a second responsibility appears; a new crate only with a second consumer or an independent seam ([crate map](architecture.md#crate-map)).
7. Add a trait or registry only when two implementations already vary across it.
8. Review asks: is the code in its owner, did an interface grow, does each module pass rule 1, did an import cross a layer or feature. Any "no" is `changes`.

## Naming

- Modules and types use [GLOSSARY](../../GLOSSARY.md) terms, singular. A new concept enters the glossary first.
- Providers are `<family>/<operation>` (`providers/anthropic/translate`).
- Test helpers live in `tests/it/support.rs` and `tests/it/support/`; helpers shared across crates live in `testkit`. Tests are named by behaviour; app test files have no `_ui` suffix.

## Engine (`crates/engine/src`)

Lowest first; imports go down only.

| Layer | Folders |
|---|---|
| Kernel | `session/`: session, command registry, history, control wire contract; the only mutation path |
| Substrate | `runtime/` (jobs, budget, cache, publication), `store/` (library, recovery), `providers/` (consent, models), `transcript/` (ASR and cue translation) |
| Features | `subtitles/`, `analysis/`, `artifacts/`, `libraries/` |
| Control | `control/`: admission and dispatch; may call every feature |

Features register commands through the registry; the kernel never names a feature. The layer table is enforced from `xtask/modules.toml`.

## Desktop app (`apps/vibecut/src`)

Lowest first; imports go down only, and a window never names another window.

| Layer | Holds |
|---|---|
| Foundation | `kit/`, `i18n/`, `input/`, `appearance.rs`; no engine knowledge |
| Services | Playback, preview frames, job progress, pickers |
| Windows and areas | `home/`, `shorts/`, `editor/panels/{assets,player,properties,timeline}`, editor dialogs |
| Frame | `editor/frame/`, `actions/`, `preferences/`, `workspace/`, `control/` |
| Composition root | App wiring, window registry, control attach |

An editor area exposes its `State` and a `show(...)` returning actions; areas never import each other, the frame routes their actions. Captions is a tab inside `assets/`.

## CLI (`apps/vibecut-cli/src`)

`args/`, `output/`, `session/`, `mcp/`, and `commands/` with one module per command exposing `run(context, args)`. `lib.rs` only wires them.

## Modules and crates

Start as a folder; promote to a crate when it gains a second consumer, an independent seam, heavy dependencies or keeps breaking its import direction. Next candidates: `subtitles`, `analysis` and `libraries` above `engine`, and the edit algebra out of `session/edit`. Why: [ADR 0004](../adr/0004-module-structure.md).

## Enforcement

`cargo xtask modules` (in `checks`) enforces rules 1–4: `//!` lines, no `mod.rs`, banned names, folder width and the layers in `xtask/modules.toml` (including `[[crate.folder]]` tables for areas inside a module). `cargo xtask docs` checks that every crate README maps its folder modules and every folder module has a README. `xtask/modules-baseline.txt` lists known violations and may only shrink.
