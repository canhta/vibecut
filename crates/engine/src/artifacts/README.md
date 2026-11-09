# Artifacts

Interface: `thumbnails`, `thumbnail_strip`, `snapshot`, `effect_thumbnails`, `chapter_sidecars`.

## Import jobs and thumbnails

The desktop import composer submits `JobKind::Probe` for canonicalize, probe, hash and capability checks, publishes each asset before the batch ends, then submits `JobKind::Thumbnail` for video and image assets. Both use the runner, progress stages (`Probe`, `Thumbnail`), cancellation and the aggregate budget; the UI keeps completed assets on cancel or failure.

`thumbnails::thumbnail_key` binds the source content hash, requested frame position and output edge. `ThumbnailCache::get_or_generate` decodes only on a miss; corrupt envelopes or payloads count as misses and regenerate. `ThumbnailCache` keeps validated source probe metadata in the same content-keyed envelope as pixels, so a warm image import reuses geometry before opening a recovery decoder. It never caches machine capabilities.

`thumbnails::RetainedThumbnail` shares pixels through an Arc and holds the aggregate `DecodedFrames` allowance until the last owner drops. It reserves four RGBA byte counts for packed pixels, egui CPU image, upload staging and GPU texture, plus the retained probe metadata allocation. App owners keep that handle while any texture is retained.

## Thumbnail strips

`thumbnail_strip::tile_interval(strip_height_px, pixels_per_second)` returns the output ticks between two tiles: one 16:9 tile, whose width is the filmstrip height times 16/9 (the implementation reference's `THUMBNAIL_ASPECT_RATIO = 16 / 9` in `apps/web/src/timeline/components/timeline-element.tsx:97`, used at `:1108`). The measured editor uses a 39 px filmstrip at the ruler's 71.5 px/s, about 0.97 s per tile.

`ThumbnailStripRequest::new(asset, source_range, interval, max_edge)` samples a video or image source on a grid anchored at source zero, so overlapping requests reuse the same `thumbnails::thumbnail_key` entry per position. `submit` runs one local CPU `GenerateClipThumbnails` job, opens the source once and decodes every missing position in one sequential `media::thumbnails_at` pass (no second decoder or key builder) and reserves 32 MiB decoded frames and a 64 MiB result. A warm cache opens no source. `finish` validates the manifest (magic, count, requested positions, per-frame geometry, 64 MiB) and publishes each frame through the existing thumbnail cache. A still image has one frame at source zero and repeats it.

`thumbnail_strip::display(asset, item, visible, interval, max_edge, cache)` is the cache-only query: it clamps the visible output range to the clip, maps each tile's start through `project::Item::source_time_at` (trim and retime), floors it to the request grid and returns `Ready(tiles)` with one `StripTile { output, at, image }` per tile, `Pending` when any frame is missing, `Failed` when a cached frame cannot be decoded, or `Unsupported` when the source has no video stream. It never decodes media and never blocks on a job. Repeated source positions share one retained image.

Limits: one request or visible range 1..=128 tiles, a positive interval and max edge 1..=320. A range outside the clip, a mismatched asset, a negative source range or a hostile tile count is `Invalid`. Cancellation publishes nothing and leaves the previous frames intact; a changed source content hash misses every old entry.

`media.generate_clip_thumbnails {item, visible}` maps the clip's visible output range through trim and retime with the same private grid helper as `display`, then runs `ThumbnailStripRequest::for_clip` on `GENERATION_INTERVAL` (one tile per output second) at the 320 px edge. `job_status` publishes the completed frames through the attached thumbnail cache; a warm request opens no source.

## Frame snapshots

`preview.snapshot {time, path}` renders one frame at the project canvas size through the production compositor and publishes a PNG without editing the Session or undo history. It is independent of app transport, zoom and transient preview state. A playhead at or past the end clamps to the last frame; negative times refuse. An empty project refuses with the typed `Project is empty` error.

`snapshot::default_name` returns `<project>-<timecode>.png` using time's shared DF/NDF formatter (DF at 29.97/59.94, NDF otherwise), replaces filesystem-reserved characters with `-`, and falls back to `snapshot` when the project name sanitizes empty. `snapshot::image` returns the same PNG bytes without publishing, for the desktop Copy action.

Publication uses the [shared publication owner](../runtime/staged_publication/README.md#completed-export-publication). The desktop and CLI register active project, metadata and recovery paths with `Session::protect_paths`. `snapshot::save` and `save_with_budget` replace the destination for trusted desktop Save confirmation; control requests default to atomic no-clobber unless their trusted `Policy::snapshot_overwrite` grants replacement (the CLI's `--allow-overwrite`).

## Chapter sidecars

`chapter_sidecars::write` takes a sidecar stem, a validated `ai::chapters::ChapterDocument`, the captured project, protected active project/metadata paths, caller hooks and overwrite policy. It derives `<stem>.txt` and `<stem>.json` without touching the extension inside a native name, serializes both formats before admission, preflights both destinations against project sources and protected paths, then publishes each through the [shared publication owner](../runtime/staged_publication/README.md#completed-export-publication). A source or project alias on either file publishes nothing.

Publication is per file: after a failure the previous destination survives and no staging remains; a failure while publishing the second file leaves the first complete file and the prior second file, never a partial file. The project is not mutated. `sidecar_paths` exposes the derived names for preflight presentation.

## Effect thumbnails

`effect_thumbnails::EffectThumbnails` schedules device-injected `EffectThumbnail` jobs through the runner. The desktop's `GpuEffects::thumbnails` constructs it using the same offscreen GPU adapter as preview. Callers supply an accounted, downsampled working-space source image and registered `EffectParams`; the result is straight sRGB RGBA8 at the input dimensions. Maximum edge is 320 pixels, with 32 LRU entries (at most 12.5 MiB), charged to the runner's shared byte budget. The cache belongs to one adapter lifetime; keys hash source pixels, geometry and canonical typed params. Consumers request on source or param changes; there is no timer or fixed cadence.

Work and readback run outside cache locks. Cancellation before the publication fence never caches an unfinished result. Typed GPU failure propagates without CPU fallback. Missing or oversized sources, invalid params, budget pressure, poisoned state and runner queue limits refuse.
