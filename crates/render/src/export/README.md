# Export and publication

`export/` execution, destination, publication.

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
[project](../../../project/README.md#streaming-output).
