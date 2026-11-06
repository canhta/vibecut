# Consent and cost

`Scope` binds exact provider configuration (including endpoint/model/settings),
operation, mode, language pair, source content fingerprint, sample rate, sample
window, upload byte ceiling and exact NFC text payloads. The caller owns content
hashing and audio preparation and must provide an honest source content identity.
Text is matched by value, not an unverified caller-supplied digest. `Consent::grant`
is called by the composition root only after affirmative approval; local requests
need no upload approval. Chunks/retries may shrink the approved window and byte
ceiling; changed provider, payload, text or operation requires another token.
Remote alignment requires its own approval; it never inherits transcription
approval. No provider selection or remote fallback happens automatically. `ProviderSelection`
stores one operation, a primary and an explicitly configured ordered list of up to
seven distinct fallback configurations. New selections and missing serialized
fallback lists default to no fallback. Schema version 1 and duplicate/size checks
apply on deserialization; upload approvals are never persisted with settings.
Engine owns [fallback execution](../../../engine/README.md#provider-selection-and-fallback).

`Money` uses integer millionths and a three-letter uppercase currency. Estimates
are free, an upper bound, or unknown. Remote execution refuses unknown estimates,
a currency mismatch and bounds exceeding approval. A budget is **per invocation**;
engine owns aggregate job/retry/chunk reservation and must deduct/reserve budget
before calling this seam again. Tokens are in-memory capabilities, not a security
sandbox for arbitrary adapter implementations.
