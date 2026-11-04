# Anthropic Messages fixtures

Offline only. Requests are never replayed against the network in ordinary CI.

- `translation-response.json` is a recorded-shape `claude-sonnet-5-5` Messages
  response with a structured `translations` text block, public authored text, and
  no request id, headers or credentials. It is refreshed from a consented live run
  when the response schema changes.
- Tests synthesize error envelopes and malformed/refusal responses inline; no
  secret, user transcript or provider request id is stored.

Refresh this fixture from a consented live run and update the date/source note.
