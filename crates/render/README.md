# render

CPU reference compositor, shared sample-window mixer and bounded MP4/MOV/WebM export.
It consumes immutable project snapshots and renders and mixes only the active scene. Media owns codecs, conversion and mux, text owns shaping and rasterization, project owns atomic publication, engine owns scheduling and aggregate byte policy. No UI, device or persisted render state.

`ExportSettings::from_project` uses media's platform encoder default (software on Windows). Hardware decoding stays enabled. Requested quality and bitrate are retained. Encoder selection, rate-control fallback and typed errors follow the [media contract](../media/src/write/README.md#encoding).

| Module | Responsibility | Interface items | Contract |
|---|---|---|---|
| `scene/` | Evaluates project items into one RGBA frame. | `Renderer`, `RenderSettings`, `Rect`, `Placement`, `Fit`, `RasterCacheStats`, `viewport` | [Working pixels and scene](src/scene/README.md) |
| `audio/` | Owns every project audio stage from mix to limiter. | `AudioBlock`, `AudioMixer`, `mix_audio`, `measure_loudness` | [Audio clock and headroom](src/audio/README.md) |
| `export/` | Runs a staged project export through to publication. | `OutputOptions`, `RangeChoice`, `SceneChoice`, `VideoCodec`, `export`, `export_with_sources`, `validate_output_path` | [Export and publication](src/export/README.md) |
| `image` | Owns the RGBA frame buffer and its bounded allocation. | `Image` | [source](src/image.rs) |
| `sources` | Provides decoded video and audio samples. | `SourceProvider`, `FileSources` | [Sources, bounds and cancellation](src/sources/README.md) |

The crate root re-exports the same public names and module paths.

## Evidence

Tests under `tests/it/` (`contracts.rs`, `goldens.rs`, `media_pipeline.rs`) own deterministic scenes, final encode/decode comparisons, exact PCM partitions, offline admission, byte pressure and failure boundaries. Engine tests exercise cancellation fences and worker unwind.

Required-oracle mode fails on absent ffmpeg/ffprobe. A found but broken tool always fails. FFmpeg shares codec algorithms, so it is a pipeline oracle, not an independent codec. Fixtures are generated under target and not distributed.

Measured on Apple M1 Pro, macOS 27.2, release profile, with a 60 s 1080p 1,800-frame fixture exported to 1080x1920: 15.99 to 20.66 s wall against 9.29 to 9.55 s for the matching external crop/Lanczos hardware-encode command (median ratio 1.86). Peak RSS is 266 to 275 MB. Every output matches all decoded frames exactly. The result holds for this fixture and host only. Windows runtime, QuickTime playback, Intel/Rosetta and VM-only fallback are unverified.
