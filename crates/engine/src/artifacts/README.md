# Artifacts

Interface: `Error` (re-exported as `engine::ArtifactError`), `Result`, `thumbnails`, `thumbnail_strip`, `snapshot`, `effect_thumbnails`, `chapter_sidecars`.

The private LUT job consumes the [durable copied-resource owner](../store/README.md#imported-lut-bundles), with a request fingerprint distinct from canonical content identity. The store returns its
LUT-owned result; this callback composes it with its own bounded JSON failure through the
Artifact result, preserving the actual typed cause. It returns at most 16 KiB of descriptor data through `JobOutput::Lut`; queue refusal retains no capture, CPU jobs run once and job output never changes the project. Cancellation/publication use the existing runner seal. Inspector grades are not library preview tiles; thumbnail requests carrying unresolved LUT references fail explicitly.

## Failures

Mixed artifact operations and their jobs return the Artifact result with only consumed
Kernel Edit, Runtime, Render, Media, AI, IO and Lut causes. Higher workflows compose it through
`engine::control::Error::Artifacts`; lower owners never name this feature. Foreign execution classifiers
are borrowed from their existing owners. Direct Runtime/Lut/AI policy delegates; foreign
render/media cancellation and semantic Failed envelopes remain ordinary opaque failures.

`EmptyProject` owns the snapshot-only `Project is empty` refusal and existing wire category,
with no source, transience or scheduling advice. Empty duration is checked before negative time.
Pure validation, grid/encoding and naming pieces return Kernel Edit results; pure cache
maintenance/open/key and manifest corruption readers return Runtime results. Publication
writers keep their actual caller error; an unrelated mixed callback is never normalized into
Artifact merely for submission.

## Import jobs and thumbnails

`thumbnails::thumbnail_key` binds the source content hash, requested frame position and output
edge. `ThumbnailCache::get_or_generate` decodes only on a miss; corrupt envelopes or payloads
count as misses and regenerate. `ThumbnailCache` keeps validated source probe metadata in the
same content-keyed envelope as pixels, so a warm image import reuses geometry before opening a
recovery decoder. It never caches machine capabilities.

`thumbnails::RetainedThumbnail` shares pixels through an `Arc` and holds the aggregate
`DecodedFrames` allowance until the last owner drops. It reserves four RGBA byte counts for
packed pixels, the egui CPU image, upload staging and the GPU texture, plus the retained probe
metadata allocation. App owners keep that handle while any texture is retained.

## Thumbnail strips

`thumbnail_strip::tile_interval(strip_height_px, pixels_per_second)` returns the output ticks
between two tiles: one 16:9 tile whose width is the filmstrip height times 16/9
(`TILE_ASPECT_RATIO`).

`ThumbnailStripRequest::new(asset, source_range, interval, max_edge)` samples a video or image
source on a grid anchored at source zero, so overlapping requests reuse the same
`thumbnails::thumbnail_key` entry per position. `submit` runs one local CPU
`GenerateClipThumbnails` job, opens the source once and decodes every missing position in one
sequential `media::thumbnails_at` pass, reserving 32 MiB decoded frames and a 64 MiB result. A
warm cache opens no source. `finish` validates the manifest (magic, count, requested positions,
per-frame geometry, 64 MiB) and publishes each frame through the thumbnail cache. A still image
has one frame at source zero and repeats it.

`thumbnail_strip::display(asset, item, visible, pixels_per_second, max_edge, cache)` is the
cache-only query: it derives one interval from the timeline zoom with
`tile_interval(STRIP_HEIGHT_PX, pixels_per_second)`, clamps the visible output range to the clip,
maps each tile's start through the item's trim and retime, floors it to the request grid and
returns `Ready(tiles)` with one `StripTile { output, at, image }` per tile, `Pending` when any
frame is missing, `Failed` when a cached frame cannot be decoded, or `Unsupported` when the
source has no video stream. It never decodes media and never blocks on a job. Repeated source
positions share one retained image.

Limits: one request or visible range 1..=128 tiles, a positive interval and `max_edge` 1..=320
(`media::THUMBNAIL_MAX_EDGE`). A range outside the clip, a mismatched asset, a negative source
range or a hostile tile count is `Invalid`. Cancellation publishes nothing and leaves the
previous frames intact; a changed source content hash misses every old entry.

`media.generate_clip_thumbnails {item, visible, pixels_per_second}` runs the same
`ThumbnailStripRequest::for_clip` at the maximum edge, and `job_status` publishes the completed
frames through the attached thumbnail cache; a warm request opens no source.

## Frame snapshots

`preview.snapshot {time, path}` renders one frame at the project canvas size through the
production compositor and publishes a PNG without editing the Session or undo history. It is
independent of app transport, zoom and transient preview state. A playhead at or past the end
clamps to the last frame; negative times refuse and an empty project refuses with the typed
`Project is empty` error.

`snapshot::default_name` returns `<project>-<timecode>.png` using time's shared DF/NDF formatter
(DF at 29.97/59.94, NDF otherwise), replaces filesystem-reserved characters with `-`, and falls
back to `snapshot` when the project name sanitizes empty. `snapshot::image` returns the same PNG
bytes without publishing, for the desktop Copy action.

Publication uses the [shared publication owner](../runtime/staged_publication/README.md#completed-export-publication).
The desktop and CLI register active project, metadata and recovery paths with
`Session::protect_paths`. `snapshot::save` and `save_with_budget` replace the destination for
trusted desktop Save confirmation; control requests default to atomic no-clobber unless their
trusted `Policy::snapshot_overwrite` grants replacement (the CLI's `--allow-overwrite`). Limits:
`MAX_SNAPSHOT_PNG_BYTES` 32 MiB and `MAX_SNAPSHOT_MEMORY` 1 GiB.

## Chapter sidecars

`chapter_sidecars::write` takes a sidecar stem, a validated `ai::chapters::ChapterDocument`, the
captured project, protected active project and metadata paths, caller hooks and overwrite
policy. It derives `<stem>.txt` and `<stem>.json` without touching the extension inside a native
name, serializes both formats before admission, preflights both destinations against project
sources and protected paths, then publishes each through the shared publication owner. A source
or project alias on either file publishes nothing, and the project is not mutated.

Publication is per file: after a failure the previous destination survives and no staging
remains; a failure while publishing the second file leaves the first complete file and the prior
second file, never a partial file. `sidecar_paths` exposes the derived names for preflight
presentation.

`chapter_sidecars::write_to_directory` is the registered `chapters.export_sidecars {dir,
document}` operation: it derives the stem from the project name with the shared portable
file-name owner, refuses a missing directory or either existing file before writing, and always
publishes with atomic no-clobber. The document is the possibly edited `ChapterDocument` returned
by `chapters.generate`, so the exported title, description and chapter titles are exactly the
reviewed text. The CLI command uses the same function; the active project and metadata paths are
passed as protected paths.

## Effect thumbnails

`effect_thumbnails::EffectThumbnails` schedules device-injected `EffectThumbnail` jobs through
the runner. Callers supply an accounted, downsampled working-space source image and registered
an `EffectInstance`, owned `Animations` and nonnegative item-local `Tick`; negative time
refuses with Invalid. The shared evaluator receives those exact inputs with
fine=false, because a thumbnail is interactive preview; that may lower sample
counts but never geometry, colour mapping or which pixels are affected
([`EffectFrame::fine`](../../../render/src/scene/effects.rs)). Malformed animated values fail through its typed admission. The result is straight
sRGB RGBA8 at the input dimensions. `MAX_THUMBNAIL_EDGE`
bounds the edge and `MAX_THUMBNAILS` the cache, charged to the runner's shared byte budget. The
cache belongs to one adapter lifetime. Version3 keys bind source pixels/geometry, the full
instance (including its animation-path ID), animation data and local time. Animation identity
streams into a hash without an intermediate serialized buffer. Consumers own cadence;
the cache has no clock, fallback version or persisted migration.

Work and readback run outside cache locks. Thumbnails use render's shared `apply_effect`
executor: declared unsupported passes run on CPU and adapter `Unsupported` retries only
that run on CPU. Other typed failures, including budget, cancellation and invalid data,
propagate without fallback. The owned source copy is reserved through the job hooks.
Cancellation before the publication fence never caches an unfinished result; late
cancellation cannot undo publication. Missing or oversized sources, invalid params,
budget pressure, poisoned state and runner queue limits refuse.

Effect thumbnails require explicit `EffectGeometry` matching source dimensions;
mismatches refuse before hashing/jobs. Both positive finite gains key version3
by f64 bits. Unknown author geometry has no default. Existing cache limits apply.
