# Provider contracts

`ProviderConfig::new` and serde validate schema version 1, nonempty provider, model/settings identity and tagged `location` (`kind: local` or `remote`). Remote URLs require HTTPS and reject embedded credentials, query and fragments. Backend names are extensible data. `auth_ref` is an external secret-store reference, never a key. The settings fingerprint includes every option affecting recognition. Configuration serializes for persistence; Debug redacts endpoint, model reference and auth reference through `NfcText`. Tokens cannot be serialized or deserialized.

`Capabilities` describes an exact model, mode, operation and locale/script. The crate has no built-in language list or implicit locale fallback. Timing units are a resolved set of supported segment/word/syllable/character/grapheme/token capabilities; the adapter omits unresolved conditional timing. Other feature flags distinguish supported/unsupported/unknown/conditional. Timing evidence is advertised, fixture-verified or live-verified, with a revision, and does not prove accuracy. Upload limits are direct invocation bytes and duration in integer microseconds; `None` means unknown, never unlimited or zero. Execution passes complete mono audio windows; streaming flags describe adapters without a streaming job protocol. Unsupported capabilities and locale pairs fail before execution.
## Consent and cost

`Scope` binds exact provider configuration (endpoint/model/settings), operation, mode, language pair, source content fingerprint, sample rate, sample window, upload byte ceiling and exact NFC text payloads. The caller owns content hashing and audio preparation and provides an honest source content identity. Text is matched by value. The composition root calls `Consent::grant` only after affirmative approval; local requests need no upload approval.

Chunks and retries may shrink the approved window and byte ceiling. A changed provider, payload, text or operation needs a new token. Remote alignment needs its own approval and never inherits transcription approval. A provider is never chosen and no remote fallback runs automatically.

Each operation has one `Scope` constructor (`transcription`, `alignment`, `translation`, `clip_suggestion`, `chapters`, `fillers`) that owns its mode, sample rate and wire bound; the transcription bound is the PCM16 WAV the adapter encodes (two bytes per sample plus a 44-byte header). `Transcriber::detect_language` is an optional capability that returns `Unsupported` unless an adapter overrides it. Upload approvals are never persisted with settings.

`Money` uses integer millionths and a three-letter uppercase currency. Estimates are free, an upper bound, or unknown. Remote execution refuses unknown estimates unless the caller separately grants an `UnknownPriceAcknowledgement` for that exact provider configuration and operation. `Consent::grant_unknown_price` carries no money ceiling; ordinary money approval cannot authorize unknown pricing. Known estimates refuse currency mismatch and bounds above the approval. A budget is per invocation; engine owns aggregate job/retry/chunk reservation and reserves budget before calling this seam again. Tokens are in-memory capabilities, not a sandbox for arbitrary adapters.

## Disclosure

`ai::UploadDisclosure` is the one provider-neutral consent value: operation, provider, model, request languages, audio duration in microseconds, exact wire payload bytes, the cost ceiling and named policy/documentation links. Remote-capable traits return it from `disclosure`: `Transcriber`, `Translator`, `clips::ClipSuggester`, `chapters::ChapterGenerator` and `fillers::FillerClassifier` take the exact `Scope`; `SpeechSynthesizer` takes its `SpeechRequest`. Local adapters inherit the default and return `Ok(None)`. Adapters keep their endpoint, request form and response parser; [`defaults`](../remote/defaults.rs) owns the shipped model/feature choices and known rates; the shared [remote core](../remote.rs) owns constructor admission, scope binding, wire arithmetic and error classification.


## Remote model settings

`defaults::validate_model` admits nonblank IDs of at most 256 UTF-8 bytes with no
control characters. Adapters send the chosen ID unchanged; they never use the
shipped table as an allowlist. The remote settings fingerprint is `sha256:` plus
64 lowercase hex digits over the canonical JSON tuple `("remote-settings-v1",
adapter settings, NFC model ID)`. Construction and admission use this same
identity, binding every component without exceeding workflow cache-version bounds.
`ModelUnavailable` retains the selected provider/model and a static diagnostic
category derived from the provider response. Raw provider messages stay out of
errors and logs, as required by [standards](../../../../../docs/engineering/standards.md#logging).
Unknown models keep the same token, request, response and audio bounds.

Known text quotes retain the previous conservative $2/$10 per million-token floor;
a lower published rate does not silently reduce an existing approval ceiling.
ASR quoting retains the existing conservative ceiling, including ElevenLabs' $0.22/h
ceiling (its published Scribe v2 price is $0.20/h). Unknown prices remain unknown,
including actual speech cost when the chosen model has no registered rate.

Defaults and rates checked 2026-11-11 against official [OpenAI pricing](https://developers.openai.com/api/docs/pricing),
[OpenAI transcription](https://developers.openai.com/api/docs/models/gpt-transcribe),
[OpenAI audio](https://developers.openai.com/api/docs/models/gpt-audio-1.5),
[Claude Sonnet](https://platform.claude.com/docs/en/models/sonnet-5-5/overview),
[ElevenLabs models](https://elevenlabs.io/docs/overview/models) and
[ElevenLabs pricing](https://elevenlabs.io/pricing/api).
`list_remote_models` makes one explicit authenticated GET using the official
[OpenAI](https://developers.openai.com/api/reference/resources/models/methods/list),
[Anthropic](https://platform.claude.com/docs/en/api/models/list) or
[ElevenLabs](https://elevenlabs.io/docs/api-reference/models/list) list endpoint.
It accepts at most 1,000 IDs and 1 MiB, deduplicates/sorts, checks cancellation
before/after IO, never retries and never fetches pages or weights automatically.
