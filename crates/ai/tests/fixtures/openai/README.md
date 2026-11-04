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
