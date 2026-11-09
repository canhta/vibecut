# Local face embeddings

`vision::TractFaceEmbedding::load(dir, threads, hooks)` loads the catalog's pinned SFace ONNX weights after size/SHA-256 verification; nothing downloads implicitly. The graph runs unmodified on the tract CPU runtime with 1..=4 private executor threads and one fixed-shape plan. `embed` takes an `IdentityFrame`: finite 112x112 CHW RGB pixels in 0..=255. Media owns five-point alignment; the graph owns centering and scaling. `FaceEmbedding` validates and normalizes its 128 finite elements and computes bounded cosine similarities. Debug is redacted and there is no serialization.

`TractVision::with_reidentification` attaches a loaded adapter; its model, alignment recipe, runtime and thread fingerprint then enter the vision bundle fingerprint and engine cache key. `VisionDetector::embedding` returns None when unavailable. YuNet `FaceBox::landmarks` keeps its five points in prepared pixels; missing landmarks stay explicit. Threshold decisions belong to engine.

SFace attribution and licence disposition: [SFACE.attribution](../../SFACE.attribution). Weights stay in the user cache, never bundled. `examples/face_embedding.rs` is an offline tensor smoke/latency and pair-score harness.

## Local vision detectors

`vision::VisionDetector` is a blocking, object-safe local inference seam. `vision::TractVision::load(VisionPaths, threads, capacity, hooks)` implements it with tract 0.23.9 (default `vision` feature); contracts and decision functions exist without the feature. No engine/media dependency.

Media supplies borrowed, finite prepared tensors through validated constructors:

- `FaceFrame`: `[1,3,384,640]` CHW BGR, values 0..=255, bottom/right letterbox. Returned boxes use prepared pixels, may extend past the frame and are not clipped. YuNet score threshold 0.6, IoU NMS threshold 0.3, top 5000 candidates, at most 256 boxes (excess is a typed failure).
- `SceneFrame`: `[1,3,224,224]` RGB, CLIP normalized; media owns resize/crop and mean/std. The two fixed CLIP prompts (see [models](../models/README.md)) are derived once per loaded bundle, kept in memory and fingerprinted; the towers and prompt plan are not retained. No text inference per frame.
- `SpeakerChunk`: 25..=75 frames at 25 fps, 112x112 grayscale 0..=255, and exactly four 13-coefficient MFCC frames per video frame (100 Hz), absolute values capped at 1e6. Media owns the crop/MFCC recipe; ai does no padding, tracking, decoding or timestamp conversion.

The adapter verifies and owns the exact bytes it parses, including external data files. It decodes from the verified slice without a second whole-file buffer and inlines external slices before ONNX loading; no path resolver or mutable mmap touches weights. For ASD it checks input names, f32 and ranks, clears input dimension annotations, output shapes and stale `value_info`, then binds concrete input facts without changing weights or operators.

A bounded LRU holds 3..=12 plans keyed by graph and external-data hashes, exact frame count, input dimensions, runtime revision and threads. Threads are 1..=4 on an instance-owned executor; global runtime settings are untouched. `fingerprint` binds weights, tokenizer, prompts, preprocessing/decision recipe, runtime and threads, and engine uses it in persistent analysis artifact keys. The cache holds only inference plans and fixed prompt vectors.

`faces`, `speaker` and `scene` observe `ExecutionHooks` at every load/compile/run boundary with `Stage::Vision` (engine maps it to its `Process` stage). A running tract call cannot be interrupted: it drains and its output is discarded if cancellation was requested. Error categories: size/hash `ModelIntegrity`, invalid inputs or output shapes `VisionShape`, ONNX/plan `VisionLoad`, nonfinite or zero-norm outputs `VisionNonFinite`, inference `Inference`. Tract panics are caught and reduced to these.

`aggregate_speakers` combines shot-local track chunk probabilities, averages overlap, smooths +/-2 observed frames and picks the highest track mean at >=0.4, else `NoVisibleSpeaker`; ties take the lowest track ID. Limits per shot: 64 tracks, 512 distinct chunks, 9000 frame coordinates. Duplicate chunk identities are rejected and gaps never filled. `SceneResult::unknown()` records absent classification; model errors propagate rather than becoming unknown. `ShotResult::new` validates bounded boxes, visible-speaker probability and scene consistency. Engine owns shot boundaries, track identity, cross-shot persistence and result caching.

Tests: `cargo test -p ai --test it vision::`. The ignored `real_model_parity_and_exact_tails` needs `VIBECUT_VISION_MODELS` (catalog-named YuNet/CLIP files plus six ASD files) and `VIBECUT_VISION_CORPUS` (private tensors and ORT references); it checks 30/30 face counts, 30/30 speaker shot verdicts across 121 chunks, 50/50 CLIP labels and every exact tail 37..=75. Windows execution is unverified.
