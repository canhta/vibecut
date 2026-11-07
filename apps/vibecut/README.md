# vibecut

Native eframe/egui desktop frontend, using wgpu to present render's shared CPU
compositor. Owns device IO, the audio transport clock, retained preview frames,
presentation state, stable automation IDs, screenshots, catalog and preferences.
Engine owns all project edits, job scheduling and aggregate byte accounting;
render owns every pixel, audio conversion, sample grid and channel mix rule.

## Run

Provision FFmpeg with `cargo xtask ffmpeg`. Build with that installation's
`FFMPEG_DIR`, `PKG_CONFIG_PATH` and native library path, as used by
[workspace verification](../../docs/engineering/verification.md).
The desktop binary is `vibecut`; the headless CLI is `vibecut-cli`.

| Module | Responsibility | Contract |
|---|---|---|
| `actions.rs` | Owns the action registry, chords and global dispatch. | [Action registry](src/actions/README.md) |
| `ai_task.rs` | Coalesces AI progress and per-upload consent. | [AI tasks](src/ai_task/README.md) |
| `capture.rs` | Captures bounded fresh frames. | [Capture](src/capture/README.md) |
| `catalog.rs` | Owns the text catalog and the preference store. | [Localization](src/catalog/README.md) |
| `color_picker.rs` / `color_picker/` | Owns the reusable colour draft and screen sampler. | [Colour picker](src/color_picker/README.md) |
| `control.rs` | Runs background control over the live workspace. | [Control](src/control/README.md) |
| `crop.rs` | Owns transient source-normalized crop interaction. | [Crop](src/crop/README.md) |
| `editor.rs` / `editor/` | Coordinates the editor through area interfaces. | [Editor presentation](src/editor/README.md) |
| `editor/assets/` | Owns assets presentation and workflows. | [Assets](src/editor/assets/README.md) |
| `editor/assets/sounds/` | Owns the Sounds tab's library, search and preview. | [Sounds tab](src/editor/assets/sounds/README.md) |
| `editor/assets/view/` | Renders the assets surface. | [Assets view](src/editor/assets/view/README.md) |
| `editor/assets/workflow/` | Applies asset choices. | [Assets workflow](src/editor/assets/workflow/README.md) |
| `editor/captions/` | Owns captions presentation. | [Captions panel](src/editor/captions/README.md) |
| `editor/export/` | Owns export presentation. | [Export dialog](src/editor/export/README.md) |
| `editor/frame/` | Composes the editor window and routes input. | [Editor frame](src/editor/frame/README.md) |
| `editor/frame/input/` | Holds chords, bindings and action dispatch. | [Keybindings](src/editor/frame/input/README.md) |
| `editor/frame/route/` | Routes area actions to their owners. | [Frame route](src/editor/frame/route/README.md) |
| `editor/models/` | Presents the local model manager. | [Editor models](src/editor/models/README.md) |
| `editor/player/` | Retains preview-only player state. | [Editor player](src/editor/player/README.md) |
| `editor/properties/` | Owns the properties area. | [Properties](src/editor/properties/README.md) |
| `editor/properties/font_picker/` | Presents the font catalog picker. | [Font picker](src/editor/properties/font_picker/README.md) |
| `editor/properties/view/` | Renders property rows. | [Properties view](src/editor/properties/view/README.md) |
| `editor/properties/workflow/` | Applies property edits. | [Properties workflow](src/editor/properties/workflow/README.md) |
| `editor/timeline/` | Owns timeline presentation and workflows. | [Timeline](src/editor/timeline/README.md) |
| `editor/timeline/view/` | Renders the timeline surface. | [Timeline view](src/editor/timeline/view/README.md) |
| `editor/timeline/view/ruler/` | Draws the timecode ruler. | [Timeline ruler](src/editor/timeline/view/ruler/README.md) |
| `editor/timeline/workflow/` | Applies timeline gestures. | [Timeline gestures](src/editor/timeline/workflow/README.md) |
| `editor/timeline/workflow/viewport/` | Maintains the timeline viewport. | [Timeline viewport](src/editor/timeline/workflow/viewport/README.md) |
| `effects.rs` | Implements the desktop GPU effect adapter. | [GPU effects](src/effects/README.md) |
| `effect_previews.rs` | Schedules effect tile previews. | [Effect previews](src/effect_previews/README.md) |
| `export.rs` / `export/` | Presents export and its saved choices. | [Export settings](src/export/README.md) |
| `home.rs` / `home/` | Owns the project library window. | [Projects page](src/home/README.md) |
| `home/view/` | Draws the Home window. | [Home view](src/home/view/README.md) |
| `import.rs` | Runs background media import. | [Media import](src/import/README.md) |
| `kit.rs` / `kit/` | Supplies reusable visual primitives. | [Kit](src/kit/README.md) |
| `kit/widgets/` | Renders shared controls. | [Kit widgets](src/kit/widgets/README.md) |
| `lifecycle.rs` | Adapts desktop sessions to engine persistence. | [Project lifecycle](src/lifecycle/README.md) |
| `media_panel.rs` | Presents the media library view and sorting. | [Media library](src/media_panel/README.md) |
| `model_manager.rs` | Presents model catalog, consent and transfers. | [Local model manager](src/model_manager/README.md) |
| `panels.rs` | Registers assets and properties panels. | [Panels](src/panels/README.md) |
| `playback.rs` | Retains frames and the audio device clock. | [Playback and limits](src/playback/README.md) |
| `preview.rs` | Presents preview and snapshot requests. | [Preview transport](src/preview/README.md) |
| `selection.rs` | Holds transient selection scopes and shared values. | [Selection](src/selection/README.md) |
| `shorts.rs` / `shorts/` | Owns the long-video-to-shorts window. | [Long video to shorts](src/shorts/README.md) |
| `shorts/view/` | Draws the shorts window. | [Shorts view](src/shorts/view/README.md) |
| `snapping.rs` | Ranks timeline snap targets. | [Snapping](src/snapping/README.md) |
| `subtitles.rs` / `subtitles/` | Presents the captions panel. | [Subtitles](src/subtitles/README.md) |
| `workspace.rs` | Holds nonblocking lifecycle and recovery presentation. | [Windows and budgets](src/workspace/README.md) |

