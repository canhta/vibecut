# Provider contracts

`ProviderConfig::new` and serde validate schema version 1, nonempty provider, model/settings identity and tagged `location` (`kind: local` or `remote`). Remote URLs require HTTPS and reject embedded credentials, query and fragments. Backend names are extensible data. `auth_ref` is an external secret-store reference, never a key. The settings fingerprint includes every option affecting recognition. Configuration serializes for persistence; Debug redacts endpoint, model reference and auth reference through `NfcText`. Tokens cannot be serialized or deserialized.

`Capabilities` describes an exact model, mode, operation and locale/script. The crate has no built-in language list or implicit locale fallback. Timing units are a resolved set of supported segment/word/syllable/character/grapheme/token capabilities; the adapter omits unresolved conditional timing. Other feature flags distinguish supported/unsupported/unknown/conditional. Timing evidence is advertised, fixture-verified or live-verified, with a revision, and does not prove accuracy. Upload limits are direct invocation bytes and duration in integer microseconds; `None` means unknown, never unlimited or zero. Execution passes complete mono audio windows; streaming flags describe adapters without a streaming job protocol. Unsupported capabilities and locale pairs fail before execution.
## Consent and cost

`Scope` binds exact provider configuration (endpoint/model/settings), operation, mode, language pair, source content fingerprint, sample rate, sample window, upload byte ceiling and exact NFC text payloads. The caller owns content hashing and audio preparation and provides an honest source content identity. Text is matched by value. The composition root calls `Consent::grant` only after affirmative approval; local requests need no upload approval.

Chunks and retries may shrink the approved window and byte ceiling. A changed provider, payload, text or operation needs a new token. Remote alignment needs its own approval and never inherits transcription approval. Provider selection and remote fallback are never automatic.

`ProviderSelection` stores one operation, a primary and an explicitly configured ordered list of up to seven distinct fallback configurations. New selections and missing serialized fallback lists default to no fallback. Deserialization checks schema version 1, duplicates and size. Upload approvals are never persisted with settings. Engine owns [fallback execution](../../../engine/src/providers/README.md#provider-selection-and-fallback).

`Money` uses integer millionths and a three-letter uppercase currency. Estimates are free, an upper bound, or unknown. Remote execution refuses unknown estimates, currency mismatch and bounds above the approval. A budget is per invocation; engine owns aggregate job/retry/chunk reservation and reserves budget before calling this seam again. Tokens are in-memory capabilities, not a sandbox for arbitrary adapters.

## Disclosure

`ai::UploadDisclosure` is the one provider-neutral consent value: operation, provider, model, request languages, audio duration in microseconds, exact wire payload bytes, the cost ceiling and named policy/documentation links. Remote-capable traits return it from `disclosure`: `Transcriber`, `Translator`, `clips::ClipSuggester`, `chapters::ChapterGenerator` and `fillers::FillerClassifier` take the exact `Scope`; `SpeechSynthesizer` takes its `SpeechRequest`. Local adapters inherit the default and return `Ok(None)`. Adapters keep their endpoint, request form, response parser and price table; the shared [remote core](../remote.rs) owns constructor admission, scope binding, wire arithmetic and error classification.

