# vibecut

Native eframe/egui desktop frontend, using wgpu to present render's shared CPU
compositor. Owns device IO, the audio transport clock, retained preview frames,
presentation state, stable automation IDs, screenshots, catalog and preferences.
Engine owns all project edits, job scheduling and aggregate byte accounting;
render owns every pixel, audio conversion, sample grid and channel mix rule.

## Localization

One app-owned catalog in `src/catalog.rs` owns every user-visible string. The
`Text` enum is the stable key set; both English and Vietnamese rows live beside
each key, so an omitted row is a compile error. `catalog::text` returns a row,
`format` fills named placeholders, `format_count` selects the locale's plural row
and groups the number, and `number`/`decimal`/`date` apply the locale's separators
and date order; project dates are UTC display only. Timecode remains raw ticks at
the display boundary and is never localized.

`Locale::from_system` reads the OS preference (`sys-locale` 0.3.2, MIT OR
Apache-2.0); a missing preference file starts from that locale, while
`Preferences::default` stays English for deterministic tests. The header language
control switches the catalog at runtime and the existing bounded preference store
persists the choice. P1/75 owns the full preferences dialog.

Catalog format decision: Project Fluent (`fluent-bundle` 0.16.0, Apache-2.0 OR
MIT; `fluent-templates` 0.15.1, MIT OR Apache-2.0) has strong plural/gender
syntax and compile-time FTL loading, but its message-id lookup and formatter stack
are heavier than this two-locale API and still do not type-check keys at call
sites. `rust-i18n` 4.2.4 (MIT) generates `t!` lookups from YAML/TOML and is simple,
but missing keys fall back at runtime and keys are strings no compiler checks. The
enum table keeps every call site typed, gives complete en/vi coverage for free and
uses no added runtime formatter for two CLDR-simple locales; `cargo xtask i18n`
adds the checks the type system cannot see (empty/duplicate rows, unused keys and
direct string literals passed to egui widgets). `skrifa` (dev-only, MIT OR
Apache-2.0) asserts that the bundled Inter font has a glyph for every Vietnamese
character in the catalog; the offscreen diacritics screenshot is audit evidence,
not a test.

## Run

