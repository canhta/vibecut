# Transcript cache

`transcript` is a serialized `captions::Transcript` with source word or span
timing. The harness caches it under
`target/clip-eval/cache/transcripts/<source-sha>-<transcript-sha>.json`; a
prepared cache named `<source-sha>.json` is used when a label omits `transcript`.
Without either, `--transcribe` runs Scribe and caches
`<source-sha>-elevenlabs-scribe_v2.json`. Missing/mismatched hashes, a missing
transcript without `--transcribe`, and a language mismatch are typed errors before
pipeline execution.
