# Export and publication

`export/` options, execution, destination, publication.

`OutputOptions` is the versioned portable extension of export settings. Version 1
requires all fields and refuses unknown fields or another version. Defaults match
the project canvas and frame rate, the full timeline and active scene. Explicit
output geometry must be even, at least 2 pixels per edge and at most 64 Mi-pixels;
target bitrate is 1,000..=1,000,000,000 bits/s. Optional rates use `FrameRate::COMMON`.
The codec is automatic, H.264 for MP4/MOV or VP9 for WebM; incompatible pairs
refuse. H.264 profiles are automatic, Main or High; an explicit profile always
encodes. Quality presets and include-audio remain the caller's existing settings.

Ranges are full, in/out markers or an explicit selected-clip bounding range.
Marker endpoints are ticks snapped to the nearest project frame (ties later),
then refused if negative, empty, reversed or beyond the scene. Each scene uses
its own duration and shared project canvas/rate/background. `ranges` validates
every immutable snapshot before execution; `paths` gives all-scene outputs stable
`<stem>-scene-<id>.<extension>` names. Export never changes project JSON or history.

`export` uses `FileSources`; `export_with_sources` accepts an explicit source
owner. Settings select exact range, canvas/crop, fps, bitrate/quality,
hardware-preferred or platform software encoder, audio and overwrite policy.
The initial encoder preference follows [media's platform default](../../../media/src/encode/README.md#input-and-quality-boundaries).
`ExportSettings::container` defaults to MP4. MP4/MOV encode H.264 + AAC;
WebM encodes VP9 + Opus using media's existing encoders. Output is selected by the
settings, never guessed from the filename. Auto copies only an admitted container
video codec (H.264/HEVC/MPEG-4 for MP4/MOV; VP8/VP9/AV1 for WebM); otherwise it
encodes, and forced copy refuses. Included audio always mixes and encodes for the target so mastering cannot be bypassed.
Optional `degraded_consent` binds an interactive caller's `allow_degraded` to the
accepted typed asset/item/range/reason diagnostics. Any additional unavailable
range or changed reason refuses, so a retry cannot silently expand that consent.
CLI callers can retain their explicit whole-export `allow_degraded` switch.

Frames are sampled at range.start + n/fps; output timestamps start at zero. Exact
non-keyframe cuts seek to a preceding keyframe and decode forward. Audio uses the
same absolute sample grid as preview. AAC delay/padding remain FFmpeg-owned.

Auto copies unchanged video when a single visible source covers the output with
no pixel changes. An opacity override (including a channel with no keys) disables
video copy; the five transform channels do likewise for both constant and animated
values. Auto encodes these edits and forced stream copy refuses them. It encodes
the exact mastered audio window while retaining video packet bytes. Only the first
video/audio stream selected by decode is published. An explicitly accepted
keyframe cut additionally requires exact start/end keys (or source EOF), regular
requested fps and PTS==DTS; reordered/open-GOP candidates refuse forced copy and
Auto encodes instead. No requested boundary is silently moved. `HdrPolicy`
requires passthrough for HDR, otherwise refuses. Resume always returns a typed
unsupported error; a partial movie is never a reusable checkpoint.

Project's `StreamingStage` owns same-directory private staging, byte/seek bounds,
overwrite authorization and finalize/publish. Export validates aliases against
source paths (including Unix hard links), flushes encoders, finishes mux/trailer,
finalizes/syncs, releases working buffers, revalidates sources, then calls the
publication fence. `ExecutionHooks::begin_publish` must atomically arbitrate
cancellation for a concurrent owner. Engine reports Publishing and ignores a
cancel arriving after its fence, so a committed file is reported Completed.
Before the fence cancellation/failure preserves the prior file. After the atomic
publish there are no fallible progress callbacks or result allocations.
Crash-orphan, replacement and filesystem guarantees belong to
[project](../../../project/src/persistence/README.md#streaming-output).
