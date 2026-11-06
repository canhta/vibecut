# ElevenLabs Scribe transcription

`elevenlabs::ElevenLabsTranscriber` admits only `scribe_v2`, backend `elevenlabs` and
the official base URL `https://api.elevenlabs.io/v1`; proxies, lookalike hosts and
other version paths are refused to keep upload consent bound to the official
provider endpoint.
The composition root resolves `ELEVENLABS_API_KEY` for development or the OS keychain
entry, then injects it as `transport::ApiKey`; the key is sent as `xi-api-key` and is
never serialized or logged.

Uploads are multipart PCM16 WAV with `model_id=scribe_v2`, the exact `language_code`
(`vi`, `en` or `zh`, no implicit fallback) and `timestamps_granularity=word`. The
documented limits are 3 GB and 10 hours in standard mode; the shared 64 MiB PCM
admission and engine chunking bound actual invocations, and responses are capped at
1 MiB. Consent's `max_bytes` bounds the encoded WAV, including its 44-byte header.

Scribe returns word, spacing and audio-event tokens in seconds. The adapter folds
spacing into the preceding token, preserves audio events as separate timed tokens,
and converts each absolute token boundary once to source samples with explicit
nearest-sample rounding, emitting one
timed segment per invocation with stable window-derived IDs. Word tokens become
`raw_tokens` and flow through captions' shared span code, preserving
raw-to-NFC-correspondence and the provider's per-language token identity; zero-width
punctuation stays explicitly untimed. No cue segmentation, timestamp interpolation
or provider fallback happens here.

`disclosure(scope)` names ElevenLabs and links its official privacy policy, with
payload details and the invocation cost bound. Composition shows these before
approval without restating the provider's retention or training terms. The adapter
uses default logging and does not send `enable_logging=false`.

Typed failures cover auth, payment/quota, rate limit, payload, server and network;
the adapter never retries, and the engine owns the bounded network retry policy.
`cargo test -p ai --test it elevenlabs::` runs offline against recorded English,
Vietnamese and Chinese responses plus synthetic errors, consent and disclosure
tests. The ignored live test uses the same bounded synthetic-speech probe as OpenAI;
a separate consented run on the English corpus sample is recorded in the
ticket report.
