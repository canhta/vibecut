# vibecut

Native eframe/egui desktop frontend; wgpu presents render's shared CPU compositor. Owns device IO, the audio transport clock, retained preview frames,
presentation state, stable automation IDs, screenshots, catalog and preferences.
Engine owns all project edits, job scheduling and aggregate byte accounting;
render owns every pixel, audio conversion, sample grid and channel mix rule.

## Run

Provision FFmpeg with `cargo xtask ffmpeg`. Build with that installation's
`FFMPEG_DIR`, `PKG_CONFIG_PATH` and native library path, as used by
[workspace verification](../../docs/engineering/verification.md).
The desktop binary is `vibecut`; the headless CLI is `vibecut-cli`.

Each module's contract is its README.

| Module | README |
|---|---|
| `actions.rs` / `actions/` | [Action registry](src/actions/README.md) |
| `ai_task.rs` | [AI tasks](src/ai_task/README.md) |
| `appearance.rs` | — |
| `capture.rs` | [Capture](src/capture/README.md) |
| `color_picker.rs` / `color_picker/` | [Colour picker](src/color_picker/README.md) |
| `control.rs` | [Control](src/control/README.md) |
| `crop.rs` | [Crop](src/crop/README.md) |
| `editor.rs` / `editor/` | [Editor presentation](src/editor/README.md) |
| `editor/assets/` | [Assets](src/editor/assets/README.md) |
| `editor/assets/sounds/` | [Sounds tab](src/editor/assets/sounds/README.md) |
| `editor/assets/view/` | [Assets view](src/editor/assets/view/README.md) |
| `editor/assets/workflow/` | [Assets workflow](src/editor/assets/workflow/README.md) |
| `editor/captions/` | [Captions panel](src/editor/captions/README.md) |
| `editor/export/` | [Export dialog](src/editor/export/README.md) |
| `editor/frame/` | [Editor frame](src/editor/frame/README.md) |
| `editor/frame/input/` | [Keybindings](src/editor/frame/input/README.md) |
| `editor/frame/route/` | [Frame route](src/editor/frame/route/README.md) |
| `editor/models/` | [Editor models](src/editor/models/README.md) |
| `editor/player/` | [Editor player](src/editor/player/README.md) |
| `editor/properties/` | [Properties](src/editor/properties/README.md) |
| `editor/properties/font_picker/` | [Font picker](src/editor/properties/font_picker/README.md) |
| `editor/properties/view/` | [Properties view](src/editor/properties/view/README.md) |
| `editor/properties/workflow/` | [Properties workflow](src/editor/properties/workflow/README.md) |
| `editor/timeline/` | [Timeline](src/editor/timeline/README.md) |
| `editor/timeline/view/` | [Timeline view](src/editor/timeline/view/README.md) |
| `editor/timeline/view/ruler/` | [Timeline ruler](src/editor/timeline/view/ruler/README.md) |
| `editor/timeline/workflow/` | [Timeline gestures](src/editor/timeline/workflow/README.md) |
| `editor/timeline/workflow/viewport/` | [Timeline viewport](src/editor/timeline/workflow/viewport/README.md) |
| `effects.rs` | [GPU effects](src/effects/README.md) |
| `effect_previews.rs` | [Effect previews](src/effect_previews/README.md) |
| `export.rs` / `export/` | [Export settings](src/export/README.md) |
| `home.rs` / `home/` | [Projects page](src/home/README.md) |
| `home/view/` | [Home view](src/home/view/README.md) |
| `i18n.rs` / `i18n/` | [Localization](src/i18n/README.md) |
| `import.rs` | [Media import](src/import/README.md) |
| `input.rs` / `input/` | [Key input](src/input/README.md) |
| `kit.rs` / `kit/` | [Kit](src/kit/README.md) |
| `kit/widgets/` | [Kit widgets](src/kit/widgets/README.md) |
| `lifecycle.rs` | [Project lifecycle](src/lifecycle/README.md) |
| `media_panel.rs` | [Media library](src/media_panel/README.md) |
| `model_manager.rs` | [Local model manager](src/model_manager/README.md) |
| `panels.rs` | [Panels](src/panels/README.md) |
| `playback.rs` | [Playback and limits](src/playback/README.md) |
| `preferences.rs` / `preferences/` | [Preferences](src/preferences/README.md) |
| `preview.rs` | [Preview transport](src/preview/README.md) |
| `selection.rs` | [Selection](src/selection/README.md) |
| `shorts.rs` / `shorts/` | [Long video to shorts](src/shorts/README.md) |
| `shorts/view/` | [Shorts view](src/shorts/view/README.md) |
| `snapping.rs` | [Snapping](src/snapping/README.md) |
| `subtitles.rs` / `subtitles/` | [Subtitles](src/subtitles/README.md) |
| `workspace.rs` | [Windows and budgets](src/workspace/README.md) |

## Platforms and evidence

The runtime PNG supplies the window/Dock icon; Windows resources embed the committed ICO via embed-resource. `packaging/macos-app.sh` stages the macOS installer. [Native distribution](../../packaging/README.md) owns both installers, source receipts, signing, replacement and runtime verification. The shared FFmpeg dependency is LGPL; see the [distribution and replacement rules](../../docs/engineering/open-source.md#ffmpeg-distribution). Native layered appearance and clean-host release acceptance need separate evidence.

`cargo test -p vibecut` asserts widget and engine state only and writes no image. `cargo xtask ui-audit [SCENE...]` renders named scenes offscreen through the automation IDs at 1x and 2x into `$CARGO_TARGET_DIR/ui-audit/` (or `target/ui-audit/`) with an `index.html` beside each `.spikes/ui-reference/` counterpart. It opens no native window and never runs in `cargo xtask ci`, which is the handoff check. Device performance (A/V lag, underruns, peak RSS) is recorded only from observed native runs, never from tests.

`--observe SECONDS OUTPUT_DIR [PROJECT]` records at most 3,601 one-second samples and one fresh window screenshot, then closes. It uses the production device, engine jobs, renderer and frontend, and sends no media remotely.

## Distribution startup

`--headless-smoke NEW_DIRECTORY` starts the real workspace, creates an editor and draws three egui frames without a window, graphics device or audio output, then joins jobs and exits. It refuses an existing directory so it cannot consume recovery data. [Packaging](../../packaging/README.md) owns its clean-runner use and the native-device verification limits.
