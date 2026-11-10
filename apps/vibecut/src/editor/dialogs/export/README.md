# Export dialog

The export sheet and the job that backs it. `State` owns at most one
`ExportDialog`, created lazily from playback's byte budget and render state;
`show` reads the saved choices from the editor context, paints the modal and
returns start, retry, save, preference, saved and remember requests. The frame
routes them through the shared desk, so the dialog never writes the editor
error line or the preview itself.

`dialog` owns the engine job: it stages a unique temporary output, reports
engine stage and progress, cancels and retries, and publishes a finished file
through the same atomic path. `settings` renders `render::OutputOptions`:
resolution presets keep the canvas ratio, custom even dimensions change output
only, and explicit frame rate, target bitrate, compatible codec and H.264
profile feed the renderer. Range is full by default, or frame-snapped in/out
markers or the selected clips' bounds; range errors disable Export before any
job is submitted. Scene scope is active by default, all, or a named scene.
Preferences persist these choices plus the format, quality, audio switch and
last directory; source consent and overwrite authorization stay transient.
Project and undo history never store export state. An empty timeline shows a
catalog error and refuses start.

All-scene output is rendered as one staged batch, then saved scene by scene
with distinct scene-id filenames through the native save workflow. Cancelling a
save picker keeps the current and remaining finished files; closing the dialog
discards them. `export.run` / `export.retry` submit through engine's one export
job path and share playback's byte budget.

On completion the app opens the asynchronous native save dialog with the
sanitized `<ProjectName>.<ext>` and last save directory. Filenames keep Unicode
and spaces, replace invalid filesystem characters, trim trailing dots and
spaces and escape Windows reserved device names. The filename and container
filter follow the rendered attempt, even if format preferences changed
meanwhile. `Editor::export_save_defaults` supplies the defaults;
`Editor::export_picked_file`, `Editor::export_save_cancelled` and the
`Workspace` picker inject native results. Cancelling the dialog writes nothing
and keeps the finished job ready to save without re-exporting; closing the
popover discards it.

Saving submits engine's completed-export publication job with the rendered
snapshot and Autosave's protected project, metadata and recovery paths.
Admission, copy and sync run on the runner through project's atomic stage.
Progress and cancel stay in the dialog; cancellation before commit and failed
staging or publication keep the finished render and the previous destination.
Only confirmed publication persists the directory, toasts the path and closes
the modal. Shutdown joins the runner before removing owned temporary files.

A refusal or failed attempt replaces progress with an inline typed error panel
with text-only Copy and Retry. Retry reruns the same attempt settings into a new
temp output, leaving a prior successful file untouched; publication Retry
reopens destination selection without re-encoding. Retry refuses running,
cancelled and successful attempts. A degraded retry needs an explicit toggle
over the listed diagnostics; a project or audio-scope change clears consent.
Consented degraded output follows the same save/toast/close path, with affected
ranges disclosed before running and no post-completion warning. Tests inject
production source failures at render's source-factory boundary.
