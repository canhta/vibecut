# Editor player

Owns the preview panel: transport, preview presentation and snapshot, the crop
dialog, and the playback service the editor windows share. `playback` holds the
engine-scheduled frames and device clock, `preview` the zoom/pan view and
snapshot request, and `crop` the source-normalized crop interaction and dialog.

`transform` draws the selected item's four corner and two side scale handles and
its rotation grip below the box from `render::viewport` geometry, then turns
pointer gestures into `timeline.set_item_param` patches inside `timeline.preview`.
One drop publishes one `timeline.commit_preview` step; Escape or a gesture that
returns to its origin discards the candidate. Hidden and locked items expose no
handles and cannot be hit. Handle face, hit target, grip gap and colours come
from `kit` tokens.

`snap` resolves the canvas targets a gesture can meet: the canvas centre and
edges and every other visible item's transformed box edges, in canvas pixels.
Move snaps the box's centre and both edges per axis, scale snaps the dragged
handle point per axis, and rotation snaps to quarter turns. The threshold is the
shared screen-pixel policy converted at the preview scale; `Preferences::snapping`
turns it off and Shift bypasses it for the gesture, the same rule the timeline
uses. A dashed accent guide is painted for each active target and cleared on
release, so the committed project and its undo history stay untouched until the
drop.

A double click on a visible text item opens one inline session over its rendered
box. Transparent input glyphs leave the preview visible; the caret and selection
are painted at the text crate's layout positions for the item's own font, size,
letter spacing, line height and alignment, mapped through the item's placement
transform so rotation and scale keep them on the rendered glyphs. Pointer press,
double-click word selection and drag selection invert that geometry and use the
text crate's point-to-index query; egui only supplies key and IME input. They use
the app accent at the measured caret width and selection opacity. Each change
stages the whole buffer as `text_content` inside `timeline.preview`; Escape, blur and
starting playback commit it as one undo step, Enter inserts a newline, and an
empty buffer is a valid commit. A locked or hidden item never opens the overlay.

## Playback and limits

Two engine Preview jobs own decode-ahead and audio mixing; there is no second scheduler. Audio's fixed atomic stereo ring is engine-budgeted. The callback only copies samples, zero-fills underruns, advances monotonic device sample counters and reports callback-to-playback latency. Video follows the latency-corrected audio clock and skips past frames under load. An absent, failed or one-second-stalled device refuses or pauses playback as unavailable.

The device must be stereo f32 at the project's render-owned sample rate (default 48 kHz), else unavailable. The app never resamples, remaps channels or substitutes a wall clock. Gaps and underruns play silence, keep the device timeline and are counted. Pausing and seeking invalidate old ring samples by generation. Shutdown cancels and joins the engine workers.

Preview fits within 640A-360 keeping aspect. At most four queued frames, one shown frame and the upload's retained frame exist at once. Render leases its working buffers; app leases cover the ring and CPU ColorImage/upload/texture retention; all share the one injected engine budget. The native default is 768 MiB including render's concurrent probe/decode allowances; it is an admission ceiling, not resident usage. Pressure or an oversized frame returns a typed error before allocation and shows unavailable. OS codec/driver allocations and allocator overhead need separate RSS measurement. A healthy cache clear keeps position and play state and invalidates queued output.

A terminated preview job pauses transport at the device-clock position, fences old output and cancels its companion. Retry keeps playback paused while engine polling joins finished jobs, then resubmits both jobs with the same source factory, audio port and budget. It never blocks the UI on an adapter; a noncooperating adapter delays replacement and shutdown until it returns. Playback resumes only explicitly. Old work cannot publish into the new generation, and a partial resubmission failure cancels any admitted replacement.

## Preview transport and snapshot

The preview panel renders the transport toolbar, an editable `HH:MM:SS:FF` timecode with the project duration, and Fit / 25 / 50 / 75 / 100 / 150 / 200% zoom presets with continuous 0.25-16x steps. Timecode format and parse come from `time`'s DF/NDF owner (`29.97`/`59.94` use `;`, other rates `:`). Enter dispatches `playback.seek`; Esc or invalid text restores the previous value. Ctrl/Cmd+wheel and pinch zoom; plain wheel or two-finger scroll pans only above 100%; middle-drag pans within canvas bounds. Fullscreen covers the window and sends the native viewport request; Esc or the toggle exits. The fit flag, explicit scale and last snapshot directory persist in the preference document.

Snapshots go through the engine's `preview.snapshot {time, path}`: the toolbar or context menu opens the native PNG save picker, a picked destination is published atomically, cancel is a no-op. Empty projects refuse with the typed `Project is empty` error; write failures stay typed with no success toast. Copy uses `engine::snapshot::image` for the same frame and hands it to egui's OS clipboard. Neither edits the project or history.

`preview::View` has no editor dependency, so another window can present the same
fit/zoom/pan state over its own frames.

## Crop

Owns the transient source-normalized crop interaction; project owns the crop
contract and render consumes it. The toolbar Crop modal edits a transient
`project::Crop` through this control, locked to the selected ratio (Custom
resizes freely, Original keeps the source shape), and writes it with one
`timeline.set_item_param` patch on Apply, so closing never mutates the project.
