# ElevenLabs speech fixtures

Offline fixtures for `ai::elevenlabs_speech`. They exercise the production wire
seam only; they are not evidence of voice quality or account retention state.

## Audio

`provider-success.wav` is a **live provider capture**: a synthetic Vietnamese
sentence generated once through the production `UreqTransport` with
`eleven_flash_v2_5`, the premade Sarah voice `EXAVITQu4vr4xnSDxMaL`, and the key
from `.privates/.env`. It is 24 kHz mono PCM16, 52,384 samples (2.18 s, 104,812
bytes including the 44-byte WAV header). `provider-success.json` records the
sanitized request/response metadata; the API key never enters it. The provider's
`character-cost` header was 22 for 44 input characters (Flash bills half a unit per
character), while the documented per-character price gives an actual cost of 1,760
Micro-USD.

The ignored live test exports this capture when
`VIBECUT_ELEVENLABS_CAPTURE_DIR` is set; the committed file is the VI run. Replay:

```sh
set -a; . /Users/canh/Startup/VibeCut/.privates/.env; set +a
VIBECUT_LIVE_AI=1 VIBECUT_ELEVENLABS_VOICE_ID=EXAVITQu4vr4xnSDxMaL \
VIBECUT_ELEVENLABS_CAPTURE_DIR=/tmp/elevenlabs-capture \
cargo test -p ai --test elevenlabs_speech \
  live_elevenlabs_speech_uses_production_transport -- --ignored --nocapture
```

`success.wav` remains a synthetic 0.25 s 440 Hz sine probe (6,000 samples, 12,044
bytes) used by the short multi-chunk and failure tests; it is not a provider
capture. The endpoint's `pcm_24000` response is headerless little-endian PCM16, so
tests strip the WAV `data` chunk before replaying it as the response body. The WAV
container exists so the audio can be inspected with ordinary tools.

```sh
python3 - <<'PY'
import math, struct, wave
rate, n = 24000, 6000
samples = [
    int(round(0.25 * min(1.0, i / 240.0, (n - 1 - i) / 240.0)
              * 32767 * math.sin(2 * math.pi * 440.0 * i / rate)))
    for i in range(n)
]
with wave.open('success.wav', 'wb') as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(rate)
    w.writeframes(b''.join(struct.pack('<h', s) for s in samples))
PY
```

## JSON

| File | Covers |
|---|---|
| `provider-success.json` | live provider capture metadata; header units versus actual cost |
| `cost.json` | exact per-character prices, minimum one, grapheme versus scalar/UTF-16 count |
| `normalization.json` | `apply_text_normalization=auto`, `language_code` model support, fingerprint fields |
| `split.json` | model limit, newline/cue boundary, continuity context |
| `unsupported-language.json` | client-side language gate, including the garbled-VI Multilingual v2 case |
| `unsupported-voice.json` | explicit premade/stock allowlist, no fallback |
| `cancellation.json` | cancellation between split chunks |
| `error-auth.json`, `error-auth-400.json` | invalid key at 401 and 400 |
| `error-quota.json`, `error-quota-400.json`, `error-quota-401.json` | payment-required and documented `quota_exceeded` at 402/400/401 |
| `error-rate-limit.json` | 429 with `Retry-After` |
| `error-content-policy.json`, `error-payload.json` | content policy versus generic bad input |

The tests also cover provider completion boundaries that have no JSON fixture
because they are HTTP-shape checks: only status 200 is a complete body (201/204/206
are rejected), a declared `Content-Length` must match the received bytes, and a
non-BMP quote uses the larger UTF-16 count as a pre-call upper bound. Raw
`pcm_24000` has no container length, so the HTTP body length is the outer boundary
that a truncated response cannot bypass.

## Sources checked 2026-11-05

- <https://elevenlabs.io/docs/api-reference/text-to-speech/convert>
- <https://elevenlabs.io/docs/overview/models>
- <https://elevenlabs.io/docs/eleven-api/resources/zero-retention-mode>
- <https://elevenlabs.io/docs/help-center/technical/api-error-code-400-or-401>
- <https://elevenlabs.io/pricing/api>
