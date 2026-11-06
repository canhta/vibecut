# Artifacts

Interface: `thumbnails`, `snapshot`, `effect_thumbnails`.

## Import jobs and thumbnails

The desktop import composer submits `JobKind::Probe` for canonicalize/probe/hash
and capability checks, publishes each asset before the batch ends, then submits
`JobKind::Thumbnail` for video/image assets. Both use the existing runner,
progress stages (`Probe`, `Thumbnail`), cancellation and the aggregate budget;
the UI keeps completed assets on cancel or failure. `thumbnails::thumbnail_key`
binds the source content hash, requested frame position and output edge through
the one artifact-cache policy. `ThumbnailCache::get_or_generate` decodes only on
a miss; corrupt envelopes or payloads count as misses and regenerate. The CLI's
`probe` command prints the same metadata and capability matrix, while the app
maps capabilities to its localized `probing`/`ready`/`unsupported` badges and
keeps a placeholder when thumbnail generation fails.

`thumbnails::RetainedThumbnail` shares pixels through an Arc and holds the
aggregate `DecodedFrames` allowance until the last owner drops. It reserves
four RGBA byte counts for packed pixels, egui CPU image, upload staging and GPU
texture, plus the retained probe metadata allocation; app owners keep that
handle while any texture is retained. Geometry follows pixels in memory too,
so disk eviction or a session-only cache cannot trigger recovery decode on a hit.
`ThumbnailCache` keeps validated source probe metadata in the same content-keyed
envelope as pixels, so eviction cannot leave a frame hit requiring recovery
decode. It never caches machine capabilities. A warm image import can reuse
geometry before opening a recovery decoder. Cache payloads remain disposable.

## Frame snapshots

`preview.snapshot {time, path}` renders one frame at the project canvas size through
the production compositor and publishes a PNG without editing the Session or undo
history. It is independent of app transport, zoom and transient preview state, so
the same request is executable from the CLI. A playhead at or past the end clamps to
the last frame; negative times refuse. An empty project refuses with the typed
`Project is empty` error.

`snapshot::default_name` returns `<project>-<timecode>.png` using time's shared
DF/NDF formatter (DF at 29.97/59.94, NDF otherwise), replaces filesystem-reserved
characters with `-`, and falls back to `snapshot` when the project name sanitizes
empty. `snapshot::image` returns the same PNG bytes without publishing, for the
desktop Copy action.

Publication uses the [shared publication owner](../runtime/staged_publication/README.md#completed-export-publication).
The desktop and CLI register active project, metadata and recovery paths with
`Session::protect_paths`. `snapshot::save` and `save_with_budget` retain replacement
semantics for trusted desktop Save confirmation; control requests default to atomic
no-clobber unless their trusted `Policy::snapshot_overwrite` grants replacement
(the CLI's `--allow-overwrite`). The request remains `preview.snapshot {time, path}`.
External processes racing protected-path replacement retain the atomic owner's
documented limits.

## Effect thumbnails

`effect_thumbnails::EffectThumbnails` schedules device-injected `EffectThumbnail`
jobs through the existing runner. The desktop's `GpuEffects::thumbnails` constructs
it using the same offscreen GPU adapter as preview. Callers supply an accounted,
downsampled working-space source image and registered `EffectParams`; the result
is straight sRGB RGBA8 at the input dimensions. Maximum edge is 320 pixels, with
32 LRU entries (at most 12.5 MiB), charged to the same shared byte budget as the
runner. The cache belongs to one adapter lifetime; keys hash source pixels,
geometry and canonical typed params. Consumers request on source/param changes;
there is no timer or fixed cadence. Tiles and background blur use the same job.
Cache hits retain job/cancellation semantics. Work and readback run outside cache
locks; cancellation before the publication fence never caches an unfinished result.
Typed GPU failure propagates without CPU fallback. Missing/oversized sources,
invalid params, budget pressure, poisoned state and runner queue limits refuse.
The UI and tile source selection are separate consumers of this contract.
