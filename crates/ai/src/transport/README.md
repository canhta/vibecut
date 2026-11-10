# Transport

`ApiKey`, `HttpRequest`, `HttpResponse`, `HttpTransport` and the bounded `UreqTransport` own the shared HTTPS call: redirects are bounded, keys never enter Debug/logs, and response bodies are capped before parsing. PCM16 WAV and multipart helpers serve speech adapters.

`TextModel` is the one structured text-model request path over that transport. `new` admits exactly one official backend, base URL, model and settings fingerprint; `send` builds the OpenAI Responses or Anthropic Messages body, checks the per-operation wire and consent byte ceilings, observes cancellation, and returns only the decoded JSON object. Operation owners (`chapters`, `fillers`) supply the prompt, schema name/schema and typed result validation; the transport never retries or switches provider.
