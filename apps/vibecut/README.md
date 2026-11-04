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
`cargo run -p vibecut -- /native/path/to/project.json` acquires the project writer
lease before loading. With no file it opens the minimal Home new/open entry points.
Project listing and thumbnail storage belong to the projects-page feature. Preview,
timeline basics and transport are present; text/audio controls and export are
integrated by their owning feature tickets.

Media import uses one native multi-file picker or a file drop. One batch is capped
at 128 paths (32 KiB each), must admit a 256 MiB probe/work lease from the shared
byte budget, and publishes results through a bounded 8-event channel, so a paused
UI applies backpressure instead of accumulating metadata. Probing and hashing run
on an engine import job with one cooperative cancellation token carried into
media's FFmpeg probe interrupt and 64 KiB hash reads; each successful file becomes
an asset immediately, so a later refusal or cancel keeps earlier assets. The media
panel shows the probe type/duration/resolution/audio format, multi-select assets,
and an "Add to timeline" action that inserts only the selected assets in one undo
step on compatible tracks. While a batch runs, an overlay shows completed/total
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
divider start at the UI design's 25/50/25 and 60/40 proportions.
English/Vietnamese, theme and panel proportions use one bounded preference store.

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
transport/seek/clear, byte refusal and worker shutdown. The timeline evidence test
renders dark/light headless kittest PNGs and an automation-ID geometry receipt into
`$CARGO_TARGET_DIR/screenshots/` (or `target/screenshots/`); it opens no native
window. `cargo xtask ci` is the handoff check. The local ticket report records observed
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
native picker dependency. `tests/lifecycle.rs` exercises lifecycle boundaries and
real rename/close/recovery widgets, and renders `target/p1-02-lifecycle.png`.
Native picker interaction and Windows runtime need separate observed evidence.
