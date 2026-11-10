# Services

Interface: `playback`, `preview`, `crop`, `color_picker`, `ai_task`, `progress`.

What more than one window needs and no window owns. Imports point here from the windows,
areas, frame and composition root; a services module never names a window, an area or
`Editor`.

| Module | Holds |
|---|---|
| `playback` | Engine-scheduled frames, the device clock and the one transport mapping. The app runs no second scheduler. The audio callback only copies samples, zero-fills underruns and reports callback-to-playback latency; every DSP and mix decision belongs to render. Video follows the latency-corrected audio clock and skips past frames under load. |
| `preview` | `View` (fit flag, explicit scale, pan offset, 0.25–16x), `FrameTexture` (one upload per playback epoch and tick, retaining its frame) and the snapshot boundary (`preview.snapshot`, clipboard copy). The player panel and the shorts window present their own chrome over the same state. |
| `crop` | The source-normalized crop model: `portrait` and `control` (edge and corner drag, ratio lock, 5% minimum edge, 1% and 10% arrow steps). The editor's crop dialog lives in the player panel. |
| `color_picker` | [Target-free colour controls and native screen sampler](color_picker/README.md). Callers own project targets, publication and history. |
| `ai_task` | `TaskEvents`: one coalescing progress slot and one exact per-upload consent request, carrying cancellation. |
| `progress` | `job_progress`: the stage line and progress bar every job surface draws from an engine snapshot. |

## Playback and limits

The device must be stereo f32 at the project's render-owned sample rate (default
48 kHz), else unavailable. The app never resamples, remaps channels or substitutes a
wall clock. Gaps and underruns play silence, keep the device timeline and are counted.
Pausing and seeking invalidate old ring samples by generation. Shutdown cancels and
joins the engine workers. An absent, failed or one-second-stalled device refuses or
pauses playback as unavailable.

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
