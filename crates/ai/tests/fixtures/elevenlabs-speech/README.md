# ElevenLabs speech fixtures

Offline fixtures for `ai::elevenlabs_speech`. They exercise the wire seam only, not voice quality or account retention.

## Audio

`provider-success.wav` is a live capture of a synthetic Vietnamese sentence through the production `UreqTransport` with `eleven_flash_v2_5` and premade voice `EXAVITQu4vr4xnSDxMaL`: 24 kHz mono PCM16, 52,384 samples (2.18 s). `provider-success.json` holds sanitized request/response metadata: `character-cost` 22 for 44 input characters, actual cost 1,760 micro-USD.

The ignored live test exports a capture when `VIBECUT_ELEVENLABS_CAPTURE_DIR` is set. From the main checkout:

```sh
set -a; . .privates/.env; set +a
VIBECUT_LIVE_AI=1 VIBECUT_ELEVENLABS_VOICE_ID=EXAVITQu4vr4xnSDxMaL \
VIBECUT_ELEVENLABS_CAPTURE_DIR=/tmp/elevenlabs-capture \
cargo test -p ai --test it \
  elevenlabs_speech::live_elevenlabs_speech_uses_production_transport -- --ignored --nocapture
```

`success.wav` is a synthetic 0.25 s 440 Hz sine (6,000 samples) for short multi-chunk and failure tests. The endpoint's `pcm_24000` response is headerless PCM16, so tests strip the WAV `data` chunk before replaying it as the body. Sample `i` is round-to-nearest-even of `0.25 × min(1, i / 240, (5999 - i) / 240) × 32767 × sin(2π × 440 × i / 24000)`.

## JSON

| File | Covers |
|---|---|
| `provider-success.json` | live capture metadata; header units versus actual cost |
| `cost.json` | per-character prices, minimum one, grapheme versus scalar/UTF-16 count |
| `normalization.json` | `apply_text_normalization=auto`, `language_code` model support, fingerprint fields |
| `split.json` | model limit, newline/cue boundary, continuity context |
| `unsupported-language.json` | client-side language gate, including garbled-VI Multilingual v2 |
| `unsupported-voice.json` | premade/stock allowlist, no fallback |
| `cancellation.json` | cancellation between split chunks |
| `error-auth.json`, `error-auth-400.json` | invalid key at 401 and 400 |
| `error-quota.json`, `error-quota-400.json`, `error-quota-401.json` | payment required and `quota_exceeded` at 402/400/401 |
| `error-rate-limit.json` | 429 with `Retry-After` |
| `error-content-policy.json`, `error-payload.json` | content policy versus generic bad input |

HTTP-shape checks without JSON fixtures: only status 200 is complete (201/204/206 rejected), a declared `Content-Length` must match the received bytes, and a non-BMP quote uses the larger UTF-16 count.
