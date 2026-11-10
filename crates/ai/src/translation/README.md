# Contextual text translation

`Translator::translate` takes one `TranslationWindow`: stable segment IDs, source/target language, before/after context, glossary terms and style. The result maps every requested ID to a nonempty target text; extra, missing or duplicate IDs and partial success are refused. Glossary rules are checked by exact containment, spelling and case included. Context never receives output IDs. Translated words get no source timestamps.

A window has 1..=64 segments, 16 context strings per side, 128 unique glossary terms, unique nonblank IDs up to 256 bytes and at most 32 KiB of canonical JSON. Result text totals at most 64 KiB. `window.consent_text()` is the complete payload for `Scope.texts` (one entry); languages must match Scope. Any change to ID, text, context, glossary, style, script, provider or model invalidates consent. Text-only Scope uses SampleRange [0,1) and sample_rate 1 as a non-audio sentinel.

`openai::OpenAiTranslator` admits only `gpt-6.1-sol`, mode `context-window`, `TRANSLATION_SETTINGS` and the official OpenAI base URL. `TRANSLATION_LANGUAGES` and `TRANSLATION_TARGET_LANGUAGES` expose the exact tags: sources `vi`, `en`, `zh`, `zh-Hans`, `zh-Hant`; a Chinese target must be `zh-Hans` or `zh-Hant`. All six ordered vi/en/zh directions are supported. ASR-only models cannot construct it. The Anthropic implementation is in the [Anthropic translator](../anthropic/README.md).

Requests use Responses `text.format` strict JSON schema, no tools, synchronous, `store=false`, `background=false`, standard tier, 8192 output tokens. Bodies cap at 64 KiB, responses at 1 MiB. `structured::parse` owns the shared envelope: complete assistant text only, typed Schema, Incomplete and Refused errors; operation owners validate IDs and text.

One attempt only. Schema joins transient transport failures as engine-retryable; refusals, incomplete results and auth/quota/consent/cost failures do not retry. No automatic fallback.

Disclosure returns the shared `ai::UploadDisclosure` with the language pair, model, payload bytes, cost bound and the provider's policy link; composition shows these and the payload before `Consent::grant`.

The per-attempt upper bound is 344,064 micro-USD: 131,072 input tokens at $2/M plus 8192 output tokens at $10/M. It bounds the capped input bytes plus overhead and is deliberately loose. Engine adds the attempt ceiling and aggregate authorization. A digit-set mismatch yields advisory cue-ID warnings; it does not validate units or equivalent dates or compare digit multiplicity.

The `translate_eval` example checks the total ceiling before network, never retries and writes only to the requested local file. Quality needs live output and independent human scoring.
