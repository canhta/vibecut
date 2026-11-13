# Local face embeddings

`vision::TractFaceEmbedding::load(dir, threads, hooks)` loads the catalog's pinned SFace ONNX weights after size/SHA-256 verification; nothing downloads implicitly. The graph runs unmodified on the tract CPU runtime with 1..=4 private executor threads and one fixed-shape plan. `embed` takes an `IdentityFrame`: finite 112x112 CHW RGB pixels in 0..=255. Media owns five-point alignment; the graph owns centering and scaling. `FaceEmbedding` validates and normalizes its 128 finite elements and computes bounded cosine similarities. Debug is redacted and there is no serialization.

`TractVision::with_reidentification` attaches a loaded adapter; its model, alignment recipe, runtime and thread fingerprint then enter the vision bundle fingerprint and engine cache key. `VisionDetector::embedding` returns None when unavailable. YuNet `FaceBox::landmarks` keeps its five points in prepared pixels; missing landmarks stay explicit. Threshold decisions belong to engine.

SFace attribution and licence disposition: [SFACE.attribution](../../SFACE.attribution). Weights stay in the user cache, never bundled.

## Person matte

`vision::TractSegmentation::load(dir, threads, hooks)` loads the catalog's pinned `pphumanseg-2023mar` weights ([card, licence, shapes](../foundation/models/README.md)) after size/SHA-256 verification, with one fixed `[1,3,192,192]` plan on the same tract runtime and error categories as the detectors; nothing downloads implicitly. `matte(PersonFrame, hooks)` takes a finite CHW RGB 192x192 frame, 0..=255, which media prepares by stretching the whole frame (`VisionFrame::person_tensor`, bilinear, aspect not kept), normalizes it and returns a `PersonMatte`: the class-1 softmax plane, finite and within 0..=1 (1e-3 softmax slack is clamped, anything else is `VisionShape`). The same weights, runtime and input give the same plane; the fingerprint binds the weights, `PERSON_RECIPE`, runtime and threads for engine's cache key. Debug is redacted.

`PersonMatte::quantized()` is the plane as one byte per cell (`round(p * 255)`), the form the engine stores. The matte carries no time and no frame size: engine owns per-frame scheduling and the cache ([footage correction](../../../engine/src/analysis/correction/README.md#person-matte)), and render owns the feather and the application at frame size ([person matte](../../../render/src/scene/correction/README.md#person-matte)), where the IoU fixtures live. Plane and frame validation and the runtime's load checks: `tests/it/matte.rs` and its ignored real-weights run.

## Local vision detectors

`vision::VisionDetector` is a blocking, object-safe local inference seam. `vision::TractVision::load(VisionPaths, threads, capacity, hooks)` implements it with tract 0.23.9 (default `vision` feature); contracts and decision functions exist without the feature. No engine/media dependency. `TractVision::load_faces(yunet, threads, capacity, hooks)` loads only the pinned YuNet graph for consumers that need face boxes alone (auto reframe); `speaker` and `scene` then refuse with `VisionLoad`, and the fingerprint names exactly the graphs loaded.

Media supplies borrowed, finite prepared tensors through validated constructors:

- `FaceFrame`: `[1,3,384,640]` CHW BGR, values 0..=255, bottom/right letterbox. Returned boxes use prepared pixels, may extend past the frame and are not clipped. YuNet score threshold 0.6, IoU NMS threshold 0.3, top 5000 candidates, at most 256 boxes (excess is a typed failure).
- `SceneFrame`: `[1,3,224,224]` RGB, CLIP normalized; media owns resize/crop and mean/std. The two fixed CLIP prompts (see [models](../foundation/models/README.md)) are derived once per loaded bundle, kept in memory and fingerprinted; the towers and prompt plan are not retained. No text inference per frame.
- `SpeakerChunk`: 25..=75 frames at 25 fps, 112x112 grayscale 0..=255, and exactly four 13-coefficient MFCC frames per video frame (100 Hz), absolute values capped at 1e6. Media owns the crop/MFCC recipe; ai does no padding, tracking, decoding or timestamp conversion.

The adapter verifies and owns the exact bytes it parses, including external data files. It decodes from the verified slice without a second whole-file buffer and inlines external slices before ONNX loading; no path resolver or mutable mmap touches weights. For ASD it checks input names, f32 and ranks, clears input dimension annotations, output shapes and stale `value_info`, then binds concrete input facts without changing weights or operators.

A bounded LRU holds 3..=12 plans keyed by graph and external-data hashes, exact frame count, input dimensions, runtime revision and threads. Threads are 1..=4 on an instance-owned executor; global runtime settings are untouched. `fingerprint` binds weights, tokenizer, prompts, preprocessing/decision recipe, runtime and threads, and engine uses it in persistent analysis artifact keys. The cache holds only inference plans and fixed prompt vectors.

`faces`, `speaker` and `scene` observe `ExecutionHooks` at every load/compile/run boundary with `Stage::Vision` (engine maps it to its `Process` stage). A running tract call cannot be interrupted: it drains and its output is discarded if cancellation was requested. Error categories: size/hash `ModelIntegrity`, invalid inputs or output shapes `VisionShape`, ONNX/plan `VisionLoad`, nonfinite or zero-norm outputs `VisionNonFinite`, inference `Inference`. Tract panics are caught and reduced to these.

`aggregate_speakers` combines shot-local track chunk probabilities, averages overlap, smooths +/-2 observed frames and picks the highest track mean at >=0.4, else `NoVisibleSpeaker`; ties take the lowest track ID. Limits per shot: 64 tracks, 512 distinct chunks, 9000 frame coordinates. Duplicate chunk identities are rejected and gaps never filled. `SceneResult::unknown()` records absent classification; model errors propagate rather than becoming unknown. `ShotResult::new` validates bounded boxes, visible-speaker probability and scene consistency. Engine owns shot boundaries, track identity, cross-shot persistence and result caching.

Windows execution is unverified.
