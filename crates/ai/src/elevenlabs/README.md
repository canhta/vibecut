# ElevenLabs Scribe transcription

`elevenlabs::ElevenLabsTranscriber` admits only `scribe_v2`, backend `elevenlabs` and the base URL `https://api.elevenlabs.io/v1`; proxies, lookalike hosts and other version paths are refused so upload consent stays bound to the official endpoint. Composition resolves `ELEVENLABS_API_KEY` (development) or the OS keychain entry and injects it as `transport::ApiKey`; it is sent as `xi-api-key` and never serialized or logged.

Uploads are multipart PCM16 WAV with `model_id=scribe_v2`, the exact `language_code` (`vi`, `en` or `zh`, no fallback) and `timestamps_granularity=word`. The provider allows 3 GB and 10 hours in standard mode; the shared 64 MiB PCM admission and engine chunking bound real invocations. Responses are capped at 1 MiB. Consent `max_bytes` bounds the encoded WAV including its 44-byte header.

Scribe returns word, spacing and audio-event tokens in seconds. The adapter folds spacing into the preceding token, keeps audio events as separate timed tokens, and converts each absolute boundary once to source samples with nearest-sample rounding. It emits one timed segment per invocation with stable window-derived IDs. Word tokens become `raw_tokens` and flow through captions' shared span code; zero-width punctuation stays untimed. No cue segmentation, interpolation or fallback.

`disclosure(scope)` names ElevenLabs, links its privacy policy and shows payload details and the cost bound; composition shows these before approval without restating provider terms. The adapter uses default logging and does not send `enable_logging=false`.

Typed failures: auth, payment/quota, rate limit, payload, server, network. The adapter never retries; engine owns network retry. Tests: `cargo test -p ai --test it elevenlabs::` (offline, see `tests/fixtures/elevenlabs/`). The ignored live test uses the same synthetic-speech probe as OpenAI.
