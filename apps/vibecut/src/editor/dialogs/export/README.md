# Export dialog

Interface: `State`, `Action`, `ExportDialog`, `ExportOptions`, `ExportFormat`,
`ExportQuality`, `AlsoOutputs`, `ExportPhase`, `ExportInspection`, `SaveRequest`, `OutputOptions`,
`RangeChoice`, `SceneChoice`, `default_file_name`, `sanitize_file_name`.

`State` owns at most one `ExportDialog`, created lazily from playback's byte budget
and render state. `show` reads the saved choices from the editor context, paints the
modal and returns start, retry, save, options, saved and remember requests; the frame
routes them through the shared write context, so the dialog never writes the editor
error line or the preview itself.

`State::tick` reports the tick a render finished on; with the `export_sound` preference
the frame then plays `chime` (two short synthesised tones on the default output device,
silent when none opens).

The dialog owns the engine job: it stages a unique temporary output, reports engine
stage and progress, cancels and retries, and publishes a finished file through the
same atomic path. Settings render `render::OutputOptions`: resolution presets keep the
canvas ratio, custom even dimensions change output only, and explicit frame rate,
target bitrate, compatible codec and H.264 profile feed the renderer. Range is full by
default, or frame-snapped in and out markers or the selected clips' bounds; range
errors disable Export before any job is submitted. Scene scope is active by default,
all, or a named scene. Preferences persist these choices plus the format, the files ticked beside it, bit rate preset,
audio switch and last directory; source consent and overwrite authorization stay
transient. Project and undo history never store export state. An empty timeline shows
a catalog error and refuses start.

The settings are three tickable sections, as in the reference (CapCut 9.7.1, `.spikes/ui-reference/capcut-desktop-2026-11-14b/` row 12, frames 105340, 105528, 105532, 105549, 105736): Video, Audio and GIF. They tick independently and one Export writes each ticked output as its own file; at least one stays ticked, so the only ticked section's box is disabled with `Keep at least one output ticked.` on hover. The reference ticks Video and Audio by default; ours starts with Video alone, so a default export is still one file, and the video file always carries its own sound (the dialog has no silent-video choice, and `include_audio` stays a saved field for programmatic starts). The saved `ExportOptions` holds the primary output in `format` (the first ticked of Video, Audio, GIF: a video format, an audio format (`Mp3`, `Wav`, `M4a`, `Flac` or `AudioWebm`), or `Gif`) and the others in `also` (`AlsoOutputs`: an audio format and a GIF size, each `None` when its section is unticked). `ExportOptions::outputs` expands that into one single-output `ExportOptions` per file, in the order video, audio, GIF; scene and range are shared, the audio file drops every video field, and the GIF takes its own size and the Frame rate row's rate. Unticking Video greys its block, not hides it, and hovering a row names why (`Not used by audio-only output.`); ticking Video again returns to the video format it left. Choosing PNG sequence drops the Audio file (frames carry no sound) and disables the Audio box, but a GIF may still go beside it. This is the disabled-not-hidden rule of every silent or audio output: the encoder rows that do not apply (Bit rate, Codec, H.264 profile) are disabled for GIF, PNG sequence and audio-only output, and Codec and Profile offer Automatic only when enabled.

**Several outputs.** Several outputs are several engine export jobs, because one job writes one container (see the [engine README](../../../../../../crates/engine/README.md)). `start` prepares every output's job first, so an output the engine would refuse starts none, then submits the first. Each following job is submitted when the previous one completes, in `tick`, and the dialog shows one progress over all of them: `Exporting file N of M` over a bar that spans the whole group. A failure ends the group and removes what rendered; Cancel stops the running job and starts no later one; Retry reruns every output. Agents get the same ability through the command declaration, one `export` command per output (see [export admission](../../../../../../docs/product/agent-control/cli.md#export-admission)); the dialog does not go through commands, so there is no multi-output command. When every job has rendered there is one save step: the native picker asks for the place of the first file, and every other file of the group is written beside it under a name derived from the picked one (`<name>-audio.m4a`, `<name>.gif`, plus `-scene-N` for an all-scenes export; a picked name that already ends in its own `-scene-N` keeps that scene). Before anything is published, `save_to` refuses with `A file with one of these names already exists...` when any derived target exists, unless the one `Replace existing files` box (`export.replace`, shown while the group holds more than the first file) is ticked for the whole group; the picked file's own replacement is the native picker's confirmation. The rest of the group is then published by `tick`, one engine publication after the other, and one toast lists every saved file once the last is published; a ticked Captions file is written through the Captions panel's job, which marks it grouped (`grouped`) so its completion raises no `Done · N captions` toast of its own. A publication that fails stops the group; Retry asks for a place again for the file that failed. The footer estimate adds one size estimate per output; a GIF output uses the render crate's own GIF model, not the video bit-rate one.

