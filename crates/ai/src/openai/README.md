# OpenAI file transcription

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

`cargo test -p ai --test it openai::` runs offline against a recorded success response,
typed error envelopes, a malformed body and an injected fake transport. It covers
consent refusal before network, payload/provider consent invalidation, multipart and
WAV shape, stable IDs, disclosure fields, error categories, redaction and bounded
response handling. The ignored `live_openai_transcription_uses_production_transport`
requires `VIBECUT_LIVE_AI=1` and `OPENAI_API_KEY`, uploads a bounded synthetic-speech
probe and asserts nonempty text plus nine spoken anchors; see the fixture README for
the reproduction command. It is the only test that reaches the network.

## OpenAI speech adapter

`openai::OpenAiSpeechSynthesizer` is the OpenAI provider adapter for this contract.
It admits only `gpt-audio-1.5` over `POST /v1/chat/completions`
on the official OpenAI base URL, with `modalities: ["text", "audio"]`,
`audio: {voice, format: "wav"}`, `store: false` and a system instruction to read
the user text verbatim. Only the 13 built-in voice IDs are advertised; custom
voice objects and `/v1/audio/voices` are refused as voice cloning. Exact
language/script pairs are `vi`/`Latn`, `en`/`Latn` and `zh`/`Hans`; any other
pair returns `UnsupportedLanguage` before IO.

Text is NFC and at most the contract's 64 KiB. The returned transcript is
NFC-normalized and compared exactly, so a paraphrase, truncation or missing
transcript is `OutputRejected` and never placed. A valid `finish_reason` of
`stop` is required; `length` and unknown/missing statuses are `OutputRejected`
and `content_filter` is `ContentPolicy`. The completion ceiling is
`ceil(max_duration_micros * 20 / 1_000_000) + text_utf8_bytes + 64`, capped at
the model's 16,384 output tokens; a larger request returns `Payload` before IO.
The per-invocation estimate uses text UTF-8 bytes (a byte-BPE token represents
at least one byte, so this is an upper bound; graphemes are not) plus a fixed
framing allowance for input, and prices every completion token at the audio
rate. Actual cost is reconciled from reported prompt/completion token details,
including text output tokens, and must stay inside the approved quote.

The adapter accepts only mono 24 kHz 16-bit PCM WAV. It derives length from the
body when OpenAI writes the observed `0xFFFFFFFF` RIFF/data placeholders, but a
finite RIFF size must exactly cover the received body: larger is wire truncation
and smaller leaves trailing data, both `OutputRejected`. It rejects wrong
channels, rate, bit depth, odd or truncated chunk data. Decoded samples are
bounded by both the approved duration and the shared 64-MiB PCM cap. A base64
audio response can
exceed the 1 MiB text-response ceiling, so the shared transport exposes
`HttpTransport::send_bounded`; `UreqTransport` reads at most the requested byte
cap while other transports inherit a checked default. The injected transport
owns the call timeout (the shared `UreqTransport` uses 120 s; composition can
call `with_timeout`). Cancellation is checked before transport, after transport
and after decoding; there is no retry, provider fallback, project edit or
voice-listing call.

The consent policy names no training, abuse-monitoring logs up to 30 days,
Chat Completions audio application state up to 1 hour, organization-approved
Zero Data Retention and AI-generated voice disclosure; it never claims zero
retention. Offline `tests/it/openai_speech.rs` replays a recorded success fixture
and covers exact wire fields, NFC input, client-side language/voice gates,
transcript mismatch, incomplete/filtered/missing finish status, broken or
truncated WAV containers (including finite RIFF boundaries), Unicode byte-bound
quotes, payload/cost bounds, auth/quota/rate-limit/content-policy/invalid-voice/
server failures, redaction, cancellation and the bounded transport. The ignored
`live_openai_speech_uses_production_transport` requires `VIBECUT_LIVE_AI=1` and
`OPENAI_API_KEY` and sends three bounded synthetic VI/EN/ZH sentences.
