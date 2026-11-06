# render

CPU reference compositor, shared sample-window mixer and bounded MP4/MOV/WebM export.
Consumes immutable project snapshots and renders/mixes only their active scene; media owns codecs/conversion/mux, text owns
shaping/rasterization, project owns atomic publication, engine owns scheduling and
aggregate byte policy. No UI, device or persisted render state; it evaluates
project-owned keyframed opacity.

`ExportSettings::from_project` uses media's platform encoder default: software
encoding on Windows pending hardware picture-integrity verification.
Hardware decoding remains enabled; requested quality and bitrate are retained.

| Module | Responsibility | Interface items | Contract |
|---|---|---|---|
| `scene/` | Evaluates project items into one RGBA frame. | `Renderer`, `RenderSettings`, `Rect`, `Placement`, `Fit`, `RasterCacheStats`, `viewport` | [Working pixels and scene](src/scene/README.md) |
| `audio/` | Owns every project audio stage from mix to limiter. | `AudioBlock`, `AudioMixer`, `mix_audio`, `measure_loudness` | [Audio clock and headroom](src/audio/README.md) |
| `export/` | Runs a staged project export through to publication. | `OutputOptions`, `RangeChoice`, `SceneChoice`, `VideoCodec`, `export`, `export_with_sources`, `validate_output_path` | [Export and publication](src/export/README.md) |
| `image` | Owns the RGBA frame buffer and its bounded allocation. | `Image` | [source](src/image.rs) |
| `sources` | Provides decoded video and audio samples. | `SourceProvider`, `FileSources` | [Sources, bounds and cancellation](src/sources/README.md) |

The crate root re-exports the same public names and module paths.

## Evidence

`tests/it/contracts.rs`, `goldens.rs` and `media_pipeline.rs` own deterministic local
scenes and final encoded/decode comparisons, exact PCM partitions, cuts, crop,
HDR/display metadata, offline admission, byte pressure and failure boundaries.
Engine tests submit real jobs and exercise cancellation fences and worker unwind.
Required-oracle mode fails on absent ffmpeg/ffprobe; a found but broken tool always
fails. External FFmpeg shares codec algorithms, so it is a pipeline oracle rather
than an independent codec implementation. Fixtures are generated under target;
none are distributed. Windows runtime and QuickTime playback remain unverified.

The software H.264 failure is corrected: the platform software encoder
exists and the rejected QSCALE setting caused the earlier open failure. Encoder
selection, rate-control fallback and typed errors follow the
[media contract](../media/src/encode/README.md#input-and-quality-boundaries).
Observed on Apple M1 Pro/macOS 27.2: the real software-export regression produces
30 H.264 frames plus AAC with maximum decoded color-patch channel error <=6.
Intel/Rosetta, VM-only fallback and Windows runtime remain unverified.

Observed on Apple M1 Pro, 16 GiB, macOS 27.2, release profile:
nine alternating warm fresh-process pairs through the production engine benchmark
exported the exact 60 s/1,800-frame 1080p fixture to 1080x1920 in 15.99–20.66 s
process wall, versus the matching external crop/Lanczos hardware-encode command's
9.29–9.55 s. The median paired wall ratio is 1.863734x. The lowest-prepair-load
run (one-minute host load 10.19) takes 16.09 s / 15.987541 s internally, versus
FFmpeg 9.55 s (1.684817x); three individual pairs exceed 2x under shared load.
The contemporaneous exact-base renderer takes 53.98 s. Current peak RSS ranges
266,452,992–275,103,744 bytes; sampled aggregate charged peak is 560,156,672.
This trades bounded worker stacks/native conversion allowances and fixed packing
tables for throughput; the original minimum-budget export still works by reducing
concurrency. Every one of the nine outputs matches all 1,800 decoded frames
exactly (SSIM 1, PSNR infinity); external-reference SSIM is 0.994169 and PSNR
38.998035 dB. Preview/export pixels, fractional crops and linear premultiplied
math are shared. These observations cover this fixture and host, not other
canvases, hardware, formats or guaranteed latency under concurrent workloads.