The footer reads `Cancel` then `Export` (Export rightmost), as in the reference (frame 105340).

**Cover.** `cover.rs` draws the cover column. It shows the engine's stored 128 px thumbnail (`session.metadata().thumbnail`) at once and, in the background, renders the cover frame at the column's physical size (logical width x pixels per point, canvas aspect, `engine::cover::render` at `cover_tick` with the stored footage corrections) on a `wake` worker that wakes the UI when it finishes; the sharp frame then replaces the thumbnail. The render is kept, keyed by (project snapshot, `cover_tick`, pixel size), so reopening the dialog on the same project does not render again; a failed render is not retried for its key and the thumbnail stays. `ExportInspection::cover` reports the sized texture's pixel size once it is on screen. The `Edit cover` chip returns `DialogAction::EditCover`, and the frame applies it as `project.thumbnail` with the playhead as `tick` through the session's autosave (`Autosave::apply`), which stores the tick as the metadata `cover_tick` and recaptures the cover there; later refreshes keep that frame. The cover is metadata beside the project, outside content history, so there is no undo step; a failure shows on the editor's error line. The chip is disabled when the session has no store (a memory session) and while an export runs. Geometry: [UI editor](../../../../../../docs/product/ui/editor.md).

Video block order is Resolution, Bit rate, Codec, Format, Frame rate, then ours: H.264 profile and Custom size. Format lists MP4, MOV, WebM and, ours, PNG sequence. The reference's Remove watermark, Optical flow, Sync to space and Check copyright rows have no feature behind them and are not built. Color space is a read-only row after Frame rate (`Rec. 709 SDR`, the only space media writes; HDR is refused upstream), with the reason on hover.

