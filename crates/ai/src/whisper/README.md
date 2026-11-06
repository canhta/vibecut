# Local Whisper

The default `whisper` feature uses safe whisper-rs bindings, with Metal on macOS
and CPU elsewhere. `whisper-vulkan` enables Vulkan (requires the Vulkan SDK).
`--no-default-features` retains offline contracts/model policy without native builds.
The narrowly admitted callback module is documented in
[ADR 0003](../../../../docs/adr/0003-whisper-callback-boundary.md); other ai modules
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
`~/Library/Caches/VibeCut/models`, Windows `%LOCALAPPDATA%/VibeCut/models` or Linux
`$XDG_CACHE_HOME/VibeCut/models` (default `~/.cache/VibeCut/models`), without
creating directories. Engine owns downloads, scheduling, resume/retry/checkpoints,
cache identity/storage/promotion and model-manager wiring; these are not implemented
by this crate.

The existing `transport::UreqTransport` also implements `DownloadTransport` for
streaming GET responses, retaining duplicate response headers for strict engine
validation. It uses the same agent/timeouts; text provider responses keep their
1 MiB buffer cap. Large media responses opt in through `send_bounded`, which reads
at most the caller's byte ceiling; the trait default checks an already-buffered
response for in-memory test transports. `download_redirect` resolves only bounded
HTTPS URLs without embedded
credentials or fragments. The engine owns redirect limits and Range/ETag policy.
Catalog validation admits bounded portable raw filenames, rejects device names,
case-folded duplicates and unsafe paths, and caps file/aggregate sizes before IO.
The [engine transfer contract](../../../engine/src/providers/README.md#local-model-transfers) owns
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

Observed corpus quality and cancellation evidence is recorded in the local review-fix report. The former independent 30 s policy damaged Vietnamese speech;
continuous windows recover it. Continuous beam search worsened English noise/music;
English greedy decoding reduces its aggregate error count and music omissions while
preserving the established Vietnamese/Chinese settings. Disabling intra-call text
history worsened English and was rejected. Flash attention remains disabled after
an observed Vietnamese regression. These corpus observations do not establish
universal model accuracy or human timing fidelity. Windows/Vulkan and speech-boundary
reconciliation beyond the invocation ceiling remain unverified.
