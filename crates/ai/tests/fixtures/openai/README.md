# OpenAI transcription fixtures

Offline only. Requests are never replayed against the network in ordinary CI.

- `transcribe-success.json` is a recorded Vietnamese `gpt-transcribe` response
  from the local cloud benchmark snapshot
  (`.spikes/asr-cloud/raw/gpt-transcribe__vi-fleurs.response`, 2026-11-04). The
  request id and every header were removed; the response carries no credentials.
- `transcribe-empty.json` covers silent/undetected speech.
- `error-*.json` are representative envelopes documented by OpenAI for invalid key,
  insufficient quota, rate limit, oversized payload and server failure. Messages
  are synthetic; only the public `type`/`code` fields drive classification.
- `error-malformed.json` is a truncated body for parser rejection.
- `request.json` records the multipart fields actually asserted for a Vietnamese
  upload, without headers, key material or audio bytes.

Refresh the recorded success fixture from a consented live run when the response
schema changes; update this note with the date and source.

## Live speech probe

The ignored live test prefers a bounded synthetic probe at
`target/fixtures/openai-review/speech.f32` (96,231 mono f32 samples, 16 kHz,
SHA-256 `567076be368500f40ae52ac6cda1d6a33084639342c69c7476dfbd837ea4f34b`).
If it is absent the test regenerates it on macOS with:

```sh
say -v Samantha -o target/fixtures/openai-live/speech.aiff \
  'The quick brown fox jumps over the lazy dog. Today we are testing speech recognition in a video editor.'
ffmpeg -hide_banner -loglevel error -y \
  -i target/fixtures/openai-live/speech.aiff \
  -ar 16000 -ac 1 -f f32le target/fixtures/openai-live/speech.f32
```

The probe is synthetic, generated locally, and never checked in. The test asserts
nine spoken anchors and logs only counts, never transcript text.
