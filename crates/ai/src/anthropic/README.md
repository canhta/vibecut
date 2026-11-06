# Anthropic translator

`anthropic::AnthropicTranslator` implements the same `Translator` contract and
`TranslationWindow` validation with Claude Sonnet 5.5 (`claude-sonnet-5-5`), mode
`context-window`; it exposes the same translation language tables. It admits only backend `anthropic` and the official base URL
`https://api.anthropic.com/v1`; proxies, lookalike hosts and other version paths are
refused. Composition resolves `ANTHROPIC_API_KEY` for development or the OS keychain
and injects it as `transport::ApiKey`; the key is sent as `x-api-key` with
`anthropic-version: 2026-10-21` and is never serialized or logged.

`anthropic::MessagesClient` is the reusable Messages transport for later LLM
operations: it owns the endpoint, headers, body cap and typed
wire errors. Translation sends synchronous Messages with `stream=false`,
`thinking={"type":"between_tools"}` (Sonnet 5.5 rejects `disabled`), an 8192
output-token ceiling, and a strict `output_config.format` JSON schema. Responses
must be one complete message with a text block, `end_turn`, no refusal and valid
IDs; thinking blocks are ignored. Schema joins transport transients as
engine-retryable; refusals, incomplete results, auth/quota/consent/cost failures do
not retry and no provider fallback is automatic.

The Anthropic disclosure names Anthropic and links its official data-retention
policy instead of restating retention or training terms; provider and model choice
belong to the user, and account-specific ZDR, retention and training settings remain
outside this adapter. Users must review the linked policy before granting consent.
Sources checked 2026-11-04:
[Messages](https://platform.claude.com/docs/en/api/messages),
[structured outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs),
[data retention](https://platform.claude.com/docs/en/manage-claude/api-and-data-retention),
[pricing](https://platform.claude.com/docs/en/about-claude/pricing).

The per-attempt USD upper bound is also 344,064 micro-USD (Sonnet 5.5 list prices
$2 input / $10 output per 1M tokens) with the same conservative byte ceiling. It is
deliberately loose, not a billing measurement or quality claim. Offline
`tests/it/anthropic.rs` covers six directions with explicit Hans/Hant targets, strict
wire/schema shape, context/glossary/style consent invalidation, numbers/glossary
validation, refusal/incomplete/oversized/error mapping, redaction and endpoint
admission. The ignored live six-direction test requires `VIBECUT_LIVE_AI=1` and
`ANTHROPIC_API_KEY`; a consented run is recorded in the ticket report. Prompt caching
is not enabled: windows are short, independent and below the useful cache threshold,
and no cached user content is needed.
