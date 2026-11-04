# ai

Provider contracts, execution policy, consent-gated OpenAI and ElevenLabs file
transcription and local Whisper/Gipformer transcription. Owns provider configuration,
capability lookup, bounded local vision inference, scoped upload consent, invocation cost bounds, recognition
assembly, model metadata, download-consent tokens, streaming integrity verification,
verified model loading and the shared HTTPS transport. Excludes model download
execution, audio decoding, job scheduling/retries/checkpoints, cache
storage/promotion, cue segmentation and user-edit policy.

## Entry points

Implement `Transcriber`, `Aligner`, `Translator` or `clips::ClipSuggester`; call the free
`transcribe`, `translate`, `assemble` and `clips::suggest_clips` execution functions. Adapter methods are blocking and
object safe; engine runs them on its worker and supplies `ExecutionHooks`.
Adapters must observe cancellation during expensive work and must not perform IO
in `config`, `capabilities` or `estimate`. No runtime or engine dependency is
introduced. Typed errors carry categories, safe static diagnostics and optional
retry delay, never provider bodies, text, keys or paths.

`ProviderConfig::new` and serde validate schema version 1, nonempty provider,
model/settings identity and tagged `location` (`kind: local` or `remote`). Remote
URLs require HTTPS and reject embedded credentials, query and fragments. Backend
names are extensible data. `auth_ref` is an external secret-store reference, never
a key; callers must not put secrets in configuration. The settings fingerprint
must include every option affecting recognition. Configuration serializes for
persistence but Debug redacts endpoint, model reference and auth reference through
`NfcText`. Tokens cannot be serialized or deserialized.

`Capabilities` describes an exact model, mode, operation and locale/script. No
language list or implicit locale fallback is built into this crate. Timing units
are a resolved set of supported segment/word/character/grapheme/token capabilities;
unresolved conditional timing is omitted by the adapter. Other feature flags
explicitly distinguish supported/unsupported/unknown/conditional. Timing evidence
is advertised, fixture-verified or live-verified, with a revision. Presence does
not prove accuracy. Upload limits are direct invocation bytes and duration in
integer microseconds; `None` means unknown, never unlimited or zero. Current
execution passes complete mono audio windows; streaming flags describe adapters
without introducing a streaming job protocol. Unsupported capabilities and locale
pairs fail before execution.

## Consent and cost

