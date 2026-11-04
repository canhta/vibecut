# ElevenLabs Scribe fixtures

Offline only. Requests are never replayed against the network in ordinary CI.

- `transcribe-en.json`, `transcribe-vi.json`, `transcribe-zh.json` are recorded
  `scribe_v2` responses from the local cloud benchmark snapshots
  (`.spikes/asr-cloud/raw/scribe-v2__{en-jfk,vi-fleurs,zh-fleurs}.response`,
  2026-11-04). Headers, keys and request IDs were removed; transcript text remains
  because the tests assert word/token correspondence.
- `error-*.json` use the documented `{"detail":{type,code,message,...}}` envelope
  with synthetic secrets to prove typed errors and redaction.
- `error-malformed.json` is a truncated body for parser rejection.
- `request.json` records the multipart fields asserted for a Vietnamese upload,
  without headers, keys or audio bytes.

Refresh the recorded responses from a consented live run when the schema changes;
update this note with the date and source.
