# Preview transport and snapshot


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
