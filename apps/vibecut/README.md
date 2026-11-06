# vibecut

Native eframe/egui desktop frontend, using wgpu to present render's shared CPU
compositor. Owns device IO, the audio transport clock, retained preview frames,
presentation state, stable automation IDs, screenshots, catalog and preferences.
Engine owns all project edits, job scheduling and aggregate byte accounting;
render owns every pixel, audio conversion, sample grid and channel mix rule.

## Editor presentation

`editor.rs` retains the public `Editor` facade; `editor/state.rs` holds its
shared presentation context, `editor/widgets.rs` its automation-recording
controls and `editor/timing.rs` the tick-grid time formatting. `editor/frame.rs`
owns window composition and editor input routing. Its `State` owns the editor's
rename draft and splitter gesture; `show` returns window actions for the frame to
route. The read-only `editor::Context` carries the session, selection, locale and
theme into area interfaces. Each panel receives its complete slot, including its
own header strip; the frame supplies only the outer insets and draggable gutters.
The assets, player, properties, timeline, captions, export and models area files
are folder interfaces with private `view` and `workflow` implementations; existing
state projections live with their area. Responsibilities follow
[structure](../../docs/engineering/structure.md).

The physical extraction preserves existing facade calls and their ordering.
The player owns its retained view state and renders from read-only context,
returning requests for the frame to route. The assets State owns its navigation
drafts and prepared workflow projection; `show` consumes the shared
`editor::Context` and returns Action values routed by the frame. Its offscreen
audit uses the same surface. Remaining State/show/action migrations belong to
their consumer UI tickets; retained cross-area calls are recorded in each report.

