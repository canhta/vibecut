# OpenAI speech fixtures

Offline only. Requests are never replayed against the network in ordinary CI.

- `success.json` is a recorded `gpt-audio-1.5` Chat Completions audio response
  captured on 2026-11-05 with the then-current request shape (voice `marin`,
  `format: "wav"`, `store: false`, the verbatim-reading system instruction and
  the Vietnamese text in `request.json`). That recording used the pre-fix
  228-token completion allowance and completed with `finish_reason: "stop"`;
  the current byte-based allowance in `request.json` is 240 tokens. The
  transcript matched the request exactly. Request headers, the endpoint URL and
  the API key were not stored.
- `success.wav` is the decoded base64 audio from `success.json`: 24 kHz mono
  PCM16, 2.85 s. It keeps OpenAI's `0xFFFFFFFF` RIFF/data length placeholders,
  so the adapter must derive length from the body.
- `transcript-mismatch.json` and `missing-transcript.json` reuse the compact
  synthetic container from the earlier offline pass and break exact-text
  validation.
- `error-truncated-wav.json`, `error-wrong-rate.json` and `error-odd-data.json`
  break container integrity and must be rejected after a successful HTTP call.
- `error-*.json` are synthetic OpenAI error envelopes for auth, quota, rate
  limit, oversized payload, content policy, invalid voice and server failure.
  Only the public `type`/`code`/`param` fields drive classification; messages and
  keys are not logged.
- `error-malformed.json` is a truncated JSON body.
- `request.json` records the wire fields asserted for the recorded Vietnamese
  request, without headers, key material or audio bytes.

The ignored live test requires `VIBECUT_LIVE_AI=1` and `OPENAI_API_KEY`; it
sends only bounded synthetic VI/EN/ZH text through the production transport.
