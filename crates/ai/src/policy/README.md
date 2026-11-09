# Consent and cost

`Scope` binds exact provider configuration (endpoint/model/settings), operation, mode, language pair, source content fingerprint, sample rate, sample window, upload byte ceiling and exact NFC text payloads. The caller owns content hashing and audio preparation and provides an honest source content identity. Text is matched by value. The composition root calls `Consent::grant` only after affirmative approval; local requests need no upload approval.

Chunks and retries may shrink the approved window and byte ceiling. A changed provider, payload, text or operation needs a new token. Remote alignment needs its own approval and never inherits transcription approval. Provider selection and remote fallback are never automatic.

`ProviderSelection` stores one operation, a primary and an explicitly configured ordered list of up to seven distinct fallback configurations. New selections and missing serialized fallback lists default to no fallback. Deserialization checks schema version 1, duplicates and size. Upload approvals are never persisted with settings. Engine owns [fallback execution](../../../engine/src/providers/README.md#provider-selection-and-fallback).

`Money` uses integer millionths and a three-letter uppercase currency. Estimates are free, an upper bound, or unknown. Remote execution refuses unknown estimates, currency mismatch and bounds above the approval. A budget is per invocation; engine owns aggregate job/retry/chunk reservation and reserves budget before calling this seam again. Tokens are in-memory capabilities, not a sandbox for arbitrary adapters.
