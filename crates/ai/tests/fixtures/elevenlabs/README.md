# ElevenLabs Scribe fixtures

Offline only; never replayed against the network.

- `transcribe-en.json`, `transcribe-vi.json`, `transcribe-zh.json` are recorded `scribe_v2` responses (from `.spikes/asr-cloud/raw/scribe-v2__{en-jfk,vi-fleurs,zh-fleurs}.response`) with headers, keys and request IDs removed. Transcript text stays because tests assert word/token correspondence.
- `error-*.json` use the documented `{"detail":{type,code,message,...}}` envelope with synthetic secrets.
- `error-malformed.json` is a truncated body.
- `request.json` records the asserted multipart fields of a Vietnamese upload, without headers, keys or audio.

Refresh the recorded responses from a consented live run when the schema changes.