## Platforms and evidence

The runtime PNG supplies the window/Dock icon. Windows build resources embed the
committed ICO using embed-resource. `packaging/macos-app.sh` stages a macOS installer with ICNS, plist wiring and
bundled FFmpeg; [native distribution](../../packaging/README.md) owns both OS
installers, source receipts, signing, replacement and runtime verification. Native
layered appearance and clean-host release acceptance require separate evidence.
The shared FFmpeg dependency is LGPL; see the [distribution and replacement
rules](../../docs/engineering/open-source.md#ffmpeg-distribution).

`cargo test -p vibecut` covers stable ID accessibility/uniqueness, real
split/trim/move/delete/undo/redo through automation IDs, locale retention,
transport/seek/clear, byte refusal and worker shutdown. It asserts widget and
engine state only; no test writes an image. The audit example's regression test
also runs with workspace tests and checks that its preview matches its edited
timeline without rendering or saving a screenshot. `cargo xtask ui-audit [SCENE...]`
includes `text-item` and `text-item-vi` for the title inspector with a presented
synthetic media frame. The audit
renders the named Home, editor, import, export, close-guard, track, scene and placement scenes offscreen through the same automation IDs at
1x and 2x into `$CARGO_TARGET_DIR/ui-audit/` (or `target/ui-audit/`) with an
`index.html` placing each render beside its `.spikes/ui-reference/` counterpart;
it opens no native window and never runs in `cargo xtask ci`. `cargo xtask ci` is
the handoff check. The local ticket report records observed
native screenshots, ten-minute A/V frame lag, underruns and peak RSS separately
from source-documented API contracts; tests alone establish no device performance.
`--observe SECONDS OUTPUT_DIR [PROJECT]` records at most 3,601 one-second samples
and requests one fresh window screenshot, then closes. It exercises the production device,
engine jobs, renderer and frontend; it never sends media remotely.

## Distribution startup

The distribution-only `--headless-smoke NEW_DIRECTORY` starts the real workspace,
creates an editor and draws three egui frames without a window, graphics device or
audio output, then joins jobs and exits. It refuses an existing directory so the
check cannot consume a user's recovery data. [Packaging](../../packaging/README.md)
owns its clean-runner use and the separate native-device verification limits.
