# Contextual text translation

`Translator::translate` now accepts one `TranslationWindow`: stable segment IDs,
source/target language, before/after context, fixed glossary terms or names to
preserve, and style. The result maps every requested ID to a nonempty target text;
no extra/missing/duplicate IDs or partial success is admitted. Glossary rules are
checked by exact term/translation containment; spelling/case must match the supplied
term. Before/after context never receives output IDs. This replaces the earlier
single-text seam; it does not assign source timestamps to translated words.

A window admits 1..=64 segments, 16 context strings on each side, 128 unique glossary
terms, unique nonblank IDs up to 256 bytes and at most 32 KiB of canonical JSON.
Result text totals at most 64 KiB. `window.consent_text()` is the complete canonical
payload for `Scope.texts` (one entry); source/target languages must also match Scope.
Changing an ID, source text, context, glossary, style, script, provider or model
invalidates consent before transport. Text-only composition uses SampleRange [0,1)
and sample_rate 1 as Scope's non-audio sentinel; these fields do not assign timing.

`openai::OpenAiTranslator` uses the shared transport's ApiKey, HttpTransport and
UreqTransport, plus the OpenAI endpoint policy and typed HTTP errors.
`TRANSLATION_LANGUAGES` and `TRANSLATION_TARGET_LANGUAGES` expose the exact source
and target tags for offline capability composition; target Chinese remains an
explicit `zh-Hans`/`zh-Hant` choice. Its only admitted model is
`gpt-6.1-sol`, mode `context-window`, with exported TRANSLATION_SETTINGS. Only the
official OpenAI base URL is admitted. Exact languages are `vi`, `en`, `zh`,
`zh-Hans`, `zh-Hant` for source; target Chinese must select `zh-Hans` or `zh-Hant`
explicitly. All six ordered vi/en/zh directions are supported by the contract;
ASR-only models cannot construct this translator. Anthropic is the other recorded
product choice and is implemented in the [Anthropic translator](../anthropic/README.md).

Requests use Responses `text.format` strict JSON schema, no tools, synchronous
execution, `store=false`, `background=false`, standard service tier, and an 8192
output-token ceiling. Wire bodies cap at 64 KiB and responses at the existing
1 MiB transport ceiling. `structured::parse` is the shared envelope/JSON owner for
other structured operations: complete assistant text only, typed Schema,
Incomplete and Refused errors; operation owners validate IDs and text semantics.
Adapters make exactly one attempt. Schema joins transient transport failures as
engine-retryable; refusals, incomplete results, auth/quota/consent/cost failures
do not retry. No provider fallback is automatic.

Translation disclosure names OpenAI and links its official policy, alongside the
language pair, model, endpoint and cost. Composition shows the payload and these
fields before Consent::grant without restating provider terms. Sources checked
2026-11-04: [structured outputs](https://developers.openai.com/api/docs/guides/structured-outputs),
[model](https://developers.openai.com/api/docs/models/gpt-6.1-sol),
[data controls](https://developers.openai.com/api/docs/guides/your-data),
[pricing](https://developers.openai.com/api/docs/pricing).

The per-attempt USD upper bound is 344,064 micro-USD: conservative 131,072 input
tokens at $2/M plus 8192 output tokens at $10/M, standard short-context list prices.
This bounds the capped input bytes plus message/schema overhead rather than using
a characters-per-token estimate. It is deliberately loose; no actual billing or
translation-quality result is established by the bound. Engine adds the operation
attempt ceiling and aggregate authorization; provider code never retries or edits
projects. Digits-set mismatch yields cue-ID warnings for numbers/units review;
this is advisory, cannot validate translated units or distinguish equivalent dates,
and intentionally does not compare digit multiplicity.

Offline `tests/it/translation.rs` covers all direction/script request configurations,
strict wire shape, context/glossary/style consent invalidation, exact ID rules,
fixed terms, digit warnings, redaction, malformed/truncated/oversized/refused output,
unsupported ASR model construction and cancellation before/after transport.
Fixtures are synthetic, not captured live model quality evidence. The opt-in
`translate_eval` example consumes public windows, requires VIBECUT_LIVE_AI=1,
OPENAI_API_KEY and VIBECUT_TRANSLATE_EVAL_BUDGET_MICROS, checks the total single-attempt
ceiling before network, and writes completed outputs only to the requested local
evaluation file. It never retries; language quality/script fidelity requires live
outputs and independent human scoring in the local evaluation sheet.
