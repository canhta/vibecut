# OpenAI speech fixtures

Offline only; never replayed against the network.

- `success.json` is a recorded `gpt-audio-1.5` Chat Completions audio response (voice `marin`, `format: "wav"`, `store: false`, verbatim-reading system instruction, Vietnamese text in `request.json`), completed with `finish_reason: "stop"` and an exact transcript. Headers, endpoint URL and key were not stored.
- `success.wav` is the decoded audio from `success.json`: 24 kHz mono PCM16, 2.85 s, with OpenAI's `0xFFFFFFFF` RIFF/data length placeholders.
- `transcript-mismatch.json` and `missing-transcript.json` use a compact synthetic container and break exact-text validation.
- `error-truncated-wav.json`, `error-wrong-rate.json` and `error-odd-data.json` break container integrity and must be rejected after a successful HTTP call.
- Other `error-*.json` are synthetic envelopes for auth, quota, rate limit, oversized payload, content policy, invalid voice and server failure. Only `type`/`code`/`param` drive classification.
- `error-malformed.json` is a truncated JSON body.
- `request.json` records the asserted wire fields, without headers, keys or audio.

The ignored live test needs `VIBECUT_LIVE_AI=1` and `OPENAI_API_KEY` and sends bounded synthetic VI/EN/ZH text.