`kit.rs` exposes the app palette, measured geometry, regular icons and the
shared controls without an engine dependency. All callers import that interface
directly. The [token contract](../../docs/product/tokens.md)
indexes the one executable value owner. The measured editor density (12/14 px type,
22 px controls, 4 px corners) is installed by `kit::install_editor_density` from the
editor frame only; every other window keeps the toolkit defaults. No standalone kit
demo is installed.

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
menu control switches the catalog at runtime and the existing bounded preference store
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
The optional control listener and background attach launch compose the CLI-owned
adapter over this app's live Workspace; syntax and boundaries are owned by
[MCP control](../../docs/product/mcp.md).
`cargo run -p vibecut -- /native/path/to/project.json` acquires the project writer
lease before loading. Startup keeps a supplied project path pending until
recovery discovery and the managed catalog are both idle, regardless of which
worker finishes first. With no file the managed Projects page lists the catalog
with thumbnails, name, main-scene duration and created date in a persisted grid
or list view and sort order. Search filters names live; click and shift-click
select, and the toolbar duplicates or permanently deletes selected projects.
Delete shows one confirmation naming the selection and has no typed gate.
Rename, open and project info (duration, created, modified and the 8-character
display ID) come from each card's actions. A catalog worker executes every
`project.create`, `project.duplicate`, `project.rename`, `project.delete` and
`project.thumbnail` command off the UI thread; the page never loads or mutates
project JSON itself. Opening a card and the new-project entry point then use the
same lifecycle/autosave path as external project files. Managed project listing
is supplied by [engine library](../../crates/engine/README.md#managed-project-library); the projects-page feature owns its presentation.
`cargo xtask ui-audit projects projects-rename projects-delete projects-info`
captures the populated page and its modal dialogs offscreen in both themes at
1x/2x. Failed catalog refresh retains the last listing; Retry restarts its worker.
Engine lifecycle hooks backfill and refresh project-owned thumbnails, including
close without saving editable content. Preview,
timeline basics, transport and the export dialog are present; text/audio controls
are integrated by their owning feature tickets.

Home, Editor and Shorts are independent native viewports. `Workspace::windows`
registers project windows by ProjectId; each owns its editor, history, playback,
lease, picker, pending persistence operation and close flow. Canonical path
reopens focus the existing window. Home continues to host catalog and startup
recovery prompts; each accepted recovery becomes a project window. Closing Home
hides it until Window > Home focuses it, while the event loop exits after
the last window closes. The old editor back-to-Projects action is removed.
`window_ui(project_id, ui)` drives that same project surface in kittest;
automation ids remain unchanged inside each viewport. `focused_editor` / `focused_editor_mut` and `editor_mut(ProjectId)` select
the registered session explicitly; Home never contains an active-editor slot.

The app shares catalog, preferences, model presentation, lifecycle runner and
aggregate media accounting. Each project's media budget is a 512 MiB child of
the 768 MiB aggregate; autosave has separate 64 MiB project children of a 256 MiB
persistence allowance, so export cannot consume journal/save admission. Model
transfers remain app-owned across project saves/closes. The native adapter keeps
an audio stream for each project and shares the GPU adapter. Preferences version
4 persists bounded logical size, position and maximization by Home, Shorts and
ProjectId (64 retained geometry entries, within the store's 16 KiB bound).
Native multi-window smoke is maintainer-gated; macOS/Linux window paths are
unverified on the Windows development host.

Media import uses one native multi-file picker or a file drop. One batch is capped
at 128 paths (32 KiB each), must admit a 448 MiB probe/work lease from the shared
byte budget, and publishes results through a bounded 8-event channel, so a paused
UI applies backpressure instead of accumulating metadata. Probing and hashing run
on an engine import job with one cooperative cancellation token carried into
media's FFmpeg probe interrupt and 64 KiB hash reads; each successful file becomes
an asset immediately, so a later refusal or cancel keeps earlier assets. Published
imports refresh playback against the updated snapshot, including ratcheted FPS. The media
panel omits media-owned ephemeral assets from rows and selection while retaining
them in the project for internal commands. Cards show a filename and duration;
probe type, resolution and audio format are available on hover. The panel can multi-select assets,
and an "Add to timeline" action that inserts only the selected assets in one undo
step through engine auto placement. The engine chooses compatible tracks and
creates secondary tracks on demand; the app owns no placement policy. While a batch runs, an overlay shows completed/total
progress and a 2 px accent bar, disables import/insert, and offers Cancel. Partial
failures use short recovery copy and close with a concise toast; typed probe
details remain in diagnostics. A terminal runner failure stays a typed
error and never reports success. Insert synchronizes playback against the new
immutable project snapshot. Image placement remains with its later feature
ticket.

The Media tab is the live asset library. Grid and list views share one persisted
mode; the sort dropdown orders by name, type, duration or file size, flips the
active key between ascending and descending, and persists key plus direction in
the versioned preference store. Sorting is stable, so equal keys keep project
order. File size is filesystem metadata read for display only (missing files show
no size); the project record stays unchanged. Cards and rows reuse the import
cache's decoded thumbnails and capability badges, with type, duration, size and
probe fields. Click, Shift range over the
displayed order, Cmd/Ctrl toggle, and a blank-area marquee share one asset
selection with insert and delete; a blank click or Escape clears it. The context
menu removes the whole selection through the registered `media.remove` command
after a confirmation dialog. In-use assets refuse with a typed reason listing the
timeline items; unused removals are undoable and restore the exact asset records.
`media.reveal` (also the timeline clip context menu's Reveal media) switches to
the Media tab, scrolls the asset into view and highlights it for 1.5 s without
touching project JSON. During an import batch the overlay reports progress and
disables view, sort, import and insert; cancel stays available.

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
single/Cmd-or-Shift multi-selection, drag move, edge trims, bookmark drag, split,
delete and ripple delete all dispatch engine commands. Snapping is a persisted preference (on by default) exposed by the
magnet toolbar control; it converts 10 logical pixels to ticks at the current
zoom and ranks item start/end, playhead, bookmark and animation-keyframe targets
by pixel distance then stable source order. The indicator line is drawn only
while a candidate is active. Move, trim and bookmark drag honour the toggle and
Shift bypasses it; the gesture preview changes no committed history or project
JSON until drop, and Escape restores the entry bytes. The app
action registry in `src/actions.rs` is the single source for key, menu and button
trigger ids and executable routes. `ActionTarget::Engine(EngineAction)` chooses
an engine-owned `CommandKind`; the same chosen kind supplies its id to execution
and its engine availability predicate. App payload construction and batch
selection updates stay in the frontend. `ActionTarget::Ui { owner, action }`
names an explicit app workflow, including dialog opening and the combined app /
engine undo history. Parameterized scene/item widgets resolve to registered
families before activation; unregistered ids refuse before state changes.
`Editor::action_enabled` adds app selection requirements to engine availability
and gates both generic action widgets and direct/key dispatch.

The read surface for the shortcuts dialog remains `actions()`, `action(id)`,
`ActionSpec` metadata/defaults/availability (`bindable()` excludes parameterized
widget families), effective bindings and conflicts on
`KeybindingPreferences` / `Keymap`, and `Editor::set_keybinding` /
`reset_keybinding` / `reset_keybindings`. Target payloads are executable route
enums; consumers need not match them to list or edit shortcuts.
[The keyboard guide](../../docs/product/keyboard.md) holds its
generated default table. Global dispatch runs in the capture phase, waits while
an overlay or dialog is open, a project is loading, recording is active or a text
field has focus, and consumes one exact chord once. Paste is intercepted only
when the in-app clipboard has an entry; otherwise the event remains for media
paste. Shift bypasses snapping during a drag. Scrubbing uses the same seek path.
Binding overrides and `isCustomized` markers persist in the one preference store;
unknown actions and conflicts are reported, and unsupported import profiles are
refused with a typed diagnostic. The header Shortcuts action opens a four-tab
dialog grouped by action category: rows show platform glyph chips, click-to-record
captures the next chord, a conflict names the existing action and leaves both
bindings untouched, unbound actions stay listed and bindable, and per-action or
all-action reset removes the override and its `isCustomized` marker. Every change
is the app-local `ui.keybindings.set` / `ui.keybindings.reset` command, so undo and
redo restore the previous map without touching Project JSON. Import/export JSON
controls are visible but disabled until settings import/export (p1-parity/75).
Two resizable columns and a horizontal
divider start at the UI design's 25/50/25 and 50/50 proportions. Drags clamp
assets/properties to 15–40%, preview to 30–85% and the timeline to 15–70%, so no
region reaches zero. The assets panel has a horizontal icon-over-label strip
for Media, Audio, Text, Effects and Captions, followed by a per-tab sub-navigation
column. Unsupported tabs and categories remain hidden. Media owns the live
library, Audio filters local audio, Text inserts text and Effects lists the engine
catalogue. A single
selection shows that element type's properties tabs, and the last tab is
remembered per type. Text, video with/without source audio, image, sticker,
graphic, audio and effect have the reference tab sets in `src/panels.rs`; image
currently means a source video stream reporting one frame, and sticker/effect
items await standalone item contracts before those two sets are reachable from a
real project. The effect-stack contract from p1-parity/33 is attached to existing
visual items, whose Effects tabs are already available; p1-parity/97 owns their
stack controls and the Effects asset view. Captions remains a separate asset-view
mount point for the subtitle editor. English/Vietnamese, theme, panel proportions,
property-section open state, the active assets tab, per-type properties tabs and
timeline snapping and the preview fit/zoom (with the last snapshot directory) use one bounded, versioned preference store;
`ui.prefs.set` edits have app-owned undo/redo and never touch Project JSON.
Tab navigation is presentation state: it persists through the same validated
store but stays out of app undo history so it cannot shadow a project edit. A
damaged layout restores its defaults with a visible warning, while an unreadable
document restores the system defaults. Unversioned and unsupported versions are
refused without rewriting their bytes on load or save. Edits and writes share the 16 KiB
read budget, so a saved document always remains readable.

The selection inspector shows audio gain/mute and text content, font, size, colour,
background and position for compatible selected items, with a dash for mixed
values and the compatible/selected count. A single element uses the per-type tabs
above instead: Text has content, font, background and position fields alongside a
size scrub and colour entry, Blending a blend menu and opacity scrub, Audio a
gain scrub and mute checkbox, and Speed (video and audio) a 0.01-5x scrub with a
reset button and, when the item has audio, a "Change pitch" checkbox. "Change
pitch" is on while pitch follows speed (`maintain_pitch` off), as in the reference.
Speed edits go through `timeline.set_speed`, one undo step per commit; the timeline
trim handles bound extension by the retimed output length. Caption generation
refuses a retimed source item until captions gain a rate-aware mapping.
`cargo xtask ui-audit inspector-speed` captures the populated Speed tab in both
themes at 1x/2x.
Each scrub field has a left drag handle, a click-to-type entry and clamps; the
drag previews through an engine gesture, commits as one registered
`timeline.set_item_param` undo step on release, and Escape cancels the gesture or
the typed draft. Changing selection or hiding the scrub field cancels its
preview before another edit can join the gesture. Invalid or non-finite input refuses without changing the item.
Drag sensitivity is our recorded decision because the inventory leaves it
unverified: opacity and audio gain move one 0.01/1 dB step per 4 px, and text size
one px per 2 px, speed 0.01 per 2 px. Enter or blur commits a typed shared-field patch through the
engine registry in one undo step. Text editing owns keyboard input while focused.
The text button inserts a five-second title at the playhead in one undo step.
Video volume awaits its owning contract before joining the same inspector.

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
enabled/focus/rect/current view, playing, position, selection, zoom, snapping
and the active snap-indicator target. The project remains the Session's immutable
snapshot. Each interactive
control, including every timeline item and trim handle, exposes the same stable
`author_id` through AccessKit, independent of locale and layout. Headless tests
query those IDs and click or drag through the real widget/engine path.

`Capture::request` queues a fresh GPU readback and `poll` waits without blocking
for at most five seconds; window and panel rectangles are supported. Unavailable
or timed-out integrations return typed failures. Captures demonstrate presentation,
never command success; inspect engine state separately. Fully occluded/minimized
windows may time out rather than return an old framebuffer.

## Preview transport and snapshot

The preview panel renders the shared transport toolbar, an editable
`HH:MM:SS:FF` timecode plus the project duration, and Fit / 25 / 50 / 75 / 100 /
150 / 200% zoom presets with continuous 0.25–16× steps. The timecode formatter
and parser come from `time`'s shared DF/NDF owner (`29.97`/`59.94` use `;`, all
other admitted rates use `:`); Enter dispatches `playback.seek`, Esc or invalid
text restores the previous value. Ctrl/Cmd+wheel and pinch zoom, a plain wheel or
two-finger scroll pans only while zoom is above 100%, and middle-drag pans within
the canvas bounds. Fullscreen covers the window and sends the native viewport
request; Esc or the context-menu/toolbar toggle exits. The fit flag and explicit
scale persist in the one preference document, as does the last snapshot
directory.

Snapshot requests go through the engine's `preview.snapshot {time, path}`
command: the toolbar or context menu opens the native PNG save picker, a picked
destination is published atomically, and cancel is a no-op. Empty projects refuse
with the typed `Project is empty` error, and write failures remain typed with no
success toast. Copy uses `engine::snapshot::image` for the same frame and hands
it to egui's OS clipboard owner. Neither action edits the project or its history.

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

Timeline header and row geometry follows the [UI design](../../docs/product/ui-editor.md#density),
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

The Transform tab reuses the inspector's scrub fields for position X/Y, independent
scale X/Y and rotation, with an inline reset on each row. Units, bounds and defaults
belong to project's `TransformField`; every edit uses `timeline.set_item_param`.

`color_picker` owns one reusable colour draft for text, its background box,
graphic fill/stroke, existing mask strokes and the empty inspector's canvas
background. Hex/HSV/alpha entries, sliders, swatches and the sixteen most recent
committed colours share it. Stable IDs are `color.hex`, `color.h/s/v/a`, their
`.slider` children, `color.swatch.N`, `color.recent.N` and `color.eyedropper`.
During a slider drag a temporary engine Session supplies only the preview snapshot;
the editable Session, persistence and undo history remain unchanged. Release
publishes one command/undo step. Escape or closing discards the active draft;
selection/snapshot changes fence stale drafts. Recent colours are unique,
newest-first and capped at sixteen in preferences version 4; older preference
versions refuse and retain their bytes under the existing store policy.

The app-only native screen sampler invokes AppKit `NSColorSampler` through macOS's
system JXA bridge asynchronously; no browser EyeDropper API, screenshot capture,
new dependency or core OS import is used. RGB replaces the draft hex and retains
alpha. Closing the draft cancels/joins its owned child, and 120 seconds bounds
an abandoned request. Windows/Linux return typed `EyedropperState::Unavailable`
with localized copy; native macOS interaction is unverified on Windows.
`cargo xtask ui-audit inspector-transform inspector-color` captures both new scenes
at 1x/2x in both themes. The ticket's headless kittest saves `target/p1-45-colour.png`.

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

Cue times seek in output coordinates. Shift extends a range from a cue identity;
after merging away that cue, the surviving selection supplies the anchor.
Cmd/Ctrl toggles.
Text edits retain independent language edit/provenance flags and normalize through
captions. Leaving a field or switching tabs commits drafts. Cue, page, item,
language and timing actions also commit drafts before changing the view or content;
unrelated project edits retain
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

Audio and video source items share the batch inspector's volume/mute controls;
volume uses a 0.01 dB step over -60..20 dB. Existing `inspector.audio_gain` and
`inspector.audio_muted` automation IDs remain stable. Playback retains render's
`AudioMixer` for each snapshot epoch; the app owns device IO and scheduling
adapters, while render owns every DSP/sample decision.
