# ai

Offline contracts and execution policy for transcription, forced alignment and
translation. Owns provider configuration, capability lookup, scoped upload
consent, invocation cost bounds and recognition assembly. Excludes networking,
model loading/downloads, audio decoding, job scheduling, retries, checkpoints,
cache storage, cue segmentation and user-edit policy.

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

`transcribe` validates native timings, then calls an explicitly supplied admissible
aligner for segments missing segment timing or the requested timing unit. Alignment
is at segment granularity; providers can omit punctuation spans. Existing native
unit layers and segment boundaries remain authoritative. The aligner must preserve
text, IDs and ordering, return every requested segment and declare forced timing.
Wrong-language/unsupported aligners leave missing timing absent; malformed output,
consent/auth/quota/payload failures propagate. No timestamp interpolation occurs.
An advertised unit layer is treated as available when present; quality/coverage
acceptance for a particular aligner remains adapter-specific.

`Recognition::is_text_only` indicates any missing segment timing. `to_transcript`
returns captions' existing `Transcript`, or `TimingUnavailable` when any segment
is untimed. Non-word unit layers return `Unsupported` during conversion; callers
retain them through `segments()` pending captions' character-span work. No layer
is silently relabelled or dropped. Source samples convert once to ticks using
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
timing layers and metadata) deduplicate. Every other overlap, including equal-text
speech with different IDs or ranges, returns a typed ambiguity error, leaving chunk-boundary
reconciliation for an explicit retry/user policy. Untimed segments retain chunk
order and cannot be deduplicated by invented coordinates. There is no fuzzy text
matching or splitting of words at chunk boundaries.

## Verification and limits

`cargo test -p ai` exercises JSON native-timed and text-only fixtures through fake
adapters using the production seams: NFC Vietnamese, source offsets, missing-span
alignment, text-only refusal, capabilities/limits, consent/cost, malformed response,
translation, overlap ambiguity, redaction, typed failure and cancellation.
`cargo xtask ci` also checks architecture, formatting and Clippy.

Observed evidence covers offline contract behavior only. No model accuracy, timing
fidelity, network adapter, real billing, live streaming, native runtime performance
or platform behavior is established here. Adapters own response-parser allocation
caps before constructing transport records; engine owns job-level memory budgets.
