# render

CPU reference compositor, shared sample-window mixer and bounded MP4/MOV/WebM export.
It consumes immutable project snapshots and renders and mixes one active-scene snapshot at a time. Media owns codecs, conversion and mux, text owns shaping and rasterization, project owns atomic publication, engine owns scheduling and aggregate byte policy. No UI, device or persisted render state.

`ExportSettings::from_project` uses media's platform encoder default (software on Windows). Hardware decoding stays enabled. Requested quality and bitrate are retained. Encoder selection, rate-control fallback and typed errors follow the [media contract](../media/src/write/README.md#encoding).

| Module | Responsibility | Interface items | Contract |
|---|---|---|---|
| `scene/` | Evaluates project items into one RGBA frame. | `Renderer`, `RenderSettings`, `Rect`, `Placement`, `Fit`, `RasterCacheStats`, `viewport` | [Working pixels and scene](src/scene/README.md) |
| `scene/effects/` | Evaluates effect passes and owns the CPU reference table and imported cubes. | `EffectPass`, `EffectCompositor`, `CpuEffects`, `EffectFrame`, `evaluator`, `evaluate_effect`, `apply_effect`, `content_time`, `lut` | [Effect passes](src/scene/effects/README.md) |
| `scene/sticker/` | Rasterises bundled flag and shape stickers. | `rasterize_sticker`, `StickerRaster`, `StickerError`, `FLAGS_PROVIDER`, `SHAPES_PROVIDER`, `MAX_STICKER_RASTER_SIDE` | [Sticker pixels](src/scene/sticker/README.md) |
| `scene/transitions/` | Implements the ported transition kinds over the working frame. | `thumbnail::ThumbnailCache` | [Transition kinds](src/scene/transitions/README.md) |
| `audio/` | Owns every project audio stage from mix to limiter. | `AudioBlock`, `AudioMixer`, `mix_audio`, `measure_loudness` | [Audio clock and headroom](src/audio/README.md) |
| `audio/cleanup/` | Runs an item's source cleanup chain with filter state carried across contiguous blocks. | `CleanupChain` | [Source cleanup](src/audio/cleanup/README.md) |
| `export/` | Runs a staged project export through to publication. | `OutputOptions`, `RangeChoice`, `SceneChoice`, `VideoCodec`, `export`, `export_with_sources`, `validate_output_path` | [Export and publication](src/export/README.md) |
| `image` | Owns the RGBA frame buffer and its bounded allocation. | `Image` | [source](src/image.rs) |
| `sources` | Provides decoded video and audio samples. | `SourceProvider`, `FileSources` | [Sources, bounds and cancellation](src/sources/README.md) |

The crate root re-exports the public names; module paths are private except the scene helpers re-exported at the root.