`Scope` binds exact provider configuration (including endpoint/model/settings),
operation, mode, language pair, source content fingerprint, sample rate, sample
window, upload byte ceiling and exact NFC text payloads. The caller owns content
hashing and audio preparation and must provide an honest source content identity.
Text is matched by value, not an unverified caller-supplied digest. `Consent::grant`
is called by the composition root only after affirmative approval; local requests
need no upload approval. Chunks/retries may shrink the approved window and byte
ceiling; changed provider, payload, text or operation requires another token.
Remote alignment requires its own approval; it never inherits transcription
approval. No provider selection or remote fallback happens automatically. `ProviderSelection`
stores one operation, a primary and an explicitly configured ordered list of up to
seven distinct fallback configurations. New selections and missing serialized
fallback lists default to no fallback. Schema version 1 and duplicate/size checks
apply on deserialization; upload approvals are never persisted with settings.
Engine owns [fallback execution](../engine/README.md#provider-selection-and-fallback).

`Money` uses integer millionths and a three-letter uppercase currency. Estimates
are free, an upper bound, or unknown. Remote execution refuses unknown estimates,
a currency mismatch and bounds exceeding approval. A budget is **per invocation**;
engine owns aggregate job/retry/chunk reservation and must deduct/reserve budget
before calling this seam again. Tokens are in-memory capabilities, not a security
sandbox for arbitrary adapter implementations.

## Recognition and time

`AudioWindow` borrows finite mono f32 PCM with an exact sample count/rate and
absolute nonnegative source sample range. Resampling and source offset mapping
happen before this boundary. Results must use that source coordinate system,
not chunk-relative samples. `RecognitionSegment`/`TimedText` are adapter transport
records, not persisted project models. Every stored text uses captions' `NfcText`.
IDs are unique, timings are contained in the input window and segment, and each
unit layer is ordered by time and matching source text. Different timing layers
may describe the same audio; tokens and characters never become fake words.
`RecognitionSegment::raw_tokens` additionally carries provider-agnostic subword
bytes with optional absolute sample ranges, provenance and validated confidence.
Individual tokens may split UTF-8; only their full concatenation is decoded and
must normalize to the authoritative segment text. Tokens retain their per-segment
ordinal identity. Empty tokens, malformed/truncated UTF-8, duplicate token layers,
nonmonotonic/out-of-segment ranges and unadvertised token timing are errors.
Wholly untimed raw bytes can accompany an untimed segment without a timing
capability. A token range requires both segment timing and advertised token timing;
raw text alone does not advertise or establish recognition timing.
Token byte and record sizes are capped at 16 MiB per segment before assembly.

`transcribe` validates native timings, then calls an explicitly supplied admissible
aligner for segments missing segment timing or the requested timing unit. Missing
timing can also be applied later through `align(recognition, provider, ...)`: this
reuses the same validation/replacement owner, keeps the recognition provider/model
and payload fingerprint, retains native unit layers and accepts a local aligner's
empty-text template scope by binding the exact missing segment texts before
execution. A remote aligner must receive an already text-bound scope and its own
consent; a template never authorizes an upload. `Recognition::needs_timing(unit)`
reports whether alignment is required without exposing timing internals. Separate
alignment rejects audio whose source window or sample rate differs from the
recognition. Alignment is at segment granularity; providers can omit punctuation spans. Existing native
unit layers and segment boundaries remain authoritative. The aligner must preserve
text, IDs and ordering, return every requested segment and declare forced timing.
Wrong-language/unsupported aligners leave missing timing absent; malformed output,
consent/auth/quota/payload failures propagate. No timestamp interpolation occurs.
A raw-token layer satisfies requested token timing only when every token has a
range; absent or partially timed raw tokens request alignment. Forced token timing
fills missing native ranges only with identical token counts and bytes at each
ordinal. Native bytes, confidence and existing ranges remain authoritative; filled
ranges carry forced provenance. Retokenized output, incomplete requested timing
and conflicts with retained native boundaries fail with a typed error. Other
unit layers are available when present; providers may omit punctuation, and
quality/coverage acceptance for those layers remains adapter-specific.

`Recognition::is_text_only` indicates any missing segment timing. `to_transcript`
returns captions' existing `Transcript`, or `TimingUnavailable` when any segment
is untimed. Non-word and raw-token layers become captions-owned `TimingSpans`, preserving
raw-to-NFC many-to-many correspondence, ordinal IDs, unit identity, provenance,
confidence, speaker and missing timing. Existing word-only output keeps `Word`
storage; when spans coexist with words, the complete layers also remain available
in the span owner. No layer is silently relabelled or dropped. Captions owns
script grouping and same-language display conversion; no native Whisper wiring
or language conversion table is introduced by this generic seam. Source samples convert once to ticks using
floor for both absolute boundaries and checked arithmetic. Native fingerprint and
optional alignment fingerprint are available separately; engine must include both
in its job/cache key. The captions conversion preserves the recognizer provider/model and combines the
recognizer settings and complete aligner fingerprint with length-prefixed fields,
so changing alignment also changes the derived transcript fingerprint.

`assemble` requires matching source content, sample rate, language and recognition
fingerprints. Native-only chunks can merge with chunks using forced alignment;
the merged result retains the actual aligner identity. Two different nonempty
aligner fingerprints are incompatible. Fully timed segments sort by source start;
only completely identical observations (same segment ID, text, source range,
timing layers, raw-token bytes/timing/confidence and metadata) deduplicate. Every other overlap, including equal-text
speech with different IDs or ranges, returns a typed ambiguity error, leaving chunk-boundary
reconciliation for an explicit retry/user policy. Untimed segments retain chunk
order and cannot be deduplicated by invented coordinates. There is no fuzzy text
matching or splitting of words at chunk boundaries.

## OpenAI file transcription

`openai::OpenAiTranscriber` is the cloud file adapter. Only `gpt-transcribe` is
admitted; the deprecated `whisper-1` and other models are refused at construction.
The tagged provider config must be `Location::Remote` with backend `openai`, the
official `base_url` `https://api.openai.com/v1`, and an `auth_ref` naming the external
secret entry. Custom processors, proxies, lookalike hosts, regional endpoints and
other version paths are refused in v1 to keep upload consent bound to the official
provider endpoint. The composition root resolves that entry from `OPENAI_API_KEY` for
development or the OS keychain for the desktop app, then injects it as
`transport::ApiKey`; the key is never serialized, logged or printed by `Debug`, and
request headers/body are redacted too. The direct transport dependency stays on
`ureq` 2.12.1 because `sherpa-onnx-sys` pins ureq 2 and one version beats adopting a
second major (coordinator decision 2026-11-04).
`keyring` 4.2.0 is MIT OR Apache-2.0 and its native stores cover macOS Keychain and
Windows Credential Manager; key storage remains app/CLI composition, not this crate.

`disclosure(scope)` returns the exact endpoint, language, audio format, duration in
microseconds, PCM16 WAV upload bytes, the per-invocation USD upper bound, provider
name and official policy link. Consent must be granted for that complete scope; a changed payload fingerprint, provider, model,
endpoint, language, mode, byte ceiling or operation refuses before transport. An
`AuthorizedUpload` value is created only after capability, audio and consent checks,
so no request can be built without a token. Uploads are multipart WAV (mono PCM16),
`model=gpt-transcribe`, `response_format=json` and a `languages[]` hint. Files are
limited to 25 MB and responses to 1 MiB; neither is a duration guarantee. Consent's
`max_bytes` bounds the encoded wire payload (including the 44-byte WAV header), while
`AudioWindow` separately admits at most 64 MiB of in-memory f32 PCM so a valid
compressed container is not rejected by its larger decoded form.

`gpt-transcribe` returns text and detected languages but no timestamps. The adapter
therefore advertises no timing units, returns one untimed segment per invocation with
a stable window-derived ID, and leaves `Recognition::to_transcript` unavailable until
an explicitly selected aligner supplies timing. Chunk overlap reconciliation is the
shared `assemble` contract after alignment; the adapter never invents coordinates or
switches providers. Empty speech returns no segments.

Transport timeouts are bounded and redirects are disabled. Cancellation is checked
before the upload, before parsing and before returning; a blocking request already in
flight runs to its timeout or response and its result is discarded. The adapter never
retries;
typed `Network`, `Server`, `RateLimit` (with optional `Retry-After`), `Auth`, `Quota`,
`Payload` and invalid-response failures propagate to the engine. `Error::is_transient`
identifies the retryable set, and the engine's Transcribe/Translate network policy owns
the attempt budget and backoff; auth, quota and payload failures are never retried.

`cargo test -p ai --test openai` runs offline against a recorded success response,
typed error envelopes, a malformed body and an injected fake transport. It covers
consent refusal before network, payload/provider consent invalidation, multipart and
WAV shape, stable IDs, disclosure fields, error categories, redaction and bounded
response handling. The ignored `live_openai_transcription_uses_production_transport`
requires `VIBECUT_LIVE_AI=1` and `OPENAI_API_KEY`, uploads a bounded synthetic-speech
probe and asserts nonempty text plus nine spoken anchors; see the fixture README for
the reproduction command. It is the only test that reaches the network.

## ElevenLabs Scribe transcription

`elevenlabs::ElevenLabsTranscriber` admits only `scribe_v2`, backend `elevenlabs` and
the official base URL `https://api.elevenlabs.io/v1`; proxies, lookalike hosts and
other version paths are refused to keep upload consent bound to the official
provider endpoint.
The composition root resolves `ELEVENLABS_API_KEY` for development or the OS keychain
entry, then injects it as `transport::ApiKey`; the key is sent as `xi-api-key` and is
never serialized or logged.

Uploads are multipart PCM16 WAV with `model_id=scribe_v2`, the exact `language_code`
(`vi`, `en` or `zh`, no implicit fallback) and `timestamps_granularity=word`. The
documented limits are 3 GB and 10 hours in standard mode; the shared 64 MiB PCM
admission and engine chunking bound actual invocations, and responses are capped at
1 MiB. Consent's `max_bytes` bounds the encoded WAV, including its 44-byte header.

Scribe returns word, spacing and audio-event tokens in seconds. The adapter folds
spacing into the preceding token, preserves audio events as separate timed tokens,
and converts each absolute token boundary once to source samples with explicit
nearest-sample rounding, emitting one
timed segment per invocation with stable window-derived IDs. Word tokens become
`raw_tokens` and flow through captions' shared `/17` span code, preserving
raw-to-NFC-correspondence and the provider's per-language token identity; zero-width
punctuation stays explicitly untimed. No cue segmentation, timestamp interpolation
or provider fallback happens here.

`disclosure(scope)` names ElevenLabs and links its official privacy policy, with
payload details and the invocation cost bound. Composition shows these before
approval without restating the provider's retention or training terms. The adapter
uses default logging and does not send `enable_logging=false`.

Typed failures cover auth, payment/quota, rate limit, payload, server and network;
the adapter never retries, and the engine owns the bounded network retry policy.
`cargo test -p ai --test elevenlabs` runs offline against recorded English,
Vietnamese and Chinese responses plus synthetic errors, consent and disclosure
tests. The ignored live test uses the same bounded synthetic-speech probe as OpenAI;
a separate consented run on the `/16` English corpus sample is recorded in the
ticket report.

## Verification and limits

`cargo test -p ai` exercises JSON native-timed and text-only fixtures through fake
adapters using the production seams: NFC Vietnamese, source offsets, missing-span
alignment, text-only refusal, capabilities/limits, consent/cost, malformed response,
translation, overlap ambiguity, raw-token public execution/serialization,
UTF-8-split NFD/Han/emoji correspondence, explicit missing timing, hostile token
responses, redaction, typed failure and cancellation. Model-catalog tests cover
language-neutral metadata, non-match of neutral weights in speech selection,
unchanged language-bound entries, pinned vision file hashes and refused
truncated/mismatched bytes.
`cargo xtask ci` also checks architecture, formatting and Clippy.

The generic provider fixtures establish offline contract behavior. Adapter-specific
live and local runtime evidence is documented below; these fixtures do not
establish general model accuracy, timing fidelity, billing, live streaming or
cross-platform behavior. Adapters own response-parser allocation caps before
constructing transport records; engine owns job-level memory budgets.

## Model catalog and language-neutral weights

`models::catalog` is metadata only: identity, licence, licence URL, source, exact
files with pinned HTTPS URLs, sizes and SHA-256. Downloads remain engine-owned,
user-initiated, consented and integrity-verified; weights are never bundled.

`Model::languages` describes supported language scope. An empty list is
**language-neutral** metadata for weights that do not read or produce language,
such as vision detectors. Validation accepts that empty list instead of rejecting
it, and `Model::language_neutral()` exposes the distinction. It never means
"supports every language": `Model::matches_language` returns false for neutral
weights, and `suggested_transcriber` requires a real language match, so a vision
entry can never be selected as an ASR model. Existing entries keep their concrete
lists (`whisper-large-v3-turbo`: vi, en, zh; `gipformer-68m-rnnt-int8`: vi;
`whisper-small-dtw`: vi, en, zh, zh-Hans, zh-Hant).

Vision entries added by p1-clips/14, both MIT and both language-neutral:

| Model id | Upstream source | Files |
|---|---|---|
| `yunet-2026may` | `opencv/opencv_zoo` `48b10fa505aeb51fbd127fc60a5c660040e823b7`, `models/face_detection_yunet` | `face_detection_yunet_2026may.onnx`, 229,738 B, `ebafce4e3c118d6554634be5c27ab333b4c047a9a8c3faf1d7cf93101c22f0f0` |
| `clip-vit-base-patch32` | `Xenova/clip-vit-base-patch32` `6b7c523703ae26e39be4b03774966f8571c7f523`; weights derived from MIT OpenAI CLIP | `vision_model.onnx` 351,685,709 B `fd6e1402a588279d1723c7534d4bcba5bc0b14b47dfab0e46f8c47b8270d7d40`; `text_model.onnx` 254,058,553 B `3f6571f5bad13a97c469c1622e1cfc4d9aef78b79fdbfcff804ca357bfada8cc`; `tokenizer.json` 2,224,119 B `f7f3b7af117d467b58374797691a6438d3e6b9e9cef800dfd5dced7f697a90cd`; `config.json` 4,524 B `493ef57ff783e42d1530c91b53469b7fdf8db8a9c1408e86998fcb7899a4f495`; `preprocessor_config.json` 520 B `6f638fb9401a6d6296feff533ee7efe657b787c49f954f82f5906b36ef2a1b1f` |

The CLIP text tower is registered so fixed prompt embeddings are reproducible,
not to run text inference per frame. No derived embedding file is downloaded,
shipped or bundled: compute the two fixed prompts once on first use, keyed and
cached by text-model hash + tokenizer hash + exact prompt text. Tokenize with the
pinned `tokenizer.json`, pad or truncate with id 49,407 to 77 ids, run
`text_model.onnx` with `input_ids` `[N,77]` int64, and L2-normalize each projected
embedding. The prompts are `a screenshot of a computer screen` and
`a photograph of a person`; image embeddings come from `vision_model.onnx` with
the pinned 224x224 RGB mean/std; screen probability uses `14.3 * (image · text)`
and a 0.5 threshold. The p1-clips/15 adapter owns that wiring; this crate only
publishes metadata.

### Light-ASD export recipe (entry intentionally absent)

`light-asd-ava` has no catalog entry until a project-controlled immutable HTTPS
host for its converted graphs is approved; `ModelFile` admits no local or derived
path. The upstream source is `Junhua-Liao/Light-ASD`
`ed38c232de5efe0261dbd68627c0ade7cdfe14eb`, MIT repository notice (the repository
has no separate weight-licence file; record the maintainer decision before any
default), `weight/pretrain_AVA_CVPR.model`, 4,175,289 B, SHA-256
`d44bc3ea7baa8e0946fa3921311714a630ed8b90a1928fab0dbe30d918909317`.

The canonical runtime bundle is the **original dynamic export** triplet from
p1-clips/13, selected by the coordinator for /15. Static exports and per-length
files are not admitted. Supply these six local files explicitly (no automatic
transfer, catalog entry or default-model exposure until hosting is approved):

| File | Bytes | SHA-256 |
|---|---:|---|
| `lightasd_audio.onnx` | 99,776 | `335dd4f221fbe90330761209c1acd8637fd67399003d69b79031a7d75c820318` |
| `lightasd_audio.onnx.data` | 1,105,408 | `b440a45f84af5e34890f48573b1e164587c690191f7a2c22d902ad5edb61f152` |
| `lightasd_visual.onnx` | 19,002 | `4bcec1cc985eea8416540ab481e198e611b4125b26c124408cd89442bf4def4e` |
| `lightasd_visual.onnx.data` | 2,173,696 | `4400bdaa0925a73bc0486fd569be5223fc8b9cb5e2c6c2d8c487b288d4b1e267` |
| `lightasd_backend.onnx` | 9,393 | `f3d2b8b2ac81888e1a023e0bb1b3d9b764a151bf309d2510ae1357fbecd58084` |
| `lightasd_backend.onnx.data` | 794,112 | `960c75126082846b34623606a9aff56baa6f092f1dbb7d37d59a2096105af9a0` |

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
  values are capped at 1e6. Media owns the /12 crop/MFCC recipe; no padding,
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

`cargo test -p ai --test vision` covers hostile sizes, finite values, missing
speaker evidence, overlap, cancellation, invalid bundles and output consistency.
The ignored `real_model_parity_and_exact_tails` additionally requires
`VIBECUT_VISION_MODELS` (the catalog-named YuNet/CLIP files plus six ASD files)
and `VIBECUT_VISION_CORPUS` (private /13 tensors and ORT references). It checks
30/30 face counts, 30/30 speaker shot verdicts across 121 chunks, 50/50 CLIP
labels, and every exact tail 37..=75; see the local /15 report for commands,
results and measurements. Windows execution remains deferred to p0-core/14.
No general accuracy, auto-layout integration or packaging claim follows.

## Local Whisper

The default `whisper` feature uses safe whisper-rs bindings, with Metal on macOS
and CPU elsewhere. `whisper-vulkan` enables Vulkan (requires the Vulkan SDK).
`--no-default-features` retains offline contracts/model policy without native builds.
The narrowly admitted callback module is documented in
[ADR 0003](../../docs/adr/0003-whisper-callback-boundary.md); other ai modules
deny unsafe code. `whisper::Whisper::load` verifies the pinned
model's exact size and SHA-256 with a fixed 64 KiB buffer before native loading;
callers must keep the cache path immutable during loading/use. Models are never
bundled. The multilingual default is `whisper-large-v3-turbo`, with admitted
`vi`, `en`, `zh` capability data. No implicit language/script fallback is applied.

`models::catalog` exposes source, licence URL, size and pinned download metadata.
Composition shows those fields and obtains a user download click before granting
`models::DownloadConsent`; this is distinct from upload consent. Authorize against
the complete catalog record before any transfer, including resumption. Other
weight licences remain opt-in and ineligible as defaults. `models::verify_file`
rejects truncation, excess bytes and hash mismatches, checks cancellation between
reads, and never writes/promotes files. `models::storage_error` maps disk-full and
other storage failures to safe typed categories. `models::cache_dir` resolves macOS
`~/Library/Caches/VibeCut/models` or Windows `%LOCALAPPDATA%/VibeCut/models`, without
creating directories. Engine owns downloads, scheduling, resume/retry/checkpoints,
cache identity/storage/promotion and model-manager wiring; these are not implemented
by this crate.

The existing `transport::UreqTransport` also implements `DownloadTransport` for
streaming GET responses, retaining duplicate response headers for strict engine
validation. It uses the same agent/timeouts; provider responses keep their 1 MiB
buffer cap. `download_redirect` resolves only bounded HTTPS URLs without embedded
credentials or fragments. The engine owns redirect limits and Range/ETag policy.
Catalog validation admits bounded portable raw filenames, rejects device names,
case-folded duplicates and unsafe paths, and caps file/aggregate sizes before IO.
The [engine transfer contract](../engine/README.md#local-model-transfers) owns
verified native bundle leases; callers retain a lease while native paths are used.

Whisper accepts finite mono 16 kHz PCM windows of at most five minutes, with absolute
source sample offsets. Whisper chooses timestamp-based decode boundaries within
each call; do not cut
continuous speech into independent 30 s calls. Engine chooses speech-safe invocation
boundaries and owns reconciliation for longer recordings. The five-minute bound
caps input buffering at 19.2 MB of f32 PCM; it is a safety ceiling, not evidence that
hard cuts at five minutes preserve speech.
Model loading is synchronous and checked for cancellation before/after loading.
Inference uses whisper-rs raw abort/progress callbacks backed by call-owned atomic
state; a scoped observer polls execution hooks every 2 ms, acknowledges cancellation
through `ExecutionHooks::cancelling` once, then suppresses progress and all results.
The engine's hook implementation owns the visible Cancelling job state. Native
work stops at the next abort poll and the observer joins before return. The binding's
released safe setters are bypassed because their allocation ownership/type cast is
incorrect; no patched binding or native runtime is vendored.

The bundled native runtime polls after encoding a window and after decoder work;
it cannot interrupt an encoder graph in flight. Coordinator-accepted residual
latency is one encoder window: observed isolated four-thread M1 Pro release runs
returned 0.36–0.41 s after the cancellation deadline on Metal and 3.0–3.7 s on CPU.
These are measurements, not universal bounds; contention and other hardware can
increase the delay. Native regression checks allow 1 s on Metal and 4.5 s on CPU
after a 1 s deadline, with acknowledgement within 100 ms. Windows/Vulkan latency
remains unverified. Cancellation never waits for the entire five-minute invocation.
CPU selection is
explicit; engine owns fallback instead of silently retrying a failed GPU call.
The adapter disables native path/text diagnostics process-wide through the
binding's logging hooks, without enabling its logging backend features.

`ai::transcribe` returns validated native segments and original subword bytes in
`RecognitionSegment::raw_tokens`, with optional absolute source-sample ranges,
native provenance and confidence. The adapter advertises segment and token timing;
individual tokens can still lack timing. Missing, invalid and zero-width timestamps
stay absent. Tokens may split UTF-8 and are never relabelled as words.
`Recognition::to_transcript` uses the shared captions span owner to retain token
ordinals, raw-to-NFC correspondence and missing timing, converting sample boundaries
once to ticks. There is no separate native token representation or conversion path.

`LANGUAGES` exposes the adapter's exact vi/en/zh data for offline composition;
`Whisper::detect_language` performs one bounded language-detection pass on an
already decoded 16 kHz window and returns a captions `LanguageTag`. Detection is
an explicit caller action, not an implicit provider fallback: the caller supplies
the detected tag to the real transcription request and still owns chunking,
consent and retries. No transcription result or timestamp is produced by detection.

`cargo run -p ai --example local_whisper -- MODEL MONO-16KHZ-F32LE LANGUAGE [cpu]`
is an explicit local smoke/benchmark command using an already downloaded model;
it exercises public execution and transcript conversion on a continuous window.
It refuses inputs above the invocation ceiling instead of silently cutting speech.
Its JSON output is recognition data deliberately requested by the caller, not logs.
Native tests and benchmarks require external weights/audio and are opt-in; ordinary
unit tests are offline. Windows native runtime and Vulkan remain unverified.

Native decoding policy is language-profile data owned by this provider: English
uses greedy decoding (best-of 1); the generic profile uses beam search (beam 5),
including the admitted Vietnamese and Chinese languages. All profiles retain
continuous timestamp-selected native windows, temperature zero with fallback
disabled, no prompt, per-call state and flash attention disabled. The provider
settings fingerprint includes the complete profile table and generic strategy.
Captions' language registry remains the owner of layout and segmentation profiles.

Observed /16 corpus quality and cancellation evidence is recorded in the local /15
review-fix report. The former independent 30 s policy damaged Vietnamese speech;
continuous windows recover it. Continuous beam search worsened English noise/music;
English greedy decoding reduces its aggregate error count and music omissions while
preserving the established Vietnamese/Chinese settings. Disabling intra-call text
history worsened English and was rejected. Flash attention remains disabled after
an observed Vietnamese regression. These corpus observations do not establish
universal model accuracy or human timing fidelity. Windows/Vulkan and speech-boundary
reconciliation beyond the invocation ceiling remain unverified.

## Local Gipformer

The default `gipformer` feature includes the safe sherpa-onnx bindings and the
`vibecut-local-asr` native runner binary. `gipformer::Gipformer::load` accepts the
pinned model, immutable cache directory, absolute shipped runner path, explicit
device/thread count and execution hooks. Load verifies all four files with the
shared integrity policy and starts a reusable native process. No local unsafe,
model download, engine dependency, second transfer cache or scheduler is added.

`models::suggested_transcriber` returns metadata for the Vietnamese profile's MIT
Gipformer model; other profiles retain the multilingual Whisper suggestion. This
never executes a provider, falls back to another provider or starts a download.
Composition still presents explicit provider choice and obtains a download click
using `DownloadConsent`; engine's existing model-transfer/cache owner consumes
the same catalog records. The app must ship the native runner alongside its
executable and pass its absolute trusted path. Windows packaging/runtime remains
unverified.

CPU execution is supported. The released static runtime silently falls back from
CoreML to CPU, so this adapter refuses `Device::CoreMl` with `Unsupported`. It does
not claim accelerator execution. A different upstream native build needs separate
validation; no third-party source or timing behavior is patched here.

Invocations accept finite mono 16 kHz f32 PCM and absolute source offsets, capped
at five minutes. The native runner buffers and decodes one 20 s chunk at a time,
including the short final tail. Engine owns longer recordings, speech-safe outer
windows and reconciliation. Hard chunk boundaries can affect recognition quality;
there is no additional VAD model download. The settings fingerprint includes the
model hashes, native binding, chunk/decode policy, hardware request and thread count.

The native runtime lives outside the application process because it can write
paths in diagnostics and terminate on native failures. Its stderr is discarded;
private framed pipes carry bounded metadata, PCM and an explicit recognition reply.
One invocation per adapter is supported; overlapping calls return `Unsupported`
for engine scheduling. Pool locks cover ownership changes only, never IO. Cancellation
acknowledges once, kills/reaps native work and joins pipe threads before returning
`Cancelled`; no partial output escapes. Hook unwinding also kills/reaps before
joining. A subsequent call replaces the stopped process. Poisoned ownership locks
fail with `Inference`; there is no speculative recovery.

Native RNNT token start points lack durations. The adapter preserves their raw
lexical bytes and ordinal order with explicit missing ranges and omits timing
capabilities. It preserves the leading BPE space rather than rewriting bytes to
match the runtime's trimmed display text; any other text discrepancy is an error.
`Recognition::to_transcript` returns `TimingUnavailable` until an explicitly
selected aligner supplies speech ranges. Shared assembly then retains raw/NFC
correspondence, forced provenance and sample-to-tick conversion. Neither adjacent
starts nor chunk ends become invented speech ends; no aligner or ASR provider is
selected automatically. Native timing MAE is consequently unavailable.

`cargo test -p ai` covers bounded framing, native failure, blocked-pipe cancellation,
hook unwind, missing timing, forced alignment/serialization and catalog consent.
Unix fake-process tests exercise actual OS pipe/kill/join behavior. The ignored
`local_gipformer` test requires explicit pinned weights and local speech; it uses
public execution, serialization, context reuse, 300 s cancellation and recovery.
`cargo run --release -p ai --example local_gipformer -- MODEL_DIRECTORY RUNNER
cpu THREADS AUDIO_F32LE...` benchmarks multiple inputs on one loaded model. JSONL
is deliberate caller-requested recognition output. Hardware/runtime performance,
model quality and accelerator acceptance require separate measured evidence;
builds and offline tests do not establish them.

## Contextual text translation

`Translator::translate` now accepts one `TranslationWindow`: stable segment IDs,
source/target language, before/after context, fixed glossary terms or names to
preserve, and style. The result maps every requested ID to a nonempty target text;
no extra/missing/duplicate IDs or partial success is admitted. Glossary rules are
checked by exact term/translation containment; spelling/case must match the supplied
term. Before/after context never receives output IDs. This replaces the earlier
single-text seam; it does not assign source timestamps to translated words.

A window admits 1..=64 segments, 16 context strings on each side, 128 unique glossary
terms, unique nonblank IDs up to 256 bytes and at most 32 KiB of canonical JSON.
Result text totals at most 64 KiB. `window.consent_text()` is the complete canonical
payload for `Scope.texts` (one entry); source/target languages must also match Scope.
Changing an ID, source text, context, glossary, style, script, provider or model
invalidates consent before transport. Text-only composition uses SampleRange [0,1)
and sample_rate 1 as Scope's non-audio sentinel; these fields do not assign timing.

`openai::OpenAiTranslator` uses the shared transport's ApiKey, HttpTransport and
UreqTransport, plus the OpenAI endpoint policy and typed HTTP errors.
`TRANSLATION_LANGUAGES` and `TRANSLATION_TARGET_LANGUAGES` expose the exact source
and target tags for offline capability composition; target Chinese remains an
explicit `zh-Hans`/`zh-Hant` choice. Its only admitted model is
`gpt-6.1-sol`, mode `context-window`, with exported TRANSLATION_SETTINGS. Only the
official OpenAI base URL is admitted. Exact languages are `vi`, `en`, `zh`,
`zh-Hans`, `zh-Hant` for source; target Chinese must select `zh-Hans` or `zh-Hant`
explicitly. All six ordered vi/en/zh directions are supported by the contract;
ASR-only models cannot construct this translator. Anthropic is the other recorded
product choice and is implemented below.

Requests use Responses `text.format` strict JSON schema, no tools, synchronous
execution, `store=false`, `background=false`, standard service tier, and an 8192
output-token ceiling. Wire bodies cap at 64 KiB and responses at the existing
1 MiB transport ceiling. `structured::parse` is the shared envelope/JSON owner for
other structured operations: complete assistant text only, typed Schema,
Incomplete and Refused errors; operation owners validate IDs and text semantics.
Adapters make exactly one attempt. Schema joins transient transport failures as
engine-retryable; refusals, incomplete results, auth/quota/consent/cost failures
do not retry. No provider fallback is automatic.

Translation disclosure names OpenAI and links its official policy, alongside the
language pair, model, endpoint and cost. Composition shows the payload and these
fields before Consent::grant without restating provider terms. Sources checked
2026-11-04: [structured outputs](https://developers.openai.com/api/docs/guides/structured-outputs),
[model](https://developers.openai.com/api/docs/models/gpt-6.1-sol),
[data controls](https://developers.openai.com/api/docs/guides/your-data),
[pricing](https://developers.openai.com/api/docs/pricing).

The per-attempt USD upper bound is 344,064 micro-USD: conservative 131,072 input
tokens at $2/M plus 8192 output tokens at $10/M, standard short-context list prices.
This bounds the capped input bytes plus message/schema overhead rather than using
a characters-per-token estimate. It is deliberately loose; no actual billing or
translation-quality result is established by the bound. Engine adds the operation
attempt ceiling and aggregate authorization; provider code never retries or edits
projects. Digits-set mismatch yields cue-ID warnings for numbers/units review;
this is advisory, cannot validate translated units or distinguish equivalent dates,
and intentionally does not compare digit multiplicity.

### Anthropic translator

`anthropic::AnthropicTranslator` implements the same `Translator` contract and
`TranslationWindow` validation with Claude Sonnet 5.5 (`claude-sonnet-5-5`), mode
`context-window`; it exposes the same translation language tables. It admits only backend `anthropic` and the official base URL
`https://api.anthropic.com/v1`; proxies, lookalike hosts and other version paths are
refused. Composition resolves `ANTHROPIC_API_KEY` for development or the OS keychain
and injects it as `transport::ApiKey`; the key is sent as `x-api-key` with
`anthropic-version: 2026-10-21` and is never serialized or logged.

`anthropic::MessagesClient` is the reusable Messages transport for later LLM
operations such as p1-clips/01: it owns the endpoint, headers, body cap and typed
wire errors. Translation sends synchronous Messages with `stream=false`,
`thinking={"type":"between_tools"}` (Sonnet 5.5 rejects `disabled`), an 8192
output-token ceiling, and a strict `output_config.format` JSON schema. Responses
must be one complete message with a text block, `end_turn`, no refusal and valid
IDs; thinking blocks are ignored. Schema joins transport transients as
engine-retryable; refusals, incomplete results, auth/quota/consent/cost failures do
not retry and no provider fallback is automatic.

The Anthropic disclosure names Anthropic and links its official data-retention
policy instead of restating retention or training terms; provider and model choice
belong to the user, and account-specific ZDR, retention and training settings remain
outside this adapter. Users must review the linked policy before granting consent.
Sources checked 2026-11-04:
[Messages](https://platform.claude.com/docs/en/api/messages),
[structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs),
[data retention](https://platform.claude.com/docs/en/manage-claude/api-and-data-retention),
[pricing](https://platform.claude.com/docs/en/about-claude/pricing).

The per-attempt USD upper bound is also 344,064 micro-USD (Sonnet 5.5 list prices
$2 input / $10 output per 1M tokens) with the same conservative byte ceiling. It is
deliberately loose, not a billing measurement or quality claim. Offline
`tests/anthropic.rs` covers six directions with explicit Hans/Hant targets, strict
wire/schema shape, context/glossary/style consent invalidation, numbers/glossary
validation, refusal/incomplete/oversized/error mapping, redaction and endpoint
admission. The ignored live six-direction test requires `VIBECUT_LIVE_AI=1` and
`ANTHROPIC_API_KEY`; a consented run is recorded in the ticket report. Prompt caching
is not enabled: windows are short, independent and below the useful cache threshold,
and no cached user content is needed.

Offline `tests/translation.rs` covers all direction/script request configurations,
strict wire shape, context/glossary/style consent invalidation, exact ID rules,
fixed terms, digit warnings, redaction, malformed/truncated/oversized/refused output,
unsupported ASR model construction and cancellation before/after transport.
Fixtures are synthetic, not captured live model quality evidence. The opt-in
`translate_eval` example consumes public windows, requires VIBECUT_LIVE_AI=1,
OPENAI_API_KEY and VIBECUT_TRANSLATE_EVAL_BUDGET_MICROS, checks the total single-attempt
ceiling before network, and writes completed outputs only to the requested local
evaluation file. It never retries; language quality/script fidelity requires live
outputs and independent human scoring in the local evaluation sheet.

## Local Candle alignment

`alignment::CandleAligner` (feature `candle-align`, enabled by default) implements
`Aligner` with its own safe CPU runtime, independent of whisper.cpp. Load the
pinned `models::alignment_model()` from immutable app-managed model files; all four
files pass existing size/SHA-256 verification before buffered safetensors loading.
No mmap, local unsafe, transfers, remote inference, provider fallback or duplicate
model store is introduced. The catalog includes source/licence/size/hash metadata;
app downloads still require a user click through the existing model manager.
The coordinator-approved development download stays outside Git under `.spikes/`.

The runtime teacher-forces caller-supplied transcript text, obtains the checkpoint's
selected cross-attention heads, softmaxes over real audio frames, standardizes over
query tokens, applies a seven-frame median filter, and runs bounded monotonic DTW.
Missing/nonfinite/degenerate attention returns `AttentionUnavailable`; zero-width
lexical spans return `TimingUnavailable`. No `(0,0)` fallback or proportional text
timestamps are returned. Prescribed text is preserved, never translated or decoded
as a replacement transcript.

Inputs are finite mono 16 kHz PCM, 201..=480000 samples (at most 30 seconds), 1..=64
nonblank identified segments, at most 16 KiB total source text, IDs up to 256 bytes,
and at most 443 model text tokens. Overlong inputs fail before model execution;
engine owns longer speech-safe windows and must not treat this as automatic chunking.
The complete original segment texts are bound to the shared alignment Request;
capabilities and consent use the existing AI policy. Local execution estimates Free.

Captions owns normalization and Unicode grouping. English words, Vietnamese
syllables and Chinese characters use its existing timing profiles; bare `zh` now
shares the Han-character policy with zh-Hans/zh-Hant. Tokenization respects those
group boundaries rather than subdividing an inferred token duration. Punctuation
can remain untimed. Forced unit records carry the profile's actual unit and preserve
source IDs/text. Native source ranges constrain the returned intervals; invalid
constraints refuse rather than fabricating durations. Shared AI assembly retains
any original native timing/token layers and performs the sole transcript conversion.

Input AudioWindow coordinates are absolute source samples. Media supplies decoded,
resampled PCM with codec/filter delay already handled; this adapter neither
resamples nor adds delay. Centered STFT contributes no extra offset. DTW frame
boundaries map to 320 samples (20 ms), clamp to the actual window/native segment,
and add the window start exactly once. The public native test checks offset
invariance through the real AI assembly and transcript conversion.

One invocation per loaded adapter is supported; the engine schedules concurrency.
Runtime ownership uses a nonblocking lock; contention/poisoning returns Inference.
Cancellation is checked during mel framing, each encoder/decoder block and each
DTW row. An in-flight matrix operation finishes before the next check; the adapter
then returns Cancelled and discards output. No extra worker/runtime is created.
The settings fingerprint hashes all model metadata, runtime policy and language
profiles, keeping model/grouping changes out of previously derived work.

Offline tests cover malformed/empty/nonfinite inputs and attention, monotonic DTW,
normalization/median filtering, Han/VI/EN grouping, bounded token counts, cancellation,
metadata refusal and analytical mel/offset fixtures. Opt-in `candle_alignment`
requires `VIBECUT_CANDLE_MODEL_DIR`, `VIBECUT_ALIGN_PCM` and `VIBECUT_ALIGN_SEGMENTS`;
it exercises actual local output through shared assembly, exact nonzero source
coordinates, cancellation and silence/empty refusal. `local_alignment` is a direct
local smoke example; its six arguments are model directory, f32le PCM, language,
segments JSON, output JSON and source offset in samples. No model accuracy or timing
quality gate is part of integration acceptance; see the AI integration verification
policy. CPU model output proves integration only. Accelerator/Windows runtime is
not established by these checks.

Candle/tokenizers impose approved transitive library-version limits alongside
existing dependencies; exact skips, parent constraints and removal conditions live
in [deny.toml](../../deny.toml). Direct dependencies remain latest stable, with no
third-party patch or downgrade. The repository had no duplicate gate wired into CI;
this file records the scoped exceptions for cargo-deny rather than adding a new
unrelated check. Remove each skip when its parent unifies with the workspace version.

## Transcript clip suggestions

`clips::ClipInput` accepts stable transcript IDs, NFC text and nonoverlapping,
ordered source ranges (`ClipSegment::from(&captions::Segment)`), plus count,
inclusive min/max total duration in ticks, required positive `min_segment_duration`,
`max_segments` (JSON default four, 1..=32), maximum allowed summed source IoU, language and
optional keywords. Count is 1..=32; input is at most 10,000 segments / 4 MiB of
ID/text data. Empty/duplicate IDs, blank text, negative or overlapping ranges,
nonfinite overlap values and invalid bounds refuse before execution. Languages
are vi/en/zh/zh-Hans/zh-Hant for all three current adapters.

`ClipInput::windows()` plans overlapping windows capped at 128 segments and 24 KiB
of canonical JSON, at most 1,024 windows. Overlap covers the configured maximum
clip duration for contiguous spans. Assembly is local to each bounded window;
pieces separated by more than a window cannot be proposed together. This does not
guarantee exhaustive noncontiguous combinations across a long transcript. A transcript too dense to preserve all admissible boundary-spanning
ranges in those limits returns Payload; increase segmentation granularity or
reduce the requested duration explicitly. Plan first, show each window's exact
`consent_text`, provider policy link and estimate, and reserve an aggregate budget
in engine. Pass one `Request` per window in the same order to `suggest_clips`.
Every window's provider/language/settings/payload consent and per-invocation cost
is checked before any IO. Requests remain independent invocations, never implicit
retries or job-level budget approvals. Wire JSON is also capped at 64 KiB and by
the approved byte ceiling; shared transport caps responses and supplies timeouts.
At most 4,096 distinct ordered lists of endpoint IDs and roles are retained per
execution, counting both valid and invalid candidates. Repeated observations of a list are
reconciled before this bound, retaining the highest validated score (first on a tie).
A valid observation supersedes an out-of-window/invalid observation of the same
list; invalid metadata can never replace an accepted short. Each
valid list retains at most one representative duplicate diagnostic; repeated
invalid observations share one typed drop diagnostic. Thus retained state
holds at most 8,192 proposals including representative duplicates, regardless of
window overlap. The final global ranking and overlap selection use all distinct
valid candidates; no window is trimmed to the requested output count.

`OpenAiClipSuggester` uses the existing shared HTTPS transport, Responses endpoint,
strict structured output parser, user-selected `gpt-6.1-sol` or `gpt-6-luna`, and
`OPENAI_SETTINGS` (version two for the segment-list contract);
`AnthropicClipSuggester` reuses `anthropic::MessagesClient` and `message_text`,
`claude-sonnet-5-5`, and `ANTHROPIC_SETTINGS`. Both require the official provider
endpoint and exact versioned settings identity through the existing ProviderConfig.
Their operation-specific disclosure returns the provider name, policy link,
endpoint/model, exact payload and conservative USD invocation upper bound (input
wire byte ceiling plus framing allowance, 8,192 output tokens). Luna uses the
same conservative price ceiling as Sol; the disclosure and consent retain the
exact selected model. No SDK, runtime,
second provider configuration, generic retry or automatic fallback is introduced.

All adapters run through `ClipSuggester` and the same result validation. A
`ClipProposal` is `title`, `hook_text`, `reason`, ranking `score` and `segments` in
play order. Each `ClipPiece` contains exact `start_segment_id`, `end_segment_id`
and a closed `role` (`hook`, `context`, `payoff`). Reordering pieces is allowed;
each piece's endpoints must remain in source order. Only transcript IDs resolve
ranges; timestamp fields, unknown roles, old single-range fields, oversized IDs,
blank/oversized metadata and nonfinite/out-of-range scores refuse as Schema.
Semantic violations drop the individual proposal with a typed reason:
`UnknownId`, `ReversedRange`, `SegmentCount`, `HookOrder`, `SegmentOverlap`,
`SegmentDuration` or `Duration`. A hook is optional; when present exactly one must
be first. Source overlap within a short is refused even when pieces play apart.
Raw piece count must fit `max_segments`. Consecutive play pieces whose source
boundaries touch exactly merge, retaining the first role and the combined endpoint
IDs; the resolved piece carries typed `repair: AdjacentMerged`. The per-piece minimum applies after this canonical repair. Total duration is
the checked sum of canonical piece durations, excluding source gaps, and must fit
the inclusive min/max bounds.

`ClipSuggestion` carries the canonical proposal, `segments` resolved to integer
source `range`, `role` and zero-based `source_order` rank (still in play order),
`duration` (summed ticks), and `source: llm` or `source: heuristic`. There is no
single enclosing range. Downstream review/export must iterate this segment list;
source-order rank lets them show source and play order independently. Inner cuts
are the gaps between pieces; this contract contains no word-level cut list.
The serialized engine provider result wraps these suggestions and drops in its
existing provider-provenance envelope. CLI provider execution/rendering belongs
to its separate feature ticket; the CLI currently exposes the job stage only.

Scores sort descending with deterministic playback-coordinate ties. Summed
pairwise source intersections divided by summed source union define IoU; gaps
never contribute. Equal source coverage, or IoU >= 0.5 with normalized source-text
character-bigram Jaccard >= 0.8, deduplicates retaining the higher score regardless
of play order. Other candidates exceeding overlap tolerance drop with `Overlap`;
candidates beyond count drop with `CountLimit`. Fewer than count, including zero,
is valid. Scores are rankings, never probabilities; suggestions await user review.

`HeuristicClipSuggester::new()` is an explicitly selected, free local provider.
It ranks complete segment-edge spans using sentence-ending punctuation, pauses
of at least 0.5 seconds and user keyword density. It never splits timed segments
or fabricates word boundaries. It emits one `payoff` piece per suggestion, respecting both total and piece
minimums. It returns at most one best end per start before
shared validation/merging. On a remote failure the original typed error propagates;
`clip_recovery_choice(&error)` supplies a `RerunLocally` offer for presentation.
Only an affirmative user selection starts a fresh local execution. Cancellation
returns Cancelled and discards partial output, including cancellation after IO.

`cargo test -p ai --test clips` covers both wire formats, hostile/schema/ID/duration
inputs, exact consent and wire budgets, provider failures, cancellation, duplicate
and overlap selection, long-window coverage and multilingual heuristic execution.
The fixed synthetic EN/VI/ZH ranking fixture compares keyword ranking to a
sentence/pause-only baseline; the broader labelled pipeline harness is separate.
The ignored `live_clip_providers` test requires `VIBECUT_LIVE_AI=1` and
`OPENAI_API_KEY` and sends one bounded `gpt-6-luna` request. It uses synthetic
input by default; `VIBECUT_CLIP_SMOKE_INPUT` accepts a local `ClipInput` JSON file
from a consented real transcript, limited to one window. Optional
`VIBECUT_CLIP_SMOKE_OUTPUT_DIR` writes the validated result locally. Offline fixtures and
live integration prove contracts, not model accuracy or editorial quality.
