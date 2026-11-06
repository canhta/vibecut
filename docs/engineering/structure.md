# Structure

Where code lives, how it is grouped, and how modules depend on each other. [Architecture](architecture.md) owns crates and their contracts; this page owns what is inside a crate or app. File length is never a rule: a module is judged by its responsibility, its interface and its place in the dependency direction.

## Vocabulary

- **Module**: a file or folder with an interface and an implementation.
- **Interface**: everything a caller must know: signatures, invariants, ordering, errors, performance. For a folder module the interface is its module file `foo.rs`, which sits next to the folder `foo/`; everything else in the folder is private.
- **Deep module**: much behaviour behind a small interface. **Shallow module**: an interface as large as its body (a pass-through, a bag of helpers).
- **Seam**: where behaviour can change without editing in place; the interface lives there. One adapter makes a hypothetical seam, two make a real one: add a trait or registry only when two things already vary across it (providers, languages, codecs, platforms, UI areas, command sets).
- **Deletion test**: delete the module in your head. If complexity vanishes it was a pass-through; if it reappears in several callers, the module earns its place.

## Rules

1. **One responsibility, said in one sentence.** Every module starts with a `//!` line stating it in one sentence without "and". If the sentence needs "and", split by the two responsibilities. Name modules after the concept (`timeline`, `recovery`), never `utils`, `common`, `manager`, `helpers`, `misc`, `core`, `types`.
2. **The folder is the grouping unit.** A folder holds one layer or one feature, and its direct children are siblings of one kind. When children fall into two groups, or when two of them share a prefix (`clip_*`, `shot_*`, `control_*`), make a folder. More than about 12 direct children is a prompt to regroup, not a number to game.
3. **Interface at the folder door.** Layout is `foo.rs` plus `foo/`, never `foo/mod.rs` (the Rust Book's current style, already used by `cache.rs`/`cache/`; do not mix styles in one module). `foo.rs` states the responsibility, declares children `mod x;` (private), and re-exports only the interface (`pub use`). Siblings and other folders reach a module only through that interface; nothing imports another module's internals. Default visibility is private, then `pub(super)`, then `pub(crate)`; `pub` only for the crate's own interface. Struct fields are private by default and a public field is a commitment ([API Guidelines C-STRUCT-PRIVATE](https://rust-lang.github.io/api-guidelines/future-proofing.html)); hide an implementation type behind a newtype when callers need less than it offers; seal a trait whose implementers must stay ours; every crate and every module file carries a `//!` doc that states responsibility.
4. **Dependencies point one way** (below). A lower layer never imports a higher one. Features never import each other; shared logic moves down to the lowest layer that both can use, or the two communicate through a seam owned by the layer below.
5. **Tests cross the interface.** A module's tests call its interface; a behaviour that can only be tested through internals means the interface or the module boundary is wrong. Test files follow the module they test ([Naming](#naming)).
6. **Where does new code go** (ask in this order): an existing owner of the concept, extended through its interface; a new module inside the folder of the layer and feature that owns it; a new folder only when a second responsibility appears; a new crate only when the boundary must be compiler-enforced and a second consumer or an independent seam exists ([architecture](architecture.md#crate-map)); see Modules and crates below. A ticket names the owning module, says whether it adds a module, and does not widen an interface without saying why.
7. **Review asks four questions:** is the code in its owner; did an interface grow, and could it stay small; does each touched module still pass the one-sentence test; did an import cross a layer or a feature. Any "no" is `changes`.

## Naming

One convention, taken from [GLOSSARY](../../GLOSSARY.md):

- Modules and folders are singular concept nouns from the glossary (`thumbnail`, `recovery`). Two spellings side by side (`thumbnail` and `thumbnails`) are a defect.
- A shared prefix becomes a folder: `clip_export` becomes `shorts/export`.
- Provider modules are `<family>/<operation>`: `providers/anthropic/translate`.
- Types use glossary terms; a new concept enters the glossary before it enters a public type.
- Test files are named by behaviour area and test functions are behaviour sentences: `trim_refuses_past_source_end`.
- Test helpers are dev-only and live in `tests/it/support.rs` plus `tests/it/support/`, never `common/` or `mod.rs`.
- App test files carry no `_ui` suffix: the binary is the app.

## Engine (`crates/engine/src`)

Layers, lowest first. Imports go downward only.

| Layer | Holds | Responsibility |
|---|---|---|
| Kernel | `session/` | Session, command registry, history and transactions; the control wire contract (revision, requests, response, limits); the only mutation path |
| Substrate | `runtime/` | Job runner, byte budget, content-addressed cache, staged publication, verified transfer |
| Substrate | `store/` | Managed project library, session recovery and the project cover image used by both |
| Substrate | `providers/` | Provider choice with explicit consent, local model availability and transfers |
| Substrate | `transcript/` | Speech windows, ASR checkpoints and cue translation runs; the one cue path used by `subtitles/` and `shorts/` |
| Features | `subtitles/` | Subtitle workflow: orchestration, SRT export, burn-in, ASS import |
| Features | `shorts/` | Long video to shorts: clip suggestion, boundary snapping, batch export |
| Features | `analysis/` | Source analysis that never mutates a project: scene detection, shot analysis, loudness |
| Features | `artifacts/` | Derived images: source and frame thumbnails, frame snapshots, effect images, bounded control previews |
| Features | `libraries/` | External asset libraries: sounds, fonts |
| Control | `control/` | Control admission, receipts, resynchronization and dispatch; the only layer that may call every feature |

Features register their commands and control schemas with the kernel through the command registry seam (one `execute(id, params)` entry for every command; many command sets already vary across it); the kernel never names a feature. `session/edit.rs` exposes the editing command families under `session/edit/`, one module per family (clips, tracks, text, effects, transitions, retime, audio). `xtask/modules.toml` carries this table; `xtask/modules-baseline.txt` lists the violations still to remove.

## Desktop app (`apps/vibecut/src`)

Layers, lowest first. Imports go downward only, and a window never names another window.

| Layer | Holds | Responsibility |
|---|---|---|
| Foundation | `kit/`, `i18n/`, `input/` | Design tokens, icons and shared widgets; the one text catalog; chords, bindings and action dispatch. Knows nothing about the engine |
| Services | playback, preview frames, job progress, pickers, preference store | One interface each, shared by more than one window; no window state |
| Windows and areas | `home/`, `shorts/`, `editor/panels/{assets,player,properties,timeline}`, editor dialogs | One module per window or area behind one interface; each window owns its size, top bar and chrome; each area owns one panel's or dialog's state |
| Frame | `editor/frame/` | The editor's top bar, window layout and the routing that calls each area and returns its actions |
| Composition root | app wiring, window registry, control attach | Names every window and lends services; the only layer above the windows |

An editor area is one module whose interface is its `State` and its `show(...)` returning the actions it wants; split inside only when the one-sentence test fails, never into a fixed `state.rs` / `view/` / `workflow.rs` triad. Captions is a feature tab inside `assets/` ([UI design](../product/ui/index.md)), not a fifth panel; the dialogs are `export`, `models` and `settings`; the shortcut list is its own window. Areas never import each other: the frame calls each area, passes the shared context (selection, engine handle) and routes the returned actions. Home, Editor and Long video to shorts import only foundation and service interfaces; they never name each other or the composition root.

## CLI (`apps/vibecut-cli/src`)

`args/` (parsing and typed exit codes), `output/`, `session/` (control-session lifetime, project lease, export admission), `mcp/` (tools and authenticated transport), `commands/` (one module per command, each with `run(context, args) -> Result<Output, CliError>`; a folder groups commands only when they share code, and then exposes each command's `run`, not a group-level dispatcher, which would be a pass-through); `lib.rs` only wires them.

## Modules and crates

Rule: start as a folder with the import direction enforced by `cargo xtask modules`; promote a folder to a crate (named for the concept, placed in its layer in the crate map) when it has a second consumer, an independent seam (a provider family, a codec, a platform), heavy dependencies the rest should not rebuild for, or its direction keeps being violated. Candidates, in order: the feature layers above the engine kernel (`subtitles`, `shorts`, `analysis`, `libraries`) as crates layered above `engine`, and the pure edit algebra out of `session/edit`. The folder split is step one, so the later cut is mechanical. A kit crate (`kit/`) follows when a second UI consumer (the audit example already is one) justifies it. [ADR 0004](../adr/0004-module-structure.md) records why folders come first and which parts of the studied reference we took.

## Other crates

Same rules, applied when a ticket touches them: each crate's README lists its modules by responsibility, and a folder appears when two modules share a concept. `media`, `render`, `ai`, `text`, `clip-eval`, `captions`, `project` and `time` keep their crate map entries; regroup inside only when the one-sentence test or the folder rule fails.

## Enforcement

`cargo xtask modules` (run by `checks` and `ci`) enforces the mechanical parts of rules 1-4 over tracked Rust sources under `apps/`, `crates/` and `xtask/`. A `mod.rs` file fails everywhere (rule 3), and no module file, folder or inline `mod` block carries a banned name (compared whole and ignoring case); both apply under `tests/`, `examples/` and `benches/` too. The rest skips those three directories and `tests.rs`/`*_tests.rs` files: every other source file starts with a non-empty `//!` responsibility line; a folder with more than 12 direct children fails (a `foo.rs` beside `foo/` counts once); and every intra-crate `crate::`, `self::` or `super::` path in non-`#[cfg(test)]` code, including a name the crate root re-exports, follows the layer table in `xtask/modules.toml`: a lower layer may not reach a higher one, feature groups in one layer may not reach each other, and code outside a folder module may not pass through its `<name>.rs` file into its private children unless that file exports the next segment (the folder's own children may use each other). A crate may also declare `[[crate.folder]]` tables (`path = "src/<folder>"` with their own layers and modules) that apply the same layer and group rules between the children of that folder, so areas inside one module stay independent; the deepest table covering both sides of an import decides, and every child of the folder must be placed. `xtask/modules-baseline.txt` lists current violations as `rule<TAB>path<TAB>detail`, sorted; an unlisted violation fails and a listed one that no longer occurs must be removed, so the list only shrinks. Reviewers apply the four questions in rule 7 where a script cannot.