Bit rate is a select with Lower, Recommended, Higher and Custom. The presets map to our encoder settings in `ExportQuality::value` (`dialog.rs`), read by `ExportOptions::settings`: Lower is quality 70 at 6,000,000 b/s, Recommended 85 at 12,000,000 b/s (the default), Higher 95 at 24,000,000 b/s. Custom shows a Kbps field under the select, starts at 12000 and takes 1 to 50000 Kbps (the reference's ceiling, read as 99999999 becoming 50000); it writes `OutputOptions::bit_rate` in b/s (Kbps times 1000), which replaces the preset's rate and keeps the last preset's quality scalar. A preset clears `bit_rate`. Only `OutputOptions` is a shared shape: the command declaration and the CLI carry `bit_rate` in b/s and no preset, so the dialog's preset names never leave the app and its saved `quality` is app preferences only (preferences version 18). The "?" beside the label shows the reference's hint about image quality.

Audio lists MP3, WAV, AAC (M4A) and FLAC in the reference's order, then, ours, Opus (WebM) (`AUDIO_FORMATS` in `settings.rs`); the Format row is live when Audio is ticked, otherwise it is disabled with `Tick Audio to use this.`. With Video ticked it picks the format of the separate audio file. An entry whose encoder the bundled FFmpeg lacks (`media::audio_export_container_available`) is not listed; MP3 needs libmp3lame, WAV (16-bit PCM) and FLAC use encoders built into FFmpeg. A saved `mp3`, `wav` or `flac` format is an additive `ExportFormat` value, so the preferences version does not change.

GIF is a section with Resolution only (240P, 320P, 640P, on the project's canvas ratio), replacing GIF as a Format choice. Ticking it sets 240P; its Resolution is disabled until it is ticked (`Tick GIF to use this.`). Alone it is the primary output and holds its size in `output.resolution` (the custom bit rate clears); beside another output its size is `also.gif` and the video keeps its own resolution and rate. The Colors and Dithering rows are gone (the reference has none); `OutputOptions::gif` keeps its defaults for dialog exports and stays settable through the command and the CLI. The Frame rate row stays live for GIF, ours, because `OutputOptions::check_gif` caps the rate; Export stays disabled with the shared range error when the resolution, rate or range breaks the GIF bounds. The reference marks 320P and 640P with Pro badges; we have no paid tiers, so none is drawn.

PNG sequence is a further Format row entry, always offered (it needs no encoder). Choosing it clears the custom bitrate, resets Codec and Profile to Automatic and turns an All-scenes choice into the active scene. The same disabled-with-reason rule applies as for GIF: the Audio box is disabled, Video's box too while it is the only one ticked, and the All-scenes option is disabled with `A PNG sequence holds one scene.` on hover. It adds no rows, so no new geometry is invented (see [UI editor](../../../../../../docs/product/ui/editor.md)). The job renders `<name>_%05d.png` frames into a temporary directory through the one export job; Save then opens the native save picker without a file-type filter, named after the project, and the picked name becomes the directory (and the frame prefix) that `submit_sequence_publication` promotes whole. Export stays disabled with the shared range error when the range holds more than 99,999 frames (`OutputOptions::check_sequence`). A finished temporary directory is removed like a finished file.

The footer's size estimate is the sum, over `ExportOptions::outputs` (one entry per file the export writes), of `ExportSettings::estimated_bytes` ([render export](../../../../../../crates/render/src/export/README.md#size-estimate)) over the settings that output's job would run with: `ExportOptions::settings` (the preset's quality and rate, audio switch), then `OutputOptions::apply` (resolution, frame rate, custom rate). The estimate is `seconds * (video + audio) / 8`; audio is the encoder's 192 kb/s when the audio track is included (always for audio-only output, which has no video), except WAV, which is its exact 16-bit PCM rate, and FLAC, which is estimated at that PCM size as an upper bound. Video is the encoder's target `bit_rate` for bit-rate encoders (Windows, Linux) and `max(bit_rate, pixels * fps * 0.3156 * 2^((quality - 85) / 15))` (`/ 10` above 85) for the macOS quality-mode encoder, chosen by `media::expected_rate_control` ([rows and measurements](../../../../../../crates/render/src/export/README.md#size-estimate)). A preset changes it through `ExportQuality::value` and nothing else (Lower 70/6 Mb/s, Recommended 85/12 Mb/s, Higher 95/24 Mb/s); `size_estimate_follows_the_encoder_settings_of_every_preset` checks all three under whichever mode the host selects.

All-scene output is rendered as one staged batch, then saved with distinct scene-id
filenames (see the one save step below). Cancelling the save picker keeps the finished
files; closing the dialog discards them.
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

The Captions section (`caption_outputs.rs`) is a tickable section with a Format select (SRT and TXT, the reference's two formats, then VTT as ours), enabled only when the project holds a caption item; the reference's Character encoding row is fixed to UTF-8 here. Ticked, it is `also.captions`: not an engine job, but a file of the same group, named `<picked name>.<ext>` beside the saved media. After the last media file is published the dialog hands the path to the frame (`Request::CaptionFile`), which writes it through the captions panel's own export, replacing a file the group's Replace choice already cleared. Four buttons below it (SRT, TXT, VTT, and the burned MP4 as ours) still save one caption output on its own: each click returns `DialogAction::Caption`, the frame raises `EditorAction::ExportCaption` and the native save picker answers with the path. The burned MP4 stays outside the group (it is a render of the video, not a text file). The settings sheet recomputes spoken-word warnings from the captured project, output choices and shared render state before submission. Safe reason labels never include caption text, and a warning keeps the ordinary base caption rather than blocking an otherwise valid export.
