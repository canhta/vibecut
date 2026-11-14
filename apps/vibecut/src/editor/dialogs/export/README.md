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
all, or a named scene. Preferences persist these choices plus the format, bit rate preset,
audio switch and last directory; source consent and overwrite authorization stay
transient. Project and undo history never store export state. An empty timeline shows
a catalog error and refuses start.

The settings are three tickable sections, as in the reference (CapCut 9.7.1, `.spikes/ui-reference/capcut-desktop-2026-11-14b/` row 12, frames 105340, 105528, 105532, 105549, 105736): Video, Audio and GIF. One export job writes one file, so the sections are exclusive here: ticking one unticks the others, and the only ticked section's box is disabled with `Keep at least one output ticked.` on hover (the reference exports several files at once; that is not built). The saved `ExportFormat` carries the choice: a video format with Video ticked, `M4a` or `AudioWebm` with only Audio ticked, `Gif` with GIF ticked; Audio beside Video is `include_audio`. Untick Video and its block (Resolution, Bit rate, Codec, Format, Frame rate, Profile, Custom size) is disabled, not hidden, and hovering a row names why (`Not used by audio-only output.`); ticking Video again returns to the video format it left. This is the disabled-not-hidden rule of every silent or audio output: the encoder rows that do not apply (Bit rate, Codec, H.264 profile) are disabled for GIF, PNG sequence and audio-only output, and Codec and Profile offer Automatic only when enabled.

Video block order is Resolution, Bit rate, Codec, Format, Frame rate, then ours: H.264 profile and Custom size. Format lists MP4, MOV, WebM and, ours, PNG sequence. The reference's Remove watermark, Optical flow, Color space, Sync to space and Check copyright rows have no feature behind them and are not built.

Bit rate is a select with Lower, Recommended, Higher and Custom. The presets map to our encoder settings in `ExportQuality::value` (`dialog.rs`), read by `ExportOptions::settings`: Lower is quality 70 at 6,000,000 b/s, Recommended 85 at 12,000,000 b/s (the default), Higher 95 at 24,000,000 b/s. Custom shows a Kbps field under the select, starts at 12000 and takes 1 to 50000 Kbps (the reference's ceiling, read as 99999999 becoming 50000); it writes `OutputOptions::bit_rate` in b/s (Kbps times 1000), which replaces the preset's rate and keeps the last preset's quality scalar. A preset clears `bit_rate`. Only `OutputOptions` is a shared shape: the command declaration and the CLI carry `bit_rate` in b/s and no preset, so the dialog's preset names never leave the app and its saved `quality` is app preferences only (preferences version 18). The "?" beside the label shows the reference's hint about image quality.

Audio lists AAC (M4A) and, ours, Opus (WebM); the Format row is live only when Video is unticked, otherwise it is disabled with `Used only when Video is unticked.` (the video file carries its own track). The reference lists MP3, WAV, AAC and FLAC. MP3, WAV and FLAC need encoders and containers the media crate does not have, so they are not offered.

GIF is a section with Resolution only (240P, 320P, 640P, on the project's canvas ratio), replacing GIF as a Format choice. Ticking it clears the custom bit rate, sets 240P and unticks the others; its Resolution is disabled until it is ticked (`Tick GIF to use this.`). Leaving it restores the project size. The Colors and Dithering rows are gone (the reference has none); `OutputOptions::gif` keeps its defaults for dialog exports and stays settable through the command and the CLI. The Frame rate row stays live for GIF, ours, because `OutputOptions::check_gif` caps the rate; Export stays disabled with the shared range error when the resolution, rate or range breaks the GIF bounds. The reference marks 320P and 640P with Pro badges; we have no paid tiers, so none is drawn.

PNG sequence is a further Format row entry, always offered (it needs no encoder). Choosing it clears the custom bitrate, resets Codec and Profile to Automatic and turns an All-scenes choice into the active scene. The same disabled-with-reason rule applies as for GIF, Video stays the only ticked section (its box and the Audio box are disabled), and the All-scenes option is disabled with `A PNG sequence holds one scene.` on hover. It adds no rows, so no new geometry is invented (see [UI editor](../../../../../../docs/product/ui/editor.md)). The job renders `<name>_%05d.png` frames into a temporary directory through the one export job; Save then opens the native save picker without a file-type filter, named after the project, and the picked name becomes the directory (and the frame prefix) that `submit_sequence_publication` promotes whole. Export stays disabled with the shared range error when the range holds more than 99,999 frames (`OutputOptions::check_sequence`). A finished temporary directory is removed like a finished file.

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

The caption section (`caption_outputs.rs`) lists SRT and TXT, the reference's two formats, then VTT and the burned MP4 as ours, under the settings. It stays four save buttons rather than a tickable section with a format select: caption files are written through the captions panel's own export, not the one export job, and the reference's Character encoding row is fixed to UTF-8 here. Each click returns `DialogAction::Caption`; the frame raises `EditorAction::ExportCaption` and the window's native save picker answers with the path, which the captions panel writes through its engine export. Its buttons are enabled only when the project holds a caption item. The settings sheet recomputes spoken-word warnings from the captured project, output choices and shared render state before submission. Safe reason labels never include caption text, and a warning keeps the ordinary base caption rather than blocking an otherwise valid export.
