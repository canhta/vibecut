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
5. **Tests cross the interface.** A module's tests call its interface; a behaviour that can only be tested through internals means the interface or the module boundary is wrong. Test files follow the module they test and are named by behaviour.
6. **Where does new code go** (ask in this order): an existing owner of the concept, extended through its interface; a new module inside the folder of the layer and feature that owns it; a new folder only when a second responsibility appears; a new crate only when the boundary must be compiler-enforced and a second consumer or an independent seam exists ([architecture](architecture.md#crate-map)); see Modules and crates below. A ticket names the owning module, says whether it adds a module, and does not widen an interface without saying why.
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

Features register their commands and control schemas with the kernel through the command registry seam (the same idea as FilmCraft's `Session::execute(id, params)` registry) (many command sets already vary across it); the kernel never names a feature. Known inversions to remove, found by import scan: `session` imports `fonts`; `control` imports `subtitles`, `fonts`, `snapshot`, `control_preview`; `recovery` and `library` import `thumbnail`. `edit.rs` (editing command families) becomes `session/edit.rs` plus `session/edit/` with one module per command family (clips, tracks, text, effects, transitions, retime, audio).

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

Editor areas, each a module `editor/<area>.rs` with folder `editor/<area>/`: `assets/` (media, audio, text, effects, transitions, import), `player/` (preview, playback, crop, snapshot), `properties/` (inspector, colour picker), `timeline/` (tracks, toolbar, snapping, selection), `captions/` (subtitles panel), `export/`, `models/` (model manager, AI consent). Inside an area: `<area>.rs` is the interface (its `State`, `show(...)` returning the actions it wants); the folder holds `state.rs`, `view/` (widgets) and `workflow.rs` (jobs and engine calls). Areas never import each other: the frame calls each area, passes the shared context (selection, engine handle) and routes the returned actions. Home, Editor and Shorts are independent windows and import only `kit/`, `i18n/`, `input/` and `shell/` interfaces.

## CLI (`apps/vibecut-cli/src`)

`args/` (parsing and typed exit codes), `output/`, `session/` (control-session lifetime, project lease, export admission), `mcp/` (tools and authenticated transport), `commands/<group>/` (one module per command group with a single `run(context, args) -> Result<Output, CliError>` interface); `lib.rs` only wires them.

## Modules and crates

The reference app (FilmCraft) gets its structure from crates, not folders: about 50 small crates in numbered layers L0 to L6, edges only downward, enforced by `cargo xtask layers`, with `speech`, `tts`, `captions`, `interchange`, `export` and the pure edit algebra each their own crate. Inside its `engine` and `ui-egui` crates it is flat (100+ files, `misc.rs`); we take the crate layering and the command registry, not that folder style. rust-analyzer and the large-workspace guidance agree: the compiler enforces a boundary only at a crate edge, internal crates are not API boundaries, and a flat `crates/<name>/` layout scales.

Rule: start as a folder with the import direction enforced by `cargo xtask modules`; promote a folder to a crate (named for the concept, placed in its layer in the crate map) when it has a second consumer, an independent seam (a provider family, a codec, a platform), heavy dependencies the rest should not rebuild for, or its direction keeps being violated. Candidates, in order: the feature layers above the engine kernel (`subtitles`, `shorts`, `analysis`, `libraries`) as crates layered above `engine`, and the pure edit algebra out of `session/edit`. The folder split is step one, so the later cut is mechanical. A kit crate (`kit/`) follows when a second UI consumer (the audit example already is one) justifies it.

## Other crates

Same rules, applied when a ticket touches them: each crate's README lists its modules by responsibility, and a folder appears when two modules share a concept. `media`, `render`, `ai`, `text`, `captions`, `project` and `time` keep their crate map entries; regroup inside only when the one-sentence test or the folder rule fails.

## Enforcement

`cargo xtask modules` (run by `checks` and `ci`): every non-test source file has a `//!` responsibility line; no module is named from the banned list; a folder with more than 12 direct children fails unless baselined; intra-crate imports must follow the layer table in `xtask/modules.toml` (an import from a lower to a higher layer, or between features, fails unless baselined). The baseline lists current violations and only shrinks: a fixed entry must be removed. Reviewers apply the four questions in rule 7 where a script cannot.
