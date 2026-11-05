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
Project listing and thumbnail storage belong to the projects-page feature. Preview,
timeline basics, transport and the export dialog are present; text/audio controls
are integrated by their owning feature tickets.

Media import uses one native multi-file picker or a file drop. One batch is capped
at 128 paths (32 KiB each), must admit a 256 MiB probe/work lease from the shared
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

The timeline draws tracks, filmstrip/waveform placeholders, bookmarks and the
playhead; zoom, horizontal/vertical scrolling, frame-snapped ruler seeking,
single/Cmd-or-Shift multi-selection, drag move, edge trims, split, delete and
ripple delete all dispatch engine commands. Delete/Backspace removes, Shift+Delete
ripple-deletes, Cmd/Ctrl+B splits at the playhead and N toggles snapping; Shift
bypasses snapping during a drag. Cmd/Ctrl+Z undoes and Shift+Cmd/Ctrl+Z or Cmd+Y
redoes. Space toggles playback; K pauses; J/L seek one second backwards/forwards;
arrows step one frame; Shift+arrows jump one second; Home/End seek the timeline
ends. Scrubbing uses the same seek path. Two resizable columns and a horizontal
divider start at the UI design's 25/50/25 and 50/50 proportions. Drags clamp
assets/properties to 15–40%, preview to 30–85% and the timeline to 15–70%, so no
region reaches zero. English/Vietnamese, theme, panel proportions and
property-section open state use one bounded, versioned preference store;
`ui.prefs.set` edits have app-owned undo/redo and never touch Project JSON. A
damaged layout restores its defaults with a visible warning, while an unreadable
document restores the system defaults. Unversioned and unsupported versions are
refused without rewriting their bytes on load or save. Edits and writes share the 4 KiB
read budget, so a saved document always remains readable.

The selection inspector shows audio gain and text size/colour for compatible
selected items, with a dash for mixed values and the compatible/selected count.
Enter or blur commits a typed shared-field patch through the engine registry in
one undo step; invalid input refuses without changing any item. Text editing owns
keyboard input while focused. Font family and video volume await their owning
contracts before joining the same inspector.

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

## Scene switcher

The timeline toolbar centres a split button with the current scene name and a
scene-list control. The sheet lists all scenes and switches through
`scene.switch`; Select mode permits multi-delete while disabling main. Delete
opens a confirmation dialog explaining that the action can be undone. Closing or
Escape dismisses the sheet. Per-scene zoom/scroll/playhead restore through the
Session metadata view; item/keyframe/mask-point selection clears on switching.
English/Vietnamese copy uses the existing catalog. Creation and rename commands
are headless here; their frontend controls belong to the scene-name feature.

`tests/it/scenes_ui.rs` operates the real widgets and engine at 1x/2x density and
checks switching, protected main and atomic multi-delete/undo without writing
images.
## Export dialog

The header Export action opens a modal with format, quality and audio disclosures
closed initially. Choices use the existing English/Vietnamese catalog and app
preference store: MP4 by default, MOV or WebM; low/medium/high/very_high quality
(default high); audio included by default. The presets request quality 55/70/85/95
and 3/6/12/24 Mb/s respectively through media's existing encoder contract. These
are explicit encoder requests, not measured perceptual guarantees. Canvas geometry
and exact frame rate follow the project. The dialog always encodes so a selected
quality is applied; the core retains its separate Auto/copy modes.

Choose destination uses the existing nonblocking native save picker;
`Editor::export_picked_file` / `Workspace::export_picked_file` inject its result
without driving an OS dialog. Replacing an existing path needs a separate checked
confirmation. Source/path, overwrite and degraded consent are transient and never
saved to preferences. An empty timeline shows a catalog error and refuses start.

`ExportDialog` submits through engine's one export job path and shares playback's
byte budget. Inspection exposes the real job snapshot, typed submission failure,
missing/changed source asset/item/range diagnostics, consent and widget state.
Running output disables settings and shows engine stage/progress plus Cancel;
Escape requests cancellation and leaves progress visible until engine acknowledges
it. A late cancellation can lose to the existing publication fence and then reports
Completed. A failed/refused/cancelled attempt preserves the destination through
render/project's staging owner. A degraded retry requires an explicit toggle over
the listed diagnostics; a project/audio-scope change clears consent and newly
unavailable source ranges refuse again. Strict retry always reruns render admission
with consent off, so a restored source can export without accepting degradation;
a still-missing source refuses again and preserves the prior destination. Stale
diagnostics clear when that strict retry is submitted. Errors remain in the modal
for retry.

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
