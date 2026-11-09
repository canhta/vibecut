# Anthropic Messages fixtures

Offline only; never replayed against the network.

- `translation-response.json` is a recorded-shape `claude-sonnet-5-5` Messages response with a structured `translations` text block and public authored text. It holds no request id, headers or credentials.
- Error envelopes and malformed or refusal responses are synthesized inline in the tests.

Refresh `translation-response.json` from a consented live run when the response schema changes.
