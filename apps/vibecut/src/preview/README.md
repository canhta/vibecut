# Preview

Preview view state, frame upload and the snapshot boundary, with no editor state. `View` owns the fit flag, explicit scale and pan offset; fit scales the canvas into the viewport keeping aspect, the scale range is 0.25–16× and pan is only legal above 100% with the scaled canvas still covering the viewport. The player panel and any other preview window present their own frames over the same state.

`FrameTexture` is the one upload rule: a frame is keyed by its playback epoch and tick, re-uploaded with `set` only when that key changes, and its source frame is retained while the texture refers to its pixels. A window that draws no frame clears it, which drops the upload and its retained frame together.

Timecode format and parse come from `time`'s DF/NDF owner (`29.97`/`59.94` use `;`, other rates `:`); the panel owns the editable field and dispatches `playback.seek` on Enter.

Snapshots go through the engine's `preview.snapshot {time, path}`: the panel's toolbar or context menu opens the native PNG save picker, a picked destination is published atomically, cancel is a no-op. Empty projects refuse with the typed `Project is empty` error; write failures stay typed with no success toast. Copy uses `engine::snapshot::image` for the same frame and hands it to egui's OS clipboard. Neither edits the project or history.
