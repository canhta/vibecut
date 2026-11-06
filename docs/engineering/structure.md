# Structure

Where code lives, how it is grouped, and how modules depend on each other. [Architecture](architecture.md) owns crates and their contracts; this page owns what is inside a crate or app. File length is never a rule: a module is judged by its responsibility, its interface and its place in the dependency direction.

## Vocabulary

- **Module**: a file or folder with an interface and an implementation.
- **Interface**: everything a caller must know: signatures, invariants, ordering, errors, performance. For a folder module the interface is its `mod.rs`; everything else in it is private.
- **Deep module**: much behaviour behind a small interface. **Shallow module**: an interface as large as its body (a pass-through, a bag of helpers).
- **Seam**: where behaviour can change without editing in place; the interface lives there. One adapter makes a hypothetical seam, two make a real one: add a trait or registry only when two things already vary across it (providers, languages, codecs, platforms, UI areas, command sets).
- **Deletion test**: delete the module in your head. If complexity vanishes it was a pass-through; if it reappears in several callers, the module earns its place.

## Rules

1. **One responsibility, said in one sentence.** Every module starts with a `//!` line stating it in one sentence without "and". If the sentence needs "and", split by the two responsibilities. Name modules after the concept (`timeline`, `recovery`), never `utils`, `common`, `manager`, `helpers`, `misc`, `core`, `types`.
2. **The folder is the grouping unit.** A folder holds one layer or one feature, and its direct children are siblings of one kind. When children fall into two groups, or when two of them share a prefix (`clip_*`, `shot_*`, `control_*`), make a folder. More than about 12 direct children is a prompt to regroup, not a number to game.
3. **Interface at the folder door.** A folder's `mod.rs` states the responsibility, declares children `mod x;` (private), and re-exports only the interface. Siblings and other folders reach a module only through that interface; nothing imports another module's internals. Default visibility is private, then `pub(super)`, then `pub(crate)`; `pub` only for the crate's own interface.
4. **Dependencies point one way** (below). A lower layer never imports a higher one. Features never import each other; shared logic moves down to the lowest layer that both can use, or the two communicate through a seam owned by the layer below.
5. **Tests cross the interface.** A module's tests call its interface; a behaviour that can only be tested through internals means the interface or the module boundary is wrong. Test files follow the module they test and are named by behaviour.
6. **Where does new code go** (ask in this order): an existing owner of the concept, extended through its interface; a new module inside the folder of the layer and feature that owns it; a new folder only when a second responsibility appears; a new crate only when a second consumer or an independent seam exists ([architecture](architecture.md#crate-map)). A ticket names the owning module, says whether it adds a module, and does not widen an interface without saying why.
7. **Review asks four questions:** is the code in its owner; did an interface grow, and could it stay small; does each touched module still pass the one-sentence test; did an import cross a layer or a feature. Any "no" is `changes`.

## Engine (`crates/engine/src`)

Layers, lowest first. Imports go downward only.

| Layer | Folder | Responsibility |
|---|---|---|
| Kernel | `session/` | Session, command registry, history, transactions; the only mutation path |
| Kernel | `control/` | Transport-neutral control admission, receipts, snapshot resync |
| Substrate | `runtime/` | Job runner, byte budget, content-addressed cache, staged publication |
| Substrate | `store/` | Managed project library and session recovery |
| Features | `subtitles/` | Subtitle workflow: speech windows, ASR checkpoints, translation, ASS import |
| Features | `shorts/` | Long video to shorts: clip suggestion, boundary snapping, batch export |
| Features | `analysis/` | Source analysis that never mutates a project: scene detection, shot analysis, loudness |
| Features | `artifacts/` | Derived images: thumbnails, snapshots, effect thumbnails, bounded control previews |
| Features | `libraries/` | External asset libraries: sounds, fonts |
| Features | `providers/` | Provider selection with consent, local model transfers |

Features register their commands and control schemas with the kernel through the command registry seam (many command sets already vary across it); the kernel never names a feature. Known inversions to remove, found by import scan: `session` imports `fonts`; `control` imports `subtitles`, `fonts`, `snapshot`, `control_preview`; `recovery` and `library` import `thumbnail`. `edit.rs` (editing command families) becomes `session/edit/` with one module per command family (clips, tracks, text, effects, transitions, retime, audio).

## Desktop app (`apps/vibecut/src`)

| Folder | Responsibility |
|---|---|
| `shell/` | Windows, workspace lifecycle, recovery presentation, preferences, control attach, capture |
| `kit/` | Design tokens, icon set, shared widgets (tab strip, section header, property row, slider and stepper, icon button, popover). Knows nothing about the engine |
| `i18n/` | The one catalog, rows grouped by screen, locale and preference ownership |
| `input/` | Chords, keybindings, action registry, key dispatch |
| `home/` | Home window: project library, create, search, trash |
| `editor/` | Editor window: `frame/` (top bar, layout, strips) and one folder per area |
| `shorts/` | Long-video-to-shorts window |

Editor areas, each a folder `editor/<area>/`: `assets/` (media, audio, text, effects, transitions, import), `player/` (preview, playback, crop, snapshot), `properties/` (inspector, colour picker), `timeline/` (tracks, toolbar, snapping, selection), `captions/` (subtitles panel), `export/`, `models/` (model manager, AI consent). Inside an area: `mod.rs` is the interface (its `State`, `show(...)` returning the actions it wants), `state.rs`, `view/` (widgets), `workflow.rs` (jobs and engine calls). Areas never import each other: the frame calls each area, passes the shared context (selection, engine handle) and routes the returned actions. Home, Editor and Shorts are independent windows and import only `kit/`, `i18n/`, `input/` and `shell/` interfaces.

## CLI (`apps/vibecut-cli/src`)

`args/` (parsing and typed exit codes), `output/`, `session/` (control-session lifetime, project lease, export admission), `mcp/` (tools and authenticated transport), `commands/<group>/` (one module per command group with a single `run(context, args) -> Result<Output, CliError>` interface); `lib.rs` only wires them.

## Other crates

Same rules, applied when a ticket touches them: each crate's README lists its modules by responsibility, and a folder appears when two modules share a concept. `media`, `render`, `ai`, `text`, `captions`, `project` and `time` keep their crate map entries; regroup inside only when the one-sentence test or the folder rule fails.

## Enforcement

`cargo xtask modules` (run by `checks` and `ci`): every non-test source file has a `//!` responsibility line; no module is named from the banned list; a folder with more than 12 direct children fails unless baselined; intra-crate imports must follow the layer table in `xtask/modules.toml` (an import from a lower to a higher layer, or between features, fails unless baselined). The baseline lists current violations and only shrinks: a fixed entry must be removed. Reviewers apply the four questions in rule 7 where a script cannot.
