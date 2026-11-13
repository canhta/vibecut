# Services

Interface: `playback`, `preview`, `crop`, `color_picker`, `ai_access`, `ai_task`, `remote_models`, `progress`, `text_clipboard`, `lifecycle`, `gpu`.

What more than one window needs and no window owns. Imports point here from the windows,
areas, frame and composition root; a services module never names a window, an area or
`Editor`.

| Module | Holds |
|---|---|
| `playback` | Engine-scheduled frames, the device clock and the one transport mapping. The app runs no second scheduler. The audio callback only copies samples, zero-fills underruns and reports callback-to-playback latency; every DSP and mix decision belongs to render. Video follows the latency-corrected audio clock and skips past frames under load. |
| `preview` | `View` (fit flag, explicit scale, pan offset, 0.25–16x), `FrameTexture` (one upload per preview generation and tick, retaining its frame) and the snapshot boundary (`preview.snapshot`, clipboard copy). The player panel and the shorts window present their own chrome over the same state. |
| `crop` | The source-normalized crop model: `portrait` and `control` (edge and corner drag, ratio lock, 5% minimum edge, 1% and 10% arrow steps). The editor's crop dialog lives in the player panel. |
| `color_picker` | [Target-free colour controls and native screen sampler](color_picker/README.md). Callers own project targets, publication and history. |
| `ai_access` | The one desktop composition of provider credentials, the process models cache (opened once) and registry dispatch (`run`): windows send `captions.*`/`clips.*` commands with the `CaptionAccess` an `Opener` builds per run; tests inject the opener. |
| `ai_task` | `TaskEvents`: one coalescing progress slot and one exact per-upload consent request, carrying cancellation. |
| `remote_models` | [Model selection and explicit refresh](remote_models/README.md). |
| `text_clipboard` | Read-only native text access with injected reader composition; viewport custody belongs to [input](../input/README.md). |
| `progress` | `job_progress` and `line`: the status line with its percentage over the kit progress bar, drawn from an engine snapshot or a surface's own status words. The percentage is now a "Status N%" line above the bar, not text inside it. |
| `lifecycle` | [Project lifecycle](lifecycle/README.md): `EditorSession`, the editor's memory or autosave-backed engine session. |
| `gpu` | [GPU adapters](gpu/README.md): `GpuEffects`, the wgpu implementation of render's effect interface, and `Capture`, bounded fresh-frame screenshots. |

## Playback and limits

The device must be stereo f32 at the project's render-owned sample rate (default
48 kHz), else unavailable. The app never resamples, remaps channels or substitutes a
wall clock. Gaps and underruns play silence, keep the device timeline and are counted.
Pausing and seeking invalidate old ring samples by generation. Shutdown cancels and
joins the engine workers. An absent, failed or one-second-stalled device refuses or
pauses playback as unavailable. `poll` takes the caller's monotonic instant for
health checks; only actual callback progress or an explicit transport reset
refreshes that baseline. End detection uses the device transport position even
while the pointer holds a different preview; the held preview survives the stop.

Preview fits within 640x360 keeping aspect. At most four queued frames, one shown frame
and the upload's retained frame exist at once. The native default is 768 MiB including
render's concurrent probe and decode allowances; it is an admission ceiling, not
resident usage. Pressure or an oversized frame returns a typed error before allocation
and shows unavailable. A healthy cache clear keeps position and play state and
invalidates queued output.

A terminated preview job pauses transport at the device-clock position, fences old
output and cancels its companion. Retry keeps playback paused while engine polling joins
finished jobs, then resubmits both jobs with the same source factory, audio port and
budget. It never blocks the UI on an adapter; a noncooperating adapter delays replacement
and shutdown until it returns. Playback resumes only explicitly, and old work cannot
publish into the new generation.

`transport` is the one mapping from a player request to a seek: play toggles and
restarts at an end position, second and frame steps clamp to the timeline, and Start and
End are explicit positions. The editor frame and the shorts window call it with the
immutable project their preview renders.

`set_hover` is a transient preview time for the timeline's preview axis: it renders one
frame at a nonnegative frame-snapped pointer time, including empty tail past the
project duration, while the transport position, play state and audio stay put.
The compositor excludes each ended layer and leaves globally empty tail black.
Each pointer time fences only the preview generation, so only the latest frame
request survives; clearing it or a seek returns the preview to the transport.
The independent audio epoch, queued PCM, source cursor, mixer state and callback
health baseline are unchanged by pointer movement. Seek, pause and snapshot
replacement invalidate both generations with checked increments. Audio failures
remain attached to the audio epoch; pointer changes clear only preview failures.

## Preview and snapshot

Timecode format and parse come from `time`'s DF/NDF owner (`29.97` and `59.94` use `;`,
other rates `:`); the panel owns the editable field and dispatches `playback.seek` on
Enter.

Snapshots go through the engine's `preview.snapshot {time, path}`: the panel's toolbar or
context menu opens the native PNG save picker, a picked destination is published
atomically, cancel is a no-op. Empty projects refuse with the typed `Project is empty`
error; write failures stay typed with no success toast. Copy hands the same frame to
egui's OS clipboard. Neither edits the project or history.

## AI task progress

`TaskEvents` implements engine's `Progress` and `ConsentPrompt` and carries one
cancellation token. An upload request holds exactly the provider, model, languages,
payload size, duration, cost and policy links the caller must disclose; the workflow
blocks until the user approves or denies, the request is answered once, and a cancelled
task returns `Cancelled` without sending approval. The modal keeps the request until
answered, so a tab switch cannot drop it.

Unknown-price uploads have an explicit checkbox. `TaskEvents` remembers at most
16 acknowledged exact provider/operation identities within that task, while every
payload still requires upload approval; changed models or job kinds need a new
acknowledgement. Cancellation never sends approval.

## Focused text reads

`TextClipboard` services each viewport's claimed input request in that viewport's callback,
then posts an explicitly tagged completion and requests repaint. Composition retires
custody for hidden/removed viewports even when they receive no final callback. It uses the same path for
Home and immediate Editor/Shorts children, independent of eframe's root-only native action
drain. The reader is injected at composition; production opens a scoped read-only
`arboard::Clipboard` and calls `get_text`, treating unavailable/empty text as completion.
There is no text history, extra clipboard store, image fallback, logging or keyframe JSON
serialization. Egui retains Copy/Cut output and genuine unowned Paste events.

Arboard 3.7.0 (MIT OR Apache-2.0) is the existing single transitive version, now a direct
app dependency with defaults disabled and `wayland-data-control` enabled. Its official
[API](https://docs.rs/arboard/3.7.0/arboard/struct.Clipboard.html) owns native read semantics;
[Linux support](https://docs.rs/crate/arboard/3.7.0/source/README.md) includes X11 and
Wayland data-control. The app contains no unsafe clipboard wrapper or framework fork.

TaskEvents’ consent request and ConsentPrompt methods consume the provider result: queue
poison/cancellation retain Runtime causes and refusal/disconnect retain provider consent refusal.
Answer/event methods keep their genuine mixed app result. Unknown-price acknowledgement is
still separate and bound to the exact provider configuration and operation.
