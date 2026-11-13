# vibecut

Native eframe/egui desktop frontend; wgpu presents render's shared CPU compositor.
Owns device IO, the audio transport clock, retained preview frames, presentation
state, stable automation IDs, screenshots, the catalog and preferences. Engine
owns every project edit, job scheduling and aggregate byte accounting; render owns
every pixel, audio conversion, sample grid and channel mix rule.

Build and provisioning: [verification](../../docs/engineering/verification.md). The
desktop binary is `vibecut`; the headless CLI is `vibecut-cli`.

Each module's contract is its README.

| Module | README |
|---|---|
| `actions.rs` / `actions/` | [Action registry](src/actions/README.md) |
| `app.rs` / `app/` | [Composition root](src/app/README.md) |
| `editor.rs` / `editor/` | [Editor presentation](src/editor/README.md) |
| `editor/panels/assets/` | [Assets](src/editor/panels/assets/README.md) |
| `editor/panels/assets/sounds/` | [Sounds tab](src/editor/panels/assets/sounds/README.md) |
| `editor/panels/assets/view/` | [Assets view](src/editor/panels/assets/view/README.md) |
| `editor/panels/assets/workflow/` | [Assets workflow](src/editor/panels/assets/workflow/README.md) |
| `editor/panels/assets/captions.rs` | [Captions tab](src/editor/panels/assets/captions/README.md) |
| `editor/panels/assets/effects.rs` | [Effect previews](src/editor/panels/assets/effects/README.md) |
| `editor/panels/assets/import.rs` | [Media import](src/editor/panels/assets/import/README.md) |
| `editor/panels/assets/media_panel.rs` | [Media library](src/editor/panels/assets/media_panel/README.md) |
| `editor/context/` | [Editor context](src/editor/context/README.md) |
| `editor/dialogs/` | [Editor dialogs](src/editor/dialogs/README.md) |
| `editor/dialogs/export/` | [Export dialog](src/editor/dialogs/export/README.md) |
| `editor/dialogs/models/` | [Model manager](src/editor/dialogs/models/README.md) |
| `editor/dialogs/preferences/` | [AI preferences dialog](src/editor/dialogs/preferences/README.md) |
| `editor/frame/` | [Editor frame](src/editor/frame/README.md) |
| `editor/frame/input/` | [Keybindings](src/editor/frame/input/README.md) |
| `editor/frame/route/` | [Frame route](src/editor/frame/route/README.md) |
| `editor/panels/` | [Editor panels](src/editor/panels/README.md) |
| `editor/selection.rs` | [Selection](src/editor/selection/README.md) |
| `editor/panels/player/` | [Editor player](src/editor/panels/player/README.md) |
| `editor/panels/properties/` | [Properties](src/editor/panels/properties/README.md) |
| `editor/panels/properties/font_picker/` | [Font picker](src/editor/panels/properties/font_picker/README.md) |
| `editor/panels/properties/mask_picker/` | [Mask picker](src/editor/panels/properties/mask_picker/README.md) |
| `editor/panels/properties/view/` | [Properties view](src/editor/panels/properties/view/README.md) |
| `editor/panels/properties/view/effects/` | [Colour wheels and curves](src/editor/panels/properties/view/effects/README.md) |
| `editor/panels/properties/workflow/` | [Properties workflow](src/editor/panels/properties/workflow/README.md) |
| `editor/timeline/` | [Timeline](src/editor/timeline/README.md) |
| `editor/timeline/view/` | [Timeline view](src/editor/timeline/view/README.md) |
| `editor/timeline/view/ruler/` | [Timeline ruler](src/editor/timeline/view/ruler/README.md) |
| `editor/timeline/workflow/` | [Timeline gestures](src/editor/timeline/workflow/README.md) |
| `editor/timeline/workflow/viewport/` | [Timeline viewport](src/editor/timeline/workflow/viewport/README.md) |
| `home.rs` / `home/` | [Projects page](src/home/README.md) |
| `home/view/` | [Home view](src/home/view/README.md) |
| `i18n.rs` / `i18n/` | [Localization](src/i18n/README.md) |
| `input.rs` / `input/` | [Key input](src/input/README.md) |
| `kit.rs` / `kit/` | [Kit](src/kit/README.md) |
| `kit/widgets/` | [Kit widgets](src/kit/widgets/README.md) |
| `preferences.rs` / `preferences/` | [Preferences](src/preferences/README.md) |
| `services.rs` / `services/` | [Services](src/services/README.md) |
| `services/color_picker/` | [Colour picker](src/services/color_picker/README.md) |
| `services/gpu/` | [GPU adapters](src/services/gpu/README.md) |
| `services/lifecycle.rs` | [Project lifecycle](src/services/lifecycle/README.md) |
| `services/remote_models/` | [Remote model selection](src/services/remote_models/README.md) |
| `shorts.rs` / `shorts/` | [Long video to shorts](src/shorts/README.md) |
| `shorts/view/` | [Shorts view](src/shorts/view/README.md) |

