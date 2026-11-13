# Export dialog

Interface: `State`, `Action`, `ExportDialog`, `ExportOptions`, `ExportFormat`,
`ExportQuality`, `ExportPhase`, `ExportInspection`, `SaveRequest`, `OutputOptions`,
`RangeChoice`, `SceneChoice`, `default_file_name`, `sanitize_file_name`.

`State` owns at most one `ExportDialog`, created lazily from playback's byte budget
and render state. `show` reads the saved choices from the editor context, paints the
modal and returns start, retry, save, options, saved and remember requests; the frame
routes them through the shared write context, so the dialog never writes the editor
error line or the preview itself.

The dialog owns the engine job: it stages a unique temporary output, reports engine
stage and progress, cancels and retries, and publishes a finished file through the
same atomic path. Settings render `render::OutputOptions`: resolution presets keep the
canvas ratio, custom even dimensions change output only, and explicit frame rate,
target bitrate, compatible codec and H.264 profile feed the renderer. Range is full by
default, or frame-snapped in and out markers or the selected clips' bounds; range
errors disable Export before any job is submitted. Scene scope is active by default,
all, or a named scene. Audio-only formats (M4A, WebM audio) hide the video controls and export the mastered mix. Preferences persist these choices plus the format, quality,
audio switch and last directory; source consent and overwrite authorization stay
transient. Project and undo history never store export state. An empty timeline shows
a catalog error and refuses start.

All-scene output is rendered as one staged batch, then saved scene by scene with
distinct scene-id filenames through the native save workflow. Cancelling a save picker
keeps the current and remaining finished files; closing the dialog discards them.
`export.run` and `export.retry` submit through engine's one export job path and share
playback's byte budget.

Desktop export keeps the CPU effect reference in its render settings. Native preview
attachment is separate from the shared render state, so a successful GPU preview
does not attach an adapter to export. The [render effect interface](../../../../../../crates/render/src/scene/README.md#effects-and-desktop-adapter)
accepts an optional device-free adapter; its native resource contract lives with
[GPU effects](../../../services/gpu/effects/README.md).

On completion the app opens the asynchronous native save dialog with the sanitized
`<ProjectName>.<ext>` and last save directory. Filenames keep Unicode and spaces,
replace invalid filesystem characters, trim trailing dots and spaces and escape
Windows reserved device names; the name and container filter follow the rendered
attempt, even if format preferences changed meanwhile.

Saving submits engine's completed-export publication job with the rendered snapshot
and Autosave's protected project, metadata and recovery paths. Progress and cancel
stay in the dialog; cancellation before commit and failed staging or publication keep
the finished render and the previous destination. Only confirmed publication persists
the directory, toasts the path and closes the modal. Shutdown joins the runner before
removing owned temporary files.

A refusal or failed attempt replaces progress with an inline typed error panel with
text-only Copy and Retry. Retry reruns the same attempt settings into a new temp
output, leaving a prior successful file untouched; publication Retry reopens
destination selection without re-encoding. Retry refuses running, cancelled and
successful attempts. A degraded retry needs an explicit toggle over the listed
diagnostics, and a project or audio-scope change clears that consent.

The settings sheet recomputes spoken-word warnings from the captured project, output choices and shared render state before submission. Safe reason labels never include caption text, and a warning keeps the ordinary base caption rather than blocking an otherwise valid export.
