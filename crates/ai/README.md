# ai

Provider contracts, execution policy and local whisper.cpp transcription. Owns
provider configuration, capability lookup, scoped upload consent, invocation cost
bounds, recognition assembly, model metadata, download-consent tokens, streaming
integrity verification and verified model loading. Excludes networking/download
execution, audio decoding, job scheduling, retries, checkpoints, cache
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
nonmonotonic/out-of-segment ranges and unadvertised token capabilities are errors.
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
No workspace unsafe exception is used. `whisper::Whisper::load` verifies the pinned
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
Model loading and each native inference call (up to five minutes of audio) are
synchronous and noninterruptible; cancellation is checked before/after each call and between result segments, and
cancelled work returns no output. Upstream's safe abort callback is deliberately
unused: its 0.16.0 implementation erases a boxed closure but casts it back to its
original concrete type, and does not reclaim that allocation. CPU selection is
explicit; engine owns fallback instead of silently retrying a failed GPU call.
The adapter disables native path/text diagnostics process-wide through the
binding's logging hooks, without enabling its logging backend features.

`ai::transcribe` returns validated segment timing and converts through the shared
`Recognition::to_transcript` sample-to-tick path. `Whisper::recognize_tokens`
additionally retains original subword bytes and optional token ranges: tokens may
split UTF-8, missing/invalid/zero-width token timestamps stay absent, and tokens are
never relabelled as words. Complete segment text is normalized once by captions.
Captions owns script-aware span mapping and persisted token timing (/17). Its
integration turns those raw byte fragments into stored token/character spans. Token capability
is therefore not advertised through the segment-only Transcriber interface.

`cargo run -p ai --example local_whisper -- MODEL MONO-16KHZ-F32LE LANGUAGE [cpu]`
is an explicit local smoke/benchmark command using an already downloaded model;
it exercises public execution and transcript conversion on a continuous window.
It refuses inputs above the invocation ceiling instead of silently cutting speech.
Its JSON output is recognition data deliberately requested by the caller, not logs.
Native tests and benchmarks require external weights/audio and are opt-in; ordinary
unit tests are offline. Windows native runtime and Vulkan remain unverified.

Quality evidence for the continuous-window policy is recorded in the local /15
fix report against all 17 /16 samples. The former independent 30 s policy is
superseded: it produced VI WER 24.91% on that corpus and 36.42% on the south sample.
Holding model, language, decoding and native runtime fixed, a single continuous
south invocation recovers 1.85% WER; restoring temperature fallback alone does not.
Flash attention remains explicitly disabled after an earlier observed VI regression.
Windows/Vulkan, human timing accuracy, and speech-boundary reconciliation for inputs
longer than the invocation ceiling remain unverified.
