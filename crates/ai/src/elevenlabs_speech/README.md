# ElevenLabs speech synthesis

`elevenlabs_speech::ElevenLabsSpeech` is the ElevenLabs adapter for the speech
contract. It admits only backend `elevenlabs`, the official base URL
`https://api.elevenlabs.io/v1`, and the two documented TTS models:
`eleven_multilingual_v2` (EN/ZH) and `eleven_flash_v2_5` (VI/EN/ZH). The
selection is explicit in `ProviderConfig::model`; unsupported language/model
combinations return `UnsupportedLanguage` before transport and the adapter never
switches model, voice or provider. Multilingual v2 returned HTTP 200 with garbled
Vietnamese audio during research, so the VI gate is deliberately client-side.
The adapter is free of IO in `config`, `capabilities` and `estimate`.

Requests are `POST /v1/text-to-speech/{voice_id}?output_format=pcm_24000` with the
exact NFC text, `model_id`, `apply_text_normalization=auto` and optional
`voice_settings.speed` in the documented 0.7–1.2 range. `language_code` is sent
only for Flash v2.5, which documents it; Multilingual v2 does not. The provider
returns headerless little-endian PCM16 at 24 kHz, decoded to finite mono f32 and
validated by the shared seam. Only a complete HTTP 200 body is accepted; other
2xx statuses are rejected as partial/incomplete, and a declared `Content-Length`
must equal the received bytes because raw PCM has no container length of its own.
Premade/stock voices are an explicit allowlist
supplied by composition; cloned and professional-clone voices are never listed,
selected or guessed. There is no automatic fallback.

The estimate is exact for BMP-only VI/EN/ZH input: 40 micro-USD per character for
Flash v2.5 and 80 for Multilingual v2, minimum one character. Because the provider
does not publish whether non-BMP text is counted as scalars or UTF-16 units, such
text quotes the larger count as an upper bound, so the approved budget still bounds
pre-call spending. Graphemes are never used for cost. The `character-cost` response
header is recorded as `billable_characters` when present, but it is the provider's
billing-unit count, not the dollar price: the live Flash capture reports 22 units
for a 44-character synthetic probe, while Multilingual v2 reports one unit per
character. Actual cost therefore follows the documented input-character rate
(40/80 micro-USD), not header x rate; multiplying the Flash header by the
per-character price would underreport by half. Requests longer than the model limit
(10,000 or 40,000 characters) split at cue (newline), sentence or whitespace
boundaries. Adjacent chunks send bounded
`previous_text`/`next_text` continuity context, and audio is concatenated only
after every chunk decodes and validates; cancellation between chunks discards all
audio. The provider minimum is charged per request, so split cost is the sum of
chunk characters.

The settings fingerprint binds the output format, normalization, optional speed
and logging mode, and the per-request fingerprint additionally binds the exact
model, voice, language/script, continuity and split policy. Composition must put
`ElevenLabsSpeechSettings::settings_fingerprint()` in the provider configuration;
a different value is refused. Consent names default retention and
model-improvement use with dashboard opt-out. Zero retention is requested only
with `enable_logging=false`, is Enterprise-only and is reported as requested, not
verified; the adapter never claims it took effect.

`transport::HttpTransport::send_bounded` lets an adapter bound an audio response
while the production transport reads it. The speech cap is derived from the
approved duration and the shared 64 MiB decoded-PCM ceiling, so a hostile or
runaway response cannot allocate past the approved invocation. The default trait
method checks after buffering for simple implementations; `UreqTransport`
enforces the ceiling during the read.

`cargo test -p ai --test it elevenlabs_speech::` replays synthetic 24 kHz PCM (stored
as `tests/fixtures/elevenlabs-speech/success.wav`, replayed as its raw `data`
chunk), recorded JSON envelopes for auth, quota, rate limit and content policy,
and fixtures for cost, normalization, unsupported language/voice, split
continuity, malformed/oversized audio and cancellation. No live network runs in
CI. The ignored `live_elevenlabs_speech_uses_production_transport` requires
`VIBECUT_LIVE_AI=1`, `ELEVENLABS_API_KEY` and a premade/stock
`VIBECUT_ELEVENLABS_VOICE_ID`, synthesizes one bounded synthetic probe each in VI,
EN and ZH, and is the only network path. No live request was run while the
adapter was built because no key was available; live output and voice quality
remain unverified.

Sources checked 2026-11-05:
[text-to-speech](https://elevenlabs.io/docs/api-reference/text-to-speech/convert),
[models](https://elevenlabs.io/docs/overview/models),
[zero retention](https://elevenlabs.io/docs/eleven-api/resources/zero-retention-mode),
[pricing](https://elevenlabs.io/pricing/api).
