# ai

Provider contracts, execution policy, consent-gated OpenAI and ElevenLabs file
transcription and local Whisper/Gipformer transcription. Owns provider configuration,
capability lookup, scoped upload consent, invocation cost bounds, recognition
assembly, model metadata, download-consent tokens, streaming integrity verification,
verified model loading and the shared HTTPS transport. Excludes model download
execution, audio decoding, job scheduling/retries/checkpoints, cache
storage/promotion, cue segmentation and user-edit policy.

## Entry points

Implement `Transcriber`, `Aligner` or `Translator`; call the free `transcribe`,
`translate` and `assemble` execution functions. Adapter methods are blocking and
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
approval. No provider selection or remote fallback happens automatically.

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
aligner for segments missing segment timing or the requested timing unit. Alignment
is at segment granularity; providers can omit punctuation spans. Existing native
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
other version paths are refused in v1 because the retention/training disclosure is
endpoint-specific. The composition root resolves that entry from `OPENAI_API_KEY` for
development or the OS keychain for the desktop app, then injects it as
`transport::ApiKey`; the key is never serialized, logged or printed by `Debug`, and
request headers/body are redacted too. The direct transport dependency stays on
`ureq` 2.12.1 because `sherpa-onnx-sys` pins ureq 2 and one version beats adopting a
second major (coordinator decision 2026-11-04).
`keyring` 4.2.0 is MIT OR Apache-2.0 and its native stores cover macOS Keychain and
Windows Credential Manager; key storage remains app/CLI composition, not this crate.

`disclosure(scope)` returns the exact endpoint, language, audio format, duration in
microseconds, PCM16 WAV upload bytes, the per-invocation USD upper bound and the current
transcription-specific training/retention statements with source links. Consent must
be granted for that complete scope; a changed payload fingerprint, provider, model,
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
other version paths are refused because the disclosure is endpoint/account specific.
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

`disclosure(scope)` states the ElevenLabs default retention, account-dependent
training opt-out, Enterprise default exclusion and enterprise-only Zero Retention
Mode with source links. The adapter uses default logging, does not send
`enable_logging=false`, and does not assume the account is Enterprise or has opted
out; the composition root must show that uncertainty before consent. The OpenAI
endpoint policy is never reused.

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
responses, redaction, typed failure and cancellation.
`cargo xtask ci` also checks architecture, formatting and Clippy.

Observed evidence covers offline contract behavior only. No model accuracy, timing
fidelity, network adapter, real billing, live streaming, native runtime performance
or platform behavior is established here. Adapters own response-parser allocation
caps before constructing transport records; engine owns job-level memory budgets.

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
UreqTransport, plus the OpenAI endpoint policy and typed HTTP errors. Its only admitted model is
`gpt-6.1-sol`, mode `context-window`, with exported TRANSLATION_SETTINGS. Only the
official OpenAI base URL is admitted. Exact languages are `vi`, `en`, `zh`,
`zh-Hans`, `zh-Hant` for source; target Chinese must select `zh-Hans` or `zh-Hant`
explicitly. All six ordered vi/en/zh directions are supported by the contract;
ASR-only models cannot construct this translator. Anthropic is the other recorded
product choice, owned by subtitle/24; this ticket does not imply it is implemented.

Requests use Responses `text.format` strict JSON schema, no tools, synchronous
execution, `store=false`, `background=false`, standard service tier, and an 8192
output-token ceiling. Wire bodies cap at 64 KiB and responses at the existing
1 MiB transport ceiling. `structured::parse` is the shared envelope/JSON owner for
other structured operations: complete assistant text only, typed Schema,
Incomplete and Refused errors; operation owners validate IDs and text semantics.
Adapters make exactly one attempt. Schema joins transient transport failures as
engine-retryable; refusals, incomplete results, auth/quota/consent/cost failures
do not retry. No provider fallback is automatic.

Translation disclosure is separate from transcription: API content is not used
for training by default, but account opt-in can differ; store=false avoids normal
application-state storage while standard abuse monitoring may retain content for
up to 30 days, with approved account controls and policy exceptions. Composition
must show this operation-specific disclosure before Consent::grant. Sources checked
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
