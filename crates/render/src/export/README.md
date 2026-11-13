# Export and publication

`export/` owns options, execution, destination and publication. `gif.rs` is the silent GIF pass.

## Options

`OutputOptions` is the versioned portable extension of export settings. Version 1 requires all fields and refuses unknown fields or another version. Defaults match the project canvas and frame rate, the full timeline and the active scene. Explicit output geometry is even, at least 2 pixels per edge and at most 64 Mi-pixels. Target bitrate is 1,000..=1,000,000,000 bits/s (refused for GIF). `gif` carries `max_colors` 2..=256 and `dither` none|bayer; it is required and used only by GIF. Optional rates use `FrameRate::COMMON`. The codec is automatic (H.264 for MP4/MOV, VP9 for WebM) and incompatible pairs refuse. H.264 profiles are automatic, Main or High, and an explicit profile always encodes.

GIF output (`ExportSettings::container` GIF, chosen by the `.gif` extension or the dialog format) is silent, always re-encoded and never stream-copied. It reuses the one frame producer and engine job: `gif_export` renders each frame exactly like the video pass and hands RGBA to [media's GIF encoder](../../../media/src/write/README.md#gif), which owns the palette, delay and loop rules and the bounds (palette 2..=256, 2..=4096 px per edge and 2^21 px, at most 50 fps, 60 s and 3000 frames). `OutputOptions::apply` refuses codec, profile and bitrate choices and out-of-bound geometry or rate; `check_gif` (called by the engine submit and the dialog) and `export_with_sources` refuse a range beyond the duration bound before any file is staged. Refusals are `Error::Media(MediaError::Gif(GifLimit))`. Audio is ignored for GIF, including source admission, and HDR sources refuse as for any encode.

Ranges are full, in/out markers or an explicit selected-clip bounding range. Marker endpoints snap to the nearest project frame (ties later), then refuse if negative, empty, reversed or beyond the scene. Each scene uses its own duration and the shared project canvas, rate and background. `ranges` validates every immutable snapshot before execution. `paths` names all-scene outputs `<stem>-scene-<id>.<extension>`. Export never changes project JSON or history.

## Execution

`export` uses `FileSources`; `export_with_sources` accepts an explicit source owner. Settings select range, canvas/crop, fps, bitrate/quality, hardware-preferred or platform software encoder, audio and overwrite policy. The initial encoder preference follows [media's platform default](../../../media/src/write/README.md#encoding).

`ExportSettings::container` defaults to MP4. MP4/MOV encode H.264 + AAC, WebM encodes VP9 + Opus and GIF encodes silent palette video. `VideoCodec::fits(container)` and `OutputOptions::profile_fits(profile, container)` are the one statement of that rule: `apply` refuses with them and a dialog offers choices with them. Output is chosen by the settings, never guessed from the filename. Auto copies only an admitted container video codec (H.264/HEVC/MPEG-4 for MP4/MOV; VP8/VP9/AV1 for WebM), otherwise it encodes, and forced copy refuses. Included audio always mixes and encodes for the target so mastering cannot be bypassed.

Optional `degraded_consent` binds an interactive caller's `allow_degraded` to the accepted typed asset, item, range and reason diagnostics. Any additional unavailable range or changed reason refuses. CLI callers keep an explicit whole-export `allow_degraded` switch.

Frames are sampled at range.start + n/fps and output timestamps start at zero. Exact non-keyframe cuts seek to a preceding keyframe and decode forward. Audio uses the preview's absolute sample grid. AAC delay and padding are FFmpeg-owned.

Auto copies unchanged video when a single visible source covers the output with no pixel changes. An opacity override (even a channel with no keys), a reflection channel or any of the five numeric transform channels, constant or animated, disables video copy; Auto encodes and forced copy refuses. Audio is still encoded from the mastered window. Only the first video and audio stream selected by decode is published. An accepted keyframe cut additionally requires exact start and end keys (or source EOF), regular requested fps and PTS==DTS. Reordered or open-GOP candidates refuse forced copy and Auto encodes. No requested boundary is silently moved. `HdrPolicy` requires passthrough for HDR, otherwise refuses. Resume returns a typed unsupported error; a partial movie is never a checkpoint.

## Publication

Project's `StreamingStage` owns same-directory private staging, byte and seek bounds, overwrite authorization and finalize/publish. Export validates aliases against source paths (including Unix hard links), flushes encoders, finishes mux and trailer, finalizes and syncs, releases working buffers, revalidates sources, then calls the publication fence. `ExecutionHooks::begin_publish` must atomically arbitrate cancellation for a concurrent owner. Engine reports Publishing and ignores a cancel arriving after its fence, so a committed file is reported Completed. Before the fence, cancellation or failure preserves the prior file. After publish there are no fallible progress callbacks or result allocations. Crash-orphan, replacement and filesystem guarantees belong to [project](../../../project/src/persistence/README.md#streaming-output).

`OutputOptions::caption_word_warnings` reads the exact selected scene snapshots and output geometry through the shared scene reader before export. It performs no decoding or publication. These warnings do not change SRT/VTT text or invent word timings, and a later per-file publication failure retains the existing partial-batch semantics.