## Public surface

`lib.rs` exports only what `main.rs`, `tests/` and `examples/` use; everything else stays
crate-private. A new export needs a named consumer here.

| Export | Consumer |
|---|---|
| `app` (`Workspace`, `EditorWindow`, `poll_workspace`, `background`) | `main.rs`; `tests/it` lifecycle, control, export, import, projects, shorts, models and canvas rows; the audit scenes |
| `actions` | keybinding, shortcut and timeline tests (`Platform`, registry reads) |
| `home` (`WINDOW`, `ProjectsSortKey`, `ProjectsView`) | `main.rs` for the first window size; the projects tests |
| `i18n`, `preferences` | `main.rs`, tests and the audit |
| `input` | none directly: `actions` re-exports its types. Kept `pub` because a private `input` would turn its `pub use` lines into unused-import warnings that only a build can confirm |
| `services` (`playback`, `lifecycle`, `gpu`, ...) | `main.rs` (audio port, capture, GPU effects); the lifecycle, effects, preview, crop and shorts tests |
| `shorts` (`show`, `Context`, `Action`, `State`) | the shorts tests and the audit scene |
| `Look`, `window_color`, `egui`, `Error`, `Result` | `main.rs`, tests and the audit |
| `Editor` and its re-exported state, inspection and selection types (`AssetsTab`, `Inspection`, `Sounds`, `Music`, `Subtitles`, `PreviewZoom`, `SETTLE_BUDGET`, ...) | the editor tests, which drive and inspect it without a window |
| `export`, `font_picker`, `model_manager`, `ColorTarget`, `ElementKind`, `PropertiesTab`, `shared_fields` | the export, font, model and properties tests |

## Checks

The offscreen audit is an example tool. `examples/ui_audit.rs` is the stable CLI
and composition root holding the two test-fixture `#[path]` seams and the example tests;
its private implementation lives under `examples/audit/`.
`application.rs` owns the harness, app and theme, `fixture.rs` the shared
deterministic media and session fixtures, `scenes.rs` the scene registry with
`scenes/{home,editor,properties,timeline,shorts,assets}.rs`, `readme.rs` the
README capture/import/edit/publication workflow with
`readme/{source,process,media,publication,project}.rs`, and `report.rs` reference
copy plus gallery HTML. `Cargo.toml` declares the `ui_audit` and
`preview_fixture` example targets explicitly with `autoexamples = false`.

`cargo test -p vibecut` asserts widget and engine state and writes no image. `cargo
xtask ui-audit [SCENE...]` renders named scenes offscreen through the automation IDs
at 1x and 2x into `$CARGO_TARGET_DIR/ui-audit/` (or `target/ui-audit/`) with an
`index.html` beside each local reference counterpart; it waits for every
async effect preview to settle, opens no native window and never runs in
`cargo xtask ci`. Device performance is recorded from observed native runs only.

README media is regenerated by the same offscreen runner, not a second app or
mockup:

```sh
cargo xtask build -- cargo xtask ui-audit --readme hero --source PATH
cargo xtask build -- cargo xtask ui-audit --readme timeline --source PATH
```

`PATH` is the explicit local Big Buck Bunny 12-second excerpt; the mode refuses an
unreadable source and has no synthetic fallback. It imports and edits the file
through the real `Editor`/`Session`/`Files` path, writes the fixed files under
`docs/assets/readme/`, and retains raw frames, probe data, film/source hashes and a
per-run manifest under the target's `readme-media` subtree. Provenance, credits,
decoded-output measurements and remaining parent limits live in
[docs/assets/readme/README.md](../../docs/assets/readme/README.md); the film
licence is [CC BY 3.0](../../docs/licenses/CC-BY-3.0.txt).

`--observe SECONDS OUTPUT_DIR [PROJECT]` records at most 3,601 one-second samples and
one window screenshot, then closes, using the production device, jobs, renderer and
frontend. `--headless-smoke NEW_DIRECTORY` starts the real workspace, creates an
editor and draws three egui frames without a window, graphics device or audio
output, then joins jobs and exits; it refuses an existing directory.

## Platforms and evidence

The runtime PNG supplies the window and Dock icon; Windows resources embed the
committed ICO through embed-resource. [Native distribution](../../packaging/README.md)
owns both installers, source receipts, signing, replacement and runtime verification.
The shared FFmpeg dependency is LGPL ([distribution rules](../../docs/engineering/open-source.md#ffmpeg-distribution)).
