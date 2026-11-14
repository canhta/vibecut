# ElevenLabs providers

## Scribe transcription

`provider_config(model)` builds the one admitted configuration; `new` admits any bounded remote model ID, backend `elevenlabs`, the adapter's exact settings fingerprint and the base URL `https://api.elevenlabs.io/v1`, so proxies, lookalike hosts, other version paths and forged or stale settings identities are refused. Composition resolves the provider key and injects it as `transport::ApiKey`; it is sent as `xi-api-key` and never serialized or logged.

Uploads are multipart PCM16 WAV with the selected `model_id`, the exact `language_code` (`vi`, `en` or `zh`, no fallback) and `timestamps_granularity=word`. The provider allows 3 GB and 10 hours in standard mode; the shared 64 MiB PCM admission and engine chunking bound real invocations. Responses are capped at 1 MiB. Consent `max_bytes` bounds the encoded WAV including its 44-byte header.

Scribe returns word, spacing and audio-event tokens in seconds. The adapter folds spacing into the preceding token, keeps audio events as separate timed tokens, and converts each absolute boundary once to source samples with nearest-sample rounding. It emits one timed segment per invocation with stable window-derived IDs. Word tokens become `raw_tokens` and flow through captions' shared span code; zero-width punctuation stays untimed. No cue segmentation, interpolation or fallback.

`disclosure(scope)` returns the shared `ai::UploadDisclosure` with provider, model, language, duration, payload bytes, cost bound and the named policy/pricing/docs links; composition shows these before approval without restating provider terms. The adapter uses default logging and does not send `enable_logging=false`.

Typed failures: auth, payment/quota, rate limit, payload, server, network. The adapter never retries; engine owns network retry.

## ElevenLabs speech synthesis

`elevenlabs::speech::ElevenLabsSpeech` is the ElevenLabs adapter for the speech contract. It admits only backend `elevenlabs`, the base URL `https://api.elevenlabs.io/v1` and any bounded ID in `ProviderConfig::model`. Known profiles are `eleven_multilingual_v2` (EN/ZH) and `eleven_flash_v2_5` (VI/EN/ZH). Known unsupported language/model pairs return `UnsupportedLanguage` before transport; the adapter never switches model, voice or provider, and the VI gate is client-side. `config`, `capabilities` and `estimate` do no IO.

Requests are `POST /v1/text-to-speech/{voice_id}?output_format=pcm_24000` with the exact NFC text, `model_id`, `apply_text_normalization=auto` and optional `voice_settings.speed` in 0.7–1.2. `language_code` is sent only for Flash v2.5. The response is headerless little-endian PCM16 at 24 kHz, decoded to finite mono f32 and validated by the shared seam. Only a complete HTTP 200 body is accepted (other 2xx are rejected as partial), and a declared `Content-Length` must equal the received bytes, since raw PCM has no container length. Voices are an explicit premade/stock allowlist supplied by composition; cloned and professional-clone voices are never listed, selected or guessed.

The estimate is exact for BMP-only VI/EN/ZH input: 40 micro-USD per character for Flash v2.5, 80 for Multilingual v2, minimum one character. The provider does not say whether non-BMP text counts as scalars or UTF-16 units, so such text quotes the larger count as an upper bound. Graphemes are never used for cost. The `character-cost` header is recorded as `billable_characters` but is a billing-unit count, not a price: Flash reports half a unit per character, Multilingual v2 one. Actual cost is the input-character rate (40/80), not header x rate.

A library voice on a free plan is HTTP 402 `paid_plan_required`, typed `PaidPlanRequired` and kept apart from the 402 quota case, which stays `Quota`. Requests over the model limit (10,000 or 40,000 characters) split at cue (newline), sentence or whitespace boundaries. Adjacent chunks send bounded `previous_text`/`next_text` context. Audio is concatenated only after every chunk decodes and validates; cancellation between chunks discards all audio. The provider minimum is charged per request, so split cost is the sum of chunk characters.

The settings fingerprint binds output format, normalization, optional speed and logging mode; the per-request fingerprint also binds model, voice, language/script, continuity and split policy. `ElevenLabsSpeech::provider_config(model, settings)` builds the one admitted configuration; a different fingerprint is refused. Consent names default retention and model-improvement use with dashboard opt-out. Zero retention is requested only with `enable_logging=false`, is Enterprise-only and is reported as requested, never as verified.

`transport::HttpTransport::send_bounded` bounds an audio response while it is read. The cap derives from the approved duration and the shared 64 MiB decoded-PCM ceiling. `UreqTransport` enforces it during the read; the default trait method checks after buffering.

Live voice quality is unverified.

Sources checked 2026-11-05: [text-to-speech](https://elevenlabs.io/docs/api-reference/text-to-speech/convert), [models](https://elevenlabs.io/docs/overview/models), [zero retention](https://elevenlabs.io/docs/eleven-api/resources/zero-retention-mode), [pricing](https://elevenlabs.io/pricing/api).

Custom speech IDs use the conservative 10,000-character split bound with unknown pricing; unsupported capabilities are reported by the provider without fallback. Model defaults and explicit unknown-price consent belong to the [provider contract](../../foundation/contracts/README.md#remote-model-settings).
