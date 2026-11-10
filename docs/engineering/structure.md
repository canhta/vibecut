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
- Test helpers live in `tests/it/support.rs` and `tests/it/support/`; helpers shared across crates live in `testkit`. Tests are named by behaviour; app test files have no `_ui` suffix. A test module covers one command family or one workflow (`tests/it/session/ripple.rs`); a new test goes into its family's module, and no module collects unrelated families.

## Engine (`crates/engine/src`)

Lowest first; imports go down only.

| Layer | Folders |
|---|---|
| Kernel | `session/`: session, command registry, history, control wire contract; the only mutation path |
| Substrate | `runtime/` (jobs, budget, cache, publication), `store/` (library, recovery), `providers/` (consent, models), `transcript/` (ASR and cue translation) |
| Features | `subtitles/`, `analysis/`, `artifacts/` |
| Control | `control/`: admission and dispatch; may call every feature |

```mermaid
flowchart TB
  control[Control: control/] --> features
  subgraph features[Features: never import each other]
    subtitles; analysis; artifacts
  end
  features --> substrate
  subgraph substrate[Substrate]
    runtime; store; providers; transcript
  end
  substrate --> kernel[Kernel: session/]
```

Features register commands through the registry; the kernel never names a feature. Each layer declares the errors it raises and states the wire code of each variant where it is declared; a higher layer wraps a lower error, and no layer names an error of a layer or crate above it. The layer table is enforced from `xtask/modules.toml`.

## Libraries (`crates/libraries/src`)

`font_catalog/` and `sound_library/` are independent features exposed as `fonts` and `sounds`; each owns its remote transport, local cache and admission limits. Imports point down to `engine` and the domain crates.

## Desktop app (`apps/vibecut/src`)

Lowest first; imports go down only, and a window never names another window.

| Layer | Holds |
|---|---|
| Foundation | `kit/` (tokens, icons, widgets, appearance), `i18n/`, `input/`; no engine knowledge |
| Services | `services/`: playback, preview, crop, AI task progress, the editor session with autosave, capture, the GPU effects adapter |
| Shared editor context | `editor/context/`: the read context, the one write context, thumbnails, the one-selected-item read and the clip display contract every editor area names |
| Windows and areas | `home/`, `shorts/`, editor areas (`assets/`, `captions/`, `dialogs/`, `panels/`, `properties/`, `timeline/`, `widgets.rs`) |
| Frame | `editor/frame/`, the `Editor` record (`editor/state.rs`), `actions/`, `preferences/` |
| Composition root | `app/`: wiring, window registry, control attach |

```mermaid
flowchart TB
  root[Composition root: app/] --> frame
  frame[Frame: editor/frame, state.rs, actions, preferences] --> areas
  subgraph areas[Windows and areas: never import each other]
    home; shorts; assets; player; properties; timeline; dialogs; widgets
  end
  areas --> context
  context[Shared editor context: editor/context] --> services[Services: services/]
  areas --> services
  services --> foundation[Foundation: kit, i18n, input]
```

```mermaid
flowchart LR
  input[egui input] --> show["area show(State, project view)"]
  show -- actions --> frame[frame routes]
  frame -- command --> session[engine Session]
  session -- project change --> show
```

The crate root holds exactly `kit/`, `i18n/`, `input/`, `services/`, `home/`, `shorts/`, `editor/`, `actions/`, `preferences/`, `app/`, `lib.rs` and `main.rs`; a new shared module goes into `services/` or `kit/`, never beside them. The engine `Session` owns the item selection; areas read it and never keep a copy.

An editor area exposes its `State` and a `show(...)` returning actions; areas never import each other, the frame routes their actions. Captions is a tab inside `assets/`. Inside `editor/`, `xtask/modules.toml` carries a folder table: `context/` is the lowest layer, the areas above it import nothing from each other, and `frame/` and the `Editor` record route above them; each new editor module adds its own line there.

## CLI (`apps/vibecut-cli/src`)

`args/`, `output/`, `session/`, `mcp/`, and `commands/` with one module per command exposing `run(context, args)`. `lib.rs` only wires them.

## Modules and crates

Start as a folder; promote to a crate when it gains a second consumer, an independent seam, heavy dependencies or keeps breaking its import direction. Next candidates: `subtitles` and `analysis` above `engine`, and the edit algebra out of `session/edit`. Why: [ADR 0004](../adr/0004-module-structure.md).

## Enforcement

`cargo xtask modules` (in `checks`) enforces rules 1–4: `//!` lines, no `mod.rs`, banned names, folder width and the layers in `xtask/modules.toml` (including `[[crate.folder]]` tables for areas inside a module). `cargo xtask docs` checks that every crate README maps its folder modules and every folder module has a README. `xtask/modules-baseline.txt` lists known violations and may only shrink.
