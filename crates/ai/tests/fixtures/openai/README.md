# OpenAI transcription fixtures

Offline only; never replayed against the network.

- `transcribe-success.json` is a recorded Vietnamese `gpt-transcribe` response (from `.spikes/asr-cloud/raw/gpt-transcribe__vi-fleurs.response`) with request id and headers removed.
- `transcribe-empty.json` covers silent or undetected speech.
- `error-*.json` are OpenAI-documented envelopes for invalid key, insufficient quota, rate limit, oversized payload and server failure. Messages are synthetic; only `type`/`code` drive classification.
- `error-malformed.json` is a truncated body.
- `request.json` records the asserted multipart fields of a Vietnamese upload, without headers, keys or audio.

Refresh the success fixture from a consented live run when the response schema changes.

## Live speech probe

The ignored live test prefers `target/fixtures/openai-review/speech.f32` (96,231 mono f32 samples, 16 kHz, SHA-256 `567076be368500f40ae52ac6cda1d6a33084639342c69c7476dfbd837ea4f34b`). If absent, it regenerates the probe on macOS:

```sh
say -v Samantha -o target/fixtures/openai-live/speech.aiff \
  'The quick brown fox jumps over the lazy dog. Today we are testing speech recognition in a video editor.'
ffmpeg -hide_banner -loglevel error -y \
  -i target/fixtures/openai-live/speech.aiff \
  -ar 16000 -ac 1 -f f32le target/fixtures/openai-live/speech.f32
```

The probe is never checked in. The test asserts nine spoken anchors and logs only counts.
