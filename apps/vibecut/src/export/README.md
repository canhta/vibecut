# Export settings and publication

`export/settings` renders `render::OutputOptions`: resolution presets preserve
canvas ratio, custom even dimensions change output only, and explicit frame rate,
target bitrate, compatible codec and H.264 profile feed the same renderer. Range
is full by default, or frame-snapped in/out markers or the selected clips' bounding
range. Scene scope is active by default, all, or a named scene. Preferences
version 6 persists these choices and refuses earlier versions. `Editor::dispatch`
accepts typed `ExportOptions` for `export.run`; Retry retains the original settings
and immutable snapshot. The project and its undo history never store export state.

All-scene output is rendered as one staged batch, then saved one scene at a time
with distinct scene-id filenames through the existing native save workflow.
Cancelling a save picker retains the current and remaining completed files;
closing the export dialog discards them. Range errors disable Export before any
job is submitted. The settings child owns the output controls inside `export`.

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
