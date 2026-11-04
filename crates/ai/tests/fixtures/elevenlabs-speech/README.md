# ElevenLabs speech fixtures

Offline fixtures for `ai::elevenlabs_speech`. They exercise the production wire
seam only; they are not evidence of voice quality, live billing or account state.

## Audio

`success.wav` is a synthetic 24 kHz mono PCM16 sine probe (440 Hz, 0.25 s, 6,000
samples, 12,044 bytes including the 44-byte header), not a provider capture. The
endpoint's `pcm_24000` response is headerless little-endian PCM16, so the test
strips the WAV `data` chunk before replaying it as the response body. The WAV
container exists so the recorded audio can be inspected with ordinary audio tools.

Reproduce:

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

No live key was available while the adapter was built, so this probe is synthetic;
the ignored live test is the only path that captures real provider audio.

## JSON

| File | Covers |
|---|---|
| `success.json` | text, model, language, character-cost header, sample count |
| `cost.json` | exact per-character prices, minimum one, grapheme versus scalar/UTF-16 count |
| `normalization.json` | `apply_text_normalization=auto`, `language_code` model support, fingerprint fields |
| `split.json` | model limit, newline/cue boundary, continuity context |
| `unsupported-language.json` | client-side language gate, including the garbled-VI Multilingual v2 case |
| `unsupported-voice.json` | explicit premade/stock allowlist, no fallback |
| `cancellation.json` | cancellation between split chunks |
| `error-*.json` | auth, quota/payment, rate limit and content policy envelopes |

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
- <https://elevenlabs.io/pricing/api>
