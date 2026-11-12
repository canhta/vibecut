# Caption emphasis selection

`EmphasisInput` is an ordered, bounded set of caption cues with verified word timing: each segment names its caption item and cue (`<item>:<cue>`), carries the cue text and one `{position, text}` per verified word, where `position` is the word's index in the cue's verified word records. Settings carry the language (vi/en/zh/zh-Hans/zh-Hant) and `max_per_cue` (1 to `captions::MAX_EMPHASIS_WORDS`). Input is at most 4,096 segments, 50,000 words and 96 KiB of canonical JSON; empty, unsupported, duplicate, blank or oversized input refuses with a typed `EmphasisRefusal` before consent.

`EmphasisSelector` is the blocking single-attempt seam. `OpenAiEmphasisSelector` and `AnthropicEmphasisSelector` build their configuration with `provider_config(model)`, use the shared `text_model` body and transport, and expose the shared `ai::UploadDisclosure` (provider, policy link, model, payload bytes and a USD upper bound). No retry, fallback or provider switch happens inside an adapter, and nothing substitutes the local lexicon: a provider failure is a typed error.

The model returns, per cue, the positions it chose. `EmphasisInput::resolve` refuses a cue that was not sent, a duplicate cue, a position that is not a word of that cue, a duplicate position, or more than `max_per_cue` positions with `InvalidSegments`; nothing is clamped or dropped silently. Accepted positions come back ascending. A cue the model skipped has an empty or absent selection.

Model defaults, known rates, unavailable-model diagnostics and explicit unknown-price consent belong to the [provider contract](../contracts/README.md#remote-model-settings).