Provision FFmpeg with `cargo xtask ffmpeg`. Build with that installation's
`FFMPEG_DIR`, `PKG_CONFIG_PATH` and native library path, as used by
[workspace verification](../../docs/engineering/verification.md).
The desktop binary is `vibecut`; the headless CLI is `vibecut-cli`.
`cargo run -p vibecut -- /native/path/to/project.json` acquires the project writer
lease before loading. With no file it opens the minimal Home new/open entry points.
Managed project listing is supplied by [engine library](../../crates/engine/README.md#managed-project-library); the projects-page feature owns its presentation.
Engine lifecycle hooks backfill and refresh project-owned thumbnails, including
close without saving editable content. Preview,
timeline basics, transport and the export dialog are present; text/audio controls
are integrated by their owning feature tickets.

Media import uses one native multi-file picker or a file drop. One batch is capped
at 128 paths (32 KiB each), must admit a 448 MiB probe/work lease from the shared
byte budget, and publishes results through a bounded 8-event channel, so a paused
UI applies backpressure instead of accumulating metadata. Probing and hashing run
on an engine import job with one cooperative cancellation token carried into
media's FFmpeg probe interrupt and 64 KiB hash reads; each successful file becomes
an asset immediately, so a later refusal or cancel keeps earlier assets. Published
imports refresh playback against the updated snapshot, including ratcheted FPS. The media
panel omits media-owned ephemeral assets from rows and selection while retaining
them in the project for internal commands. The panel shows the probe type/duration/resolution/audio format, multi-select assets,
and an "Add to timeline" action that inserts only the selected assets in one undo
step through engine auto placement. The engine chooses compatible tracks and
creates secondary tracks on demand; the app owns no placement policy. While a batch runs, an overlay shows completed/total
progress and a 2 px accent bar, disables import/insert, and offers Cancel. Partial
failures remain listed with their typed media reason and close with a toast counting
imported, failed and unfinished files; a terminal runner failure stays a typed
error and never reports success. Insert synchronizes playback against the new
immutable project snapshot. Image placement, full library views/sort and asset
deletion remain with their later feature tickets.

A pending import row is emitted before probe/hash execution and remains transient
until validated metadata is added through the engine command. Audio and video
follow the same probing -> ready/unsupported presentation. Restored assets
(including undo/redo) default to probing and reopen capabilities in a background
import job before timeline insertion; persisted thumbnails are requested through
the same content cache. Missing, changed or refused sources remain blocked.

Queue and editor retain at most 64 shared thumbnail handles each, charged by
engine `RetainedThumbnail` for CPU/upload/GPU storage. Undo prunes both owners;
entry limits and aggregate budget pressure evict images and textures together.
Import retries work admission after dropping retained thumbnails on pressure.
The batch work allowance is 448 MiB, covering bounded native decoding, codec
scratch, packet/probe buffers, hashing and handoff; retained images have separate
leases. Media owns the stricter thumbnail/recovery geometry and worker bounds.

The timeline draws tracks, filmstrip/waveform placeholders, bookmarks and the
playhead; an adaptive ruler picks frame/second major and minor ticks from zoom
and project fps, labels them with time's shared DF/NDF timecode, and never
overlaps labels. The exponential zoom slider, ×1.7 buttons and Cmd/Ctrl+wheel
share a dynamic fit minimum (content occupies 25% of the viewport), a 100×
maximum and 75%→15% trailing padding; zooms above the 15% slider threshold hold
the playhead's viewport offset and crossing back below restores the pre-anchor
scroll. Wheel scrolls vertically, Shift/horizontal wheel scrolls horizontally,
and both clamp to content plus padding. Dragging near an edge auto-scrolls in a
100 px zone at up to 15 px per frame and stops at the content boundary or when
the pointer leaves. Movement compensates only the clamped viewport shift;
horizontal wheel follows egui content direction.
`cargo xtask ui-audit timeline-zoom-low timeline-zoom-high` captures both zoom
levels in dark/light at 1x/2x outside CI. Zoom, horizontal scroll, frame-snapped
ruler seeking,
single/Cmd-or-Shift multi-selection, drag move, edge trims, split, delete and
ripple delete all dispatch engine commands. Delete/Backspace removes, Shift+Delete
ripple-deletes, Cmd/Ctrl+B splits at the playhead and N toggles snapping; Shift
bypasses snapping during a drag. Cmd/Ctrl+Z undoes and Shift+Cmd/Ctrl+Z or Cmd+Y
redoes. Space toggles playback; K pauses; J/L seek one second backwards/forwards;
arrows step one frame; Shift+arrows jump one second; Home/End seek the timeline
ends. Scrubbing uses the same seek path. Two resizable columns and a horizontal
divider start at the UI design's 25/50/25 and 50/50 proportions. Drags clamp
assets/properties to 15–40%, preview to 30–85% and the timeline to 15–70%, so no
region reaches zero. The assets panel has the reference's nine-tab vertical icon
bar (Media, Sounds, Text, Stickers, Effects, Transitions, Captions, Adjustment,
Settings) with tooltips and top/bottom scroll fades; Media owns the live library,
Transitions and Adjustment are thin stubs until their feature tickets, and the
remaining tabs show the same catalog stub until their owners land. A single
selection shows that element type's properties tabs, and the last tab is
remembered per type. Text, video with/without source audio, image, sticker,
graphic, audio and effect have the reference tab sets in `src/panels.rs`; image
currently means a source video stream reporting one frame, and sticker/effect
items await standalone item contracts before those two sets are reachable from a
real project. The effect-stack contract from p1-parity/33 is attached to existing
visual items, whose Effects tabs are already available; p1-parity/97 owns their
stack controls and the Effects asset view. Captions remains a separate asset-view
mount point for the subtitle editor. English/Vietnamese, theme, panel proportions,
property-section open state, the active assets tab and per-type properties tabs
use one bounded, versioned preference store;
`ui.prefs.set` edits have app-owned undo/redo and never touch Project JSON.
Tab navigation is presentation state: it persists through the same validated
store but stays out of app undo history so it cannot shadow a project edit. A
damaged layout restores its defaults with a visible warning, while an unreadable
document restores the system defaults. Unversioned and unsupported versions are
refused without rewriting their bytes on load or save. Edits and writes share the 4 KiB
read budget, so a saved document always remains readable.

The selection inspector shows audio gain/mute and text size/colour for compatible
selected items, with a dash for mixed values (an indeterminate mute checkbox) and
the compatible/selected count.
A single element uses the per-type tabs above instead: Text has a size scrub and
colour entry, Blending a blend menu and opacity scrub, and Audio a gain scrub and mute checkbox.
Each scrub field has a left drag handle, a click-to-type entry and clamps; the
drag previews through an engine gesture, commits as one registered
`timeline.set_item_param` undo step on release, and Escape cancels the gesture or
the typed draft. Changing selection or hiding the scrub field cancels its
preview before another edit can join the gesture. Invalid or non-finite input refuses without changing the item.
Drag sensitivity is our recorded decision because the inventory leaves it
unverified: opacity and audio gain move one 0.01/1 dB step per 4 px, and text size
one px per 2 px. Enter or blur commits a typed shared-field patch through the
engine registry in one undo step. Text editing owns keyboard input while focused.
Font family and video volume await their owning contracts before joining the same
inspector.

An empty-track drag selects intersecting visible items; Cmd/Ctrl/Shift adds to
the selection at gesture entry. Reversing the drag recomputes intersections, and
Escape cancels an active marquee. Otherwise Escape clears mask points, then
keyframes, then items. `selection_scopes` is transient app state; `inspect` reports
the most specific active scope. Timeline Shift clicks toggle; ordered media-list
Shift clicks extend from its anchor. `timeline.deselect_all` clears every scope.
Item selection is synchronized to Session history before edits and restored on
undo/redo; it never enters Project JSON.

## Injection and inspection

`Workspace::import_picked_files` applies native picker result paths through the
same import queue as drops; kittests inject at this boundary without opening a
dialog, then use real library/insert widgets. Import shutdown keeps its probe/work
allowance reserved until the cancelled worker has joined.

`Editor::ui` runs without a window/device. `Playback::new` injects `Sources`, which
creates render's `SourceProvider`; production uses `FileSources` exactly as export
does. `AudioPort::fill` accepts a headless device callback for transport tests.
`Editor::activate` maps stable widget IDs to engine `execute` for every project
edit and to app-owned transport/preferences/selection. `inspect` exposes
enabled/focus/rect/current view, playing, position, selection, zoom and snapping
state. The project remains the Session's immutable snapshot. Each interactive
control, including every timeline item and trim handle, exposes the same stable
`author_id` through AccessKit, independent of locale and layout. Headless tests
query those IDs and click or drag through the real widget/engine path.

`Capture::request` queues a fresh GPU readback and `poll` waits without blocking
for at most five seconds; window and panel rectangles are supported. Unavailable
or timed-out integrations return typed failures. Captures demonstrate presentation,
never command success; inspect engine state separately. Fully occluded/minimized
windows may time out rather than return an old framebuffer.

## Playback and limits

Two engine Preview jobs own decode-ahead and audio mixing. They retain no second
scheduler. Audio's fixed atomic stereo ring is engine-budgeted; the callback only
copies samples, zero-fills underruns, advances monotonic device sample counters and
reports callback-to-playback latency. Video follows the latency-corrected audio
clock and skips past frames under load. An absent, failed or one-second-stalled
audio device refuses/pauses playback with an unavailable state.

Device negotiation requires stereo f32 at the project's render-owned sample rate
(default 48 kHz); unsupported devices report unavailable. The app never resamples,
remaps channels or substitutes a wall clock. Gaps/underruns play silence and retain
the device timeline; they are counted. Pausing and seeking invalidate old ring
samples by generation. Shutdown cancels and joins the engine workers.

Preview fits within 640×360, preserving aspect ratio. At most four queued frames,
one shown frame and the upload's retained frame survive at once. Render leases its
working buffers; app leases cover its fixed ring, CPU ColorImage/upload/texture
retention. All leases share the one injected engine budget. The native default is
768 MiB, including render's conservative concurrent probe/decode allowances; this
is an admission ceiling, not resident usage. Pressure or a frame larger than the
available budget returns a typed error before allocation and shows unavailable.
Healthy cache clear preserves position/play state and invalidates queued output.
A terminated preview job pauses transport at the current device-clock position,
fences old output and cancels its companion. Retry keeps playback paused while
engine polling joins finished jobs, then resubmits both jobs with the same source
factory, audio port and aggregate budget. It never blocks the UI waiting for an
adapter; a noncooperating adapter delays replacement and normal shutdown until it
returns. Playback can resume explicitly after the jobs are available. Old work
cannot publish into the new generation, and partial resubmission failure cancels
any admitted replacement. OS codec/driver allocations and
allocator overhead require separate RSS measurement.

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

## Project lifecycle

`Workspace` owns loading/exiting/error/recovery presentation and dispatches blocking
open, discovery, save, flush and discard work through engine's job runner. The
persistent `EditorSession` wraps the sole engine `Autosave`; the memory variant
supports the existing headless editor injection interface. Home creates “New
project”; a genuinely missing route path creates “Untitled Project” once. Other
load failures retain typed errors and Retry/Back controls. New/Open/Save As use
rfd's asynchronous native picker so the event loop keeps autosave ticking; paths stay native through the writer lease.

Header rename commits on Enter and cancels on Esc through `project.rename`.
The timeline restores its scale multiplier and logical-pixel horizontal scroll
through `project.set_view_state`, outside edit history; seeking and navigation save
the current view, deferring metadata writes until an active edit gesture finishes.
Playhead is restored through the existing seek path. Save/exit wait off-thread for engine acknowledgement; an error retains the
active editor. A window close is intercepted with `CancelClose`: save and close,
keep a flushed recovery copy and close, or cancel. New/recovered work requires
Save As. Loading can be cancelled without accepting its eventual result.

Startup discovers only abandoned journals and shows corrupt-candidate diagnostics.
Recovery publishes a new never-saved session journal before discarding the old
candidate; the original project is untouched. Other candidates remain deferred
while that recovered editor is active, and reappear after Save As/leave. Recovery
cannot replace an active editor outside its normal save/leave boundary. rfd 0.17.2 is MIT and is the one
native picker dependency. `tests/it/lifecycle.rs` exercises lifecycle boundaries and
real rename/close/recovery widgets without writing images.
Native picker interaction and Windows runtime need separate observed evidence.

## GPU effects

`effects::GpuEffects` implements render's device-free effect interface using the
eframe-owned wgpu device/queue. Native startup attaches it to each preview player;
headless players default to CPU. Shader entry names and ordered separable passes
are data; adding shader entries does not change scene composition. Bindings expose
the item, optional coverage and track-below working images. Blur performs no
colour/alpha conversion. Compute buffers, upload, pass metadata and readback are
charged to the engine hooks; native driver/pipeline allocations remain outside
that accounting. Device limit/validation/readback failure returns Unsupported for
CPU fallback. Submitted work retains its byte lease until GPU completion, even
if the bounded readback wait times out. No unsafe mapping or buffer casts are used.
`tests/it/effects.rs` compares f32 CPU/GPU output with max absolute tolerance 2e-5.

## Effects panel

The assets Effects tab lists `project::effect_definitions()` as draggable tiles;
each tile shows a live preview scheduled through the engine `EffectThumbnails`
job/cache with the CPU reference compositor headlessly; native startup attaches
the playback GPU adapter and shares playback's byte budget. Pending, failed and ready
states stay visible, and a failed tile offers Retry. A tile can be dragged onto a
video, image, sticker or graphic clip, or added with `+` to the selected clip.

The properties Effects tab lists the selected clip's ordered stack. Each header
has a drag-to-reorder handle, enable/disable toggle and remove action; registered
scalar parameters use the same scrub/typed field contract as other inspector
fields. Reorder gestures move in either direction and cancel when the selection
or inspector changes. Empty stacks, disabled effects, locked tracks, audio clips, unsupported
kinds, missed drops and the project's 64-effect limit render an explicit state or
localized refusal reason.

Every edit goes through `effects.add`, `effects.reorder`, `effects.toggle`,
`effects.remove` or `effects.update_param`; the UI never mutates a project copy.
`tests/it/effects_ui.rs` covers drag/drop, reorder, toggle, remove, parameter
gesture reversal/cancel/clamping, refusal boundaries and preview job states.

## Track controls

Timeline header and row geometry follows the [UI design](../../docs/product/ui.md#density),
using embedded regular SVG kind icons, output toggles and accessibility-only
names. Logical row sizes stay constant across DPI scaling; vectors are cached at
the display density. Empty rows have dashed placeholders. Header context
menus remove unlocked secondary tracks; main has a persisted magnet control in
its header and the timeline toolbar. All edits use registered engine commands and
refresh the shared preview snapshot. Paste is supplied by the clipboard owner.
Media insertion reuses an unlocked compatible track only when the playhead range
fits; otherwise it creates a secondary track at the top of its group. Solid colour
is generated Video content, distinct from Sticker tracks.

Header context menus also rename tracks inline (Enter commits, Escape cancels)
and toggle the engine's lock barrier. Lock protects item structure and properties,
masks, effects and keyframes; selection, playback and track metadata stay usable.
Dragging a header reorders within its overlay or audio section; main stays fixed.
Escape cancels the reorder. The explicit Add track toolbar action uses the active
item's compatible kind above its track, or Video at the overlay section end when
there is no active item. Commands and undo/persistence belong to
[engine](../../crates/engine/README.md#edit-semantics-and-limits).

## Scene switcher

The timeline toolbar centres a split button with the current scene name and a
scene-list control. The sheet lists all scenes and switches through
`scene.switch`; Select mode permits multi-delete while disabling main. Delete
opens a confirmation dialog explaining that the action can be undone. Closing or
Escape dismisses the sheet. Per-scene zoom/scroll/playhead restore through the
Session metadata view; item/keyframe/mask-point selection clears on switching.
The sheet's plus action names the scene `Scene N`, runs `scene.create` as one
undoable edit and then switches to it; the sheet stays open. A row's context menu
opens an inline rename editor for any scene, including main: Enter commits through
`scene.rename` and Escape cancels. New scenes inherit the project canvas, frame
rate and background and start with an empty timeline. English/Vietnamese copy uses
the existing catalog.
A delete confirmation takes Escape before the rename editor. If a command or undo
removes the edited scene, its pending name edit is discarded.

`tests/it/scenes_ui.rs` operates the real widgets and engine at 1x/2x density, checks
switching, protected main, create/rename history, persistence and atomic
multi-delete/undo without writing images.
`cargo xtask ui-audit scene-create scene-rename` captures the created scene and
inline editor separately in both themes at 1x/2x.

## Export dialog

The header Export action opens a modal with format and quality disclosures plus an
inline Include audio switch that is on by default. Choices use the existing
English/Vietnamese catalog and app preference store: MP4 by default, MOV or WebM;
low/medium/high/very_high quality (default high); audio included by default. The
presets request quality 55/70/85/95 and 3/6/12/24 Mb/s respectively through media's
existing encoder contract. These are explicit encoder requests, not measured
perceptual guarantees. Canvas geometry and exact frame rate follow the project. The
dialog always encodes so a selected quality is applied; the core retains its
separate Auto/copy modes.

`export.run` writes only to a unique temporary output. On completion the app opens
the asynchronous native save dialog with the sanitized `<ProjectName>.<ext>` and
the last save directory, then submits background publication, shows a transient toast
with the chosen path and closes the popover. Cancelling that save dialog writes
nothing and keeps the finished job ready for a later save without re-exporting;
closing the popover discards it. `Editor::export_picked_file`,
`Editor::export_save_cancelled` and the `Workspace` picker inject native results
without driving an OS dialog. Filenames keep Unicode and spaces, replace invalid
filesystem characters, trim trailing dots/spaces and escape Windows reserved
device names. `Editor::export_save_defaults` supplies the dialog name/directory.
The format, quality, audio switch and last directory persist; source consent and
overwrite authorization stay transient. An empty timeline shows a catalog error
and refuses start.
Saving submits engine's completed-export publication job with the rendered
snapshot and Autosave's protected project/metadata/recovery paths. All admission,
copy and sync run on the runner through project's atomic stage. Progress and
cancellation remain in the export dialog; cancellation before commit and failed
staging/publication retain the finished render and previous destination. Failures
use the export diagnostics panel with text-only Copy and Retry. Publication Retry
opens destination selection again without re-encoding. Only confirmed publication
persists the directory, toasts the path and closes the modal. Shutdown joins the
runner before cleaning up owned temporary files.
The save filename and container filter follow the rendered attempt, including
when current format preferences changed while the failure panel was visible.

`export.run` / `export.retry` submit through engine's one export job path and share
playback's byte budget. Inspection exposes the real job snapshot, phase, typed
failure, missing/changed source asset/item/range diagnostics, consent, finished
temp and attempt settings. Running output disables settings and shows engine
stage/progress plus Cancel; cancel or close clears the temporary output and returns
to settings with no final file. A refusal or failed attempt replaces progress with
an inline typed error panel carrying Copy (diagnostics text only) and Retry. Retry
reruns the same attempt settings into a new temp output, so a prior successful file
is untouched. Retry refuses running, cancelled and successful attempts; a degraded
retry requires an explicit toggle over the listed
diagnostics, and a project/audio-scope change clears consent. Errors remain in the
modal for retry. Successful explicitly consented degraded output follows the same
native Save/toast/close path; affected ranges are disclosed before running, with
no post-completion warning or failure screen.

`tests/it/export_ui.rs` asserts real widget and engine state, never pixels.
`cargo xtask ui-audit export-dialog` renders the modal offscreen in dark and
light at 1x and 2x beside the export reference; it opens no window, produces only
audit evidence and is excluded from CI. Production source failures
are injected through the same render source-factory boundary for deterministic
start/cancel/failure tests; no second job state machine is introduced.

## Distribution startup

The distribution-only `--headless-smoke NEW_DIRECTORY` starts the real workspace,
creates an editor and draws three egui frames without a window, graphics device or
audio output, then joins jobs and exits. It refuses an existing directory so the
check cannot consume a user's recovery data. [Packaging](../../packaging/README.md)
owns its clean-runner use and the separate native-device verification limits.

## Blend and opacity inspector

Visual selections show a localized blend dropdown (`inspector.blendMode`) and
0..=1 opacity entry (`inspector.opacity`); options use
`inspector.blendMode.<persisted-id>`. Mixed selections show the existing mixed
value marker. Both controls dispatch the registered shared-field command, giving
one atomic undo step and retaining unrelated fields. Blend labels use the same
two-language catalog; opacity edits set a constant channel. Animated editing
continues through the engine keyframe commands. The desktop adapter implements
the same linear-light blend formulas in WGSL; unsupported device operations
fall back to the render reference with the unchanged backdrop.
`cargo xtask ui-audit inspector-blend inspector-blend-vi` renders these inspector
scenes through the shared audit registry in both themes and at 1x/2x.

## Long video to shorts

The Home `home.shorts` card opens a separate native viewport. `Shorts` owns its
bounded presentation/review history; `LocalVideoWorkflow` composes engine's
`clip_suggest::suggest` against the picked/dropped local video. Provider and model
pickers use offline capability data. Local heuristics are an explicit choice;
remote ASR and suggestions stop at an exact per-upload disclosure (provider,
model, language, audio duration, payload bytes, cost ceiling and policy links).
Refusal or cancellation sends no approval and never selects another provider.
Missing models and credentials are actionable errors; no implicit downloads occur.

Candidates show title, duration, editorial score, reason and heuristic/AI source.
The shared catalog formats metadata and numeric fields; trim values parse the
same en/vi decimal and grouping notation, refusing malformed or mixed input.
Changing the runtime locale refreshes trim drafts without changing source ticks.
Review retains roles and both play/source ranks, safe engine boundary snapping,
accept/reject and a source-normalized 9:16 crop. The reusable `crop` control moves
within source bounds, with keyboard arrows and Escape rollback. Review actions
have a separate 100-step undo/redo history; a whole crop gesture is one step.
`ClipBatch` produces the exact concatenated snapshot and crop placements used by
both preview and export, so playback skips source gaps and follows play order.
Only accepted shorts submit batch jobs, with independent progress/cancel/retry;
editing a review does not alter an already submitted immutable batch.

Open in editor inserts every video segment and mapped source-caption item into a
fresh portrait project through one engine transaction, retaining the prepared
media's original dimensions while replaying items so first-import initialization
preserves the explicit portrait canvas, then uses the existing
nonblocking recovery/autosave owner. Editor preview and export share the same
transient render-state owner, resolving
placements against each immutable snapshot so deleting items and undo/redo do
not leave stale placement IDs or recenter editor exports. Review crops are
transient (preview/export);
persisted crop channels and subsequent full-editor crop edits belong to the crop
controls feature. Source-language captions can be burned and exported as SRT.
`tests/it/shorts_ui.rs` injects generation and preview sources, uses real automation
IDs and engine commands, and asserts state only. UI image evidence is an offscreen
audit, separate from the test suite; no native window is opened by verification.

`cargo xtask ui-audit shorts-pick shorts-setup shorts-review shorts-pick-vi
shorts-setup-vi shorts-review-vi` captures the three Shorts screens in en/vi,
dark/light, at 1x/2x and the minimum 1280×800 logical size. These shared scenes
use injected candidates and synthetic frames by default; `SHORTS_AUDIT_VIDEO`
optionally selects one local audit source, imported once for the run. The audit
is separate from tests and never makes provider calls.

Effect thumbnail consumers use `Arc<GpuEffects>::thumbnails(shared_budget)` to
construct the engine's bounded offscreen preview cache. It schedules the same GPU
adapter without a window; source selection and Effects panel presentation belong
to their UI consumer. Engine's [thumbnail contract](../../crates/engine/README.md#effect-commands)
owns bounds, cadence, cancellation, cache keys and typed failure.

## Local model manager

The header Models action opens one modal over `ai::models::catalog` and engine's
`ModelTransfers`; it opens the same `ai::models::cache_dir` bundle cache as the
CLI and never adds a second downloader, checkpoint or cache. Rows show model id,
language scope (including language-neutral weights), exact licence/source links,
total and per-file size/SHA-256, install state and the current job stage/progress.
Only MIT, Apache-2.0 and BSD weights can carry the Suggested default badge; every
other licence shows its restriction and needs the explicit review checkbox before
Download is enabled. A fresh manager hashes installed bundles on a background
check; dropping the manager cancels and joins that checker before releasing its
cache ownership. Opening, selecting a model, accepting a licence or starting transcription
never starts a transfer. The affirmative Download click is the only place a
`DownloadConsent` is granted; closing while a transfer is active is refused, and
Cancel leaves the engine checkpoint available for Resume.

`tests/it/models_ui.rs` drives the real widgets against injected cache, transport and
`ModelIo` boundaries for complete, cancel, resumed Range/ETag, checksum-failure
and disk-full paths, proves zero HTTP requests before the Download click, checks
the non-permissive opt-in, and writes 1x/2x screenshots under `target/`. The
production composition instantiates the manager lazily on first open; `/09` uses
`Editor::open_model` to open it on a missing model. `Editor::set_model_manager`
injects another cache/transport composition for tests and `cargo xtask ui-audit`.

## Captions panel

The assets-panel Captions tab owns `subtitles` presentation. It reuses engine's
capability catalog, ASR/checkpoint workflow, translation windows and export jobs;
no adapter is constructed by opening a picker and no model downloads implicitly.
Local Whisper is the initial explicit choice. Gipformer appears only for
Vietnamese. OpenAI and ElevenLabs are explicit remote ASR choices; OpenAI and
Anthropic are translation choices. Settings persist in preferences; approval,
transcripts and the session glossary do not.

SRT/VTT imports retain the file's content identity as an internal SourceAsset and
insert the asset and caption item in one undoable batch. Generate targets one
selected media item, clips source cues to its source range and inserts at its
output start. Translation stages a snapshot and applies one content command only
against the original Session revision. A changed provider/language setting cancels
the pending run; denial, cancellation, stale results and failures apply no partial
project changes. `ai_task` coalesces progress and holds one exact upload request;
its modal remains available when switching tabs. Disclosures show provider/model,
languages, payload size, duration, cost and the provider's own HTTPS policy links.
The Models action on a missing bundle opens the existing model manager.

Cue times seek in output coordinates. Shift extends a range; Cmd/Ctrl toggles.
Text edits retain independent language edit/provenance flags and normalize through
captions. Leaving a field or switching tabs commits drafts; unrelated project edits retain
them, and concurrent changes to the same caption retain a copyable draft until
explicit reload. Focused text never
triggers timeline shortcuts. Split uses the chosen playhead time and whole
graphemes; merge requires adjacent cues with matching languages. One-frame nudges
refuse overlaps or item-boundary crossings atomically. All edits use registered
engine commands and the shared history; no subtitle-specific undo stack exists.
Lists page at 50 cues, imports cap at 16 MiB/10,000 cues, text at 64 KiB per field
and glossary entries at 128. Reading-speed warnings follow LanguageProfile data.
The existing bundled common-Chinese font subset is the egui fallback.

`tests/it/subtitles_ui.rs` uses real widgets plus offline adapters for editing,
undo/redo, all vi/en/zh directions and Hans/Hant targets, bilingual SRT/VTT,
consent denial/cancellation/provider changes, stale output and oversized imports.
`cargo xtask ui-audit captions-empty captions-empty-vi captions-editor
captions-editor-vi` captures the real minimum-size panel in both themes at 1x/2x;
these audit scenes open no native window and make no provider requests.
