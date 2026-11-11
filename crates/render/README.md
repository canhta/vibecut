# render

CPU reference compositor, shared sample-window mixer and bounded MP4/MOV/WebM export.
It consumes immutable project snapshots and renders and mixes one active-scene snapshot at a time. Media owns codecs, conversion and mux, text owns shaping and rasterization, project owns atomic publication, engine owns scheduling and aggregate byte policy. No UI, device or persisted render state.

`ExportSettings::from_project` uses media's platform encoder default (software on Windows). Hardware decoding stays enabled. Requested quality and bitrate are retained. Encoder selection, rate-control fallback and typed errors follow the [media contract](../media/src/write/README.md#encoding).

| Module | Responsibility | Interface items | Contract |
|---|---|---|---|
| `scene/` | Evaluates project items into one RGBA frame. | `Renderer`, `RenderSettings`, `Rect`, `Placement`, `Fit`, `RasterCacheStats`, `viewport` | [Working pixels and scene](src/scene/README.md) |
| `scene/effects/` | Evaluates effect passes and owns the CPU reference table and imported cubes. | `EffectPass`, `EffectCompositor`, `CpuEffects`, `EffectFrame`, `evaluator`, `evaluate_effect`, `apply_effect`, `content_time`, `lut` | [Effect passes](src/scene/effects/README.md) |
| `audio/` | Owns every project audio stage from mix to limiter. | `AudioBlock`, `AudioMixer`, `mix_audio`, `measure_loudness` | [Audio clock and headroom](src/audio/README.md) |
| `export/` | Runs a staged project export through to publication. | `OutputOptions`, `RangeChoice`, `SceneChoice`, `VideoCodec`, `export`, `export_with_sources`, `validate_output_path` | [Export and publication](src/export/README.md) |
| `image` | Owns the RGBA frame buffer and its bounded allocation. | `Image` | [source](src/image.rs) |
| `sources` | Provides decoded video and audio samples. | `SourceProvider`, `FileSources` | [Sources, bounds and cancellation](src/sources/README.md) |

The crate root re-exports the public names; module paths are private except the scene helpers re-exported at the root.
