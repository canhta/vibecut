# Local face embeddings

`vision::TractFaceEmbedding::load(dir, threads, hooks)` loads the catalog's pinned
SFace ONNX weights only after size/SHA-256 verification; nothing is downloaded
implicitly. The graph runs unmodified on the existing tract CPU runtime with
1..=4 private executor threads and one fixed-shape plan. `embed` accepts an
`IdentityFrame`: finite 112x112 CHW RGB pixels in 0..=255. Media owns five-point
alignment; the graph itself owns centering/scaling. `FaceEmbedding` validates and
normalizes its 128 finite elements and computes bounded cosine similarities. Its
Debug output is redacted and it has no serialization implementation.

Attach a loaded adapter with `TractVision::with_reidentification`; its model,
alignment recipe, runtime and thread fingerprint then enter the vision bundle
fingerprint and engine cache key. `VisionDetector::embedding` returns None when
unavailable. YuNet `FaceBox::landmarks` retains its five points in prepared pixels;
missing landmarks remain explicit. Loading/compilation/inference checks use the
same cancellation, caught-panic and typed-error boundaries as other vision models.
One running tract operation drains before cancellation returns.

SFace model attribution and the approved licence disposition are in
[SFACE.attribution](../../SFACE.attribution). Weights stay in the user cache and are never
bundled. `examples/face_embedding.rs` is an offline tensor smoke/latency and pair
score harness; `tests/vision.rs` covers descriptor validation and biometric
redaction. Sample accuracy is agent-labelled evidence in the ticket report, not
a runtime or library guarantee. Threshold decisions belong to engine.

## Local vision detectors

`vision::VisionDetector` is a blocking, object-safe local inference seam;
`vision::TractVision::load(VisionPaths, threads, capacity, hooks)` implements it
with stable tract 0.23.9 (default `vision` feature). The contracts and decision
functions remain available without that feature. No engine/media dependency or
new scheduling runtime is introduced.

Media supplies borrowed, finite prepared tensors through validated constructors:

- `FaceFrame`: `[1,3,384,640]` CHW BGR, pixel values 0..=255, bottom/right
  letterbox. Returned boxes use prepared pixels, may extend past the frame, and
  are not silently clipped. YuNet score threshold 0.6, IoU NMS threshold 0.3,
  top 5000 candidates, at most 256 returned boxes (excess is a typed failure).
- `SceneFrame`: `[1,3,224,224]` RGB, CLIP normalized. Media owns resize/crop and
  pinned mean/std. CLIP's two fixed prompts above are derived from the pinned
  text tower/tokenizer once per loaded bundle, kept in memory and fingerprinted;
  neither tower nor prompt plan is retained afterwards. No text inference per frame.
- `SpeakerChunk`: exact 25..=75 25-fps 112x112 grayscale frames at 0..=255 and
  exactly four 13-coefficient MFCC frames per video frame, 100 Hz. MFCC absolute
  values are capped at 1e6. Media owns the crop/MFCC recipe; no padding,
  tracking, decoding or timestamp conversion happens in ai.

The adapter verifies and owns the exact bytes it parses, including each external
data file. It decodes directly from the verified byte slice without a second
whole-file reader buffer, and inlines verified external slices before ONNX loading; no ONNX path
resolver or mutable mmap accesses weights. For ASD it checks input names, f32
and ranks, clears input dimension annotations, output shapes and stale
`value_info`, then binds concrete input facts without changing learned weights or
operators. The bounded LRU holds 3..=12 plans, keyed by graph+external-data hashes,
exact frame count, input dimensions, runtime revision and threads. Thread count
is 1..=4 with an instance-owned executor; global runtime settings are untouched.
`fingerprint` binds all weights, tokenizer, exact prompts, preprocessing/decision
recipe, runtime and threads; engine uses it in its persistent analysis artifact
keys. AI's cache stores only concrete inference plans and fixed prompt vectors.

`faces`, `speaker` and `scene` observe `ExecutionHooks` at every load/compile/run
boundary using `Stage::Vision` (engine maps this to its existing `Process` job
stage). A running tract call cannot be interrupted; it
drains and its output is discarded if cancellation was requested. No partial
result is returned. Size/hash failures use `ModelIntegrity`, invalid inputs or
output shapes use `VisionShape`, ONNX/plan failures use `VisionLoad`, nonfinite
or zero-norm outputs use `VisionNonFinite`, inference failures use `Inference`;
tract panics are caught at its boundary and reduced to those safe categories.

`aggregate_speakers` combines shot-local track chunk probabilities, averages
overlap, smooths +/-2 observed frames and chooses the highest track mean at
>=0.4, else `NoVisibleSpeaker`. Ties choose the lowest track ID. It caps a shot
at 64 tracks, 512 distinct chunks and 9000 frame coordinates, rejects duplicate
chunk identities and never fills gaps. `SceneResult::unknown()` records absent
classification explicitly; model errors propagate instead of becoming unknown.
`ShotResult::new` validates bounded boxes, visible-speaker probability and scene
consistency; it exposes face count, boxes, active speaker and scene. Engine owns
shot boundaries, track identity, cross-shot persistence and job/result caching.

`cargo test -p ai --test it vision::` covers hostile sizes, finite values, missing
speaker evidence, overlap, cancellation, invalid bundles and output consistency.
The ignored `real_model_parity_and_exact_tails` additionally requires
`VIBECUT_VISION_MODELS` (the catalog-named YuNet/CLIP files plus six ASD files)
and `VIBECUT_VISION_CORPUS` (private tensors and ORT references). It checks
30/30 face counts, 30/30 speaker shot verdicts across 121 chunks, 50/50 CLIP
labels, and every exact tail 37..=75; see the report for commands,
results and measurements. Windows execution remains deferred.
No general accuracy, auto-layout integration or packaging claim follows.
