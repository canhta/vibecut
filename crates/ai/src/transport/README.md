# Transport

`ApiKey`, `HttpRequest`, `HttpResponse`, `HttpTransport` and the bounded `UreqTransport` own the shared HTTPS call: provider redirects are refused, download redirects are bounded, keys never enter Debug/logs, and response bodies are capped before parsing. PCM16 WAV and multipart helpers serve transcription and speech adapters.

`TextModel` is the one structured text-model request path over that transport. `new` admits exactly one official backend, base URL, model and settings fingerprint; `send` takes one `TextRequest` (prompt name/instructions, schema, exact payload, stage, hooks and byte ceiling), builds the OpenAI Responses or Anthropic Messages body, checks the per-operation wire and consent byte ceilings, observes cancellation, and returns only the decoded JSON object. Operation owners (`chapters`, `fillers`) supply the prompt, schema name/schema and typed result validation; the transport never retries or switches provider.
