# Transcript

Interface: `transcribe`, `translate_selected`, `translate_track`, `build_session`, `source_session`,
`wait_for_job`, `Cancellation`, `Error`, `Result`, `Progress`, `translation`.

Acquires source-timed cues from the user's chosen providers, shared by
[subtitles](../subtitles/README.md) and the [shorts crate](../../../shorts/README.md):
acquisition, cue-session assembly, translation and blocking job waiting.
`transcribe` is synchronous and takes execution hooks, progress, a source range and a
mutex-protected checkpoint cache (owned or borrowed). It never submits or pumps a runner.
Provider calls run outside the cache lock; the caller supplies the same cancellation flag
to media decoding and provider hooks. Registry jobs use their job token for both. `Cancellation::checkpoint`
refuses the next stage; `media_flag` shares cancellation with media decoding. Provider
credentials expose only the selected OpenAI or Anthropic key lookup, returning the typed
missing-credential refusal before adapter construction.

Synchronous speech/detection and recognition checkpoint operations return the Transcript
result, retaining Kernel Edit, Runtime, Providers, Media and AI causes. Cancellation checkpoints
return Runtime results directly. Only the existing acquisition execution bridges normalize AI
cancellation and the extraction cancellation sentinel to direct Runtime cancellation; a captured
concrete failure wins over that sentinel. General AI/Media conversions remain ordinary foreign
failures. Root Transcript composition delegates the actual owner's code/transience/job policy;
native Model/Transfer/Failed remain opaque. Cue-session construction and the translation
reference helper return Kernel Edit results. Translation selection, preparation and execution
return Transcript results, and TranslationRun submits that natural producer through the
generic runner. Session stale causes compose through Kernel Edit; translation AI failures,
including AI Cancelled, remain ordinary AI causes. Only a cancelled execution hook produces
direct Runtime cancellation. The blocking
`wait_for_job` returns Runtime results: failed jobs retain the original raw cause Arc in
`Failed`, direct cancellation stays direct, and missing snapshots or completed output retain
Kernel Session `StaleResult` through Runtime Edit composition.

A blocking composition root (clips, desktop Generate) reserves `WORK_BYTES` (64 MiB) and `RESULT_BYTES` (16 MiB)
from the runner's shared `ByteBudget` before extraction/detection or provider work. Work
leases cover recognition and alignment and release before subsequent jobs; result leases
cover the retained transcript/cues through the composition. A workflow already admitted by
its outer job declares its work and output ceiling there instead: registry generation uses
its bounded job result, and desktop Generate keeps its existing larger result reservation.
Neither the synchronous pipeline nor a cache mutex acquires a second workflow reservation.

`Providers::detect_language` detects an imported asset's spoken language through the local
whisper.cpp adapter; composition roots or the caption workflow job pass the detected tag explicitly to `transcribe`. A run with no speech returns `NoSpeech`. When every
recognized word carries confidence and their mean is below the fixed floor, `transcribe` returns
`LowConfidence` instead of inserting cues for a likely wrong language; providers without
confidence data never produce this refusal. Both are `transcript::Error`: the acquisition stage
constructs them, so they are not provider-selection failures. `AutoLanguageUnsupported` stays a
`providers::Error` composed through Transcript because it refuses a provider capability. Mixed
callers compose the acquisition result through `engine::control::Error::Transcript`.

The desktop consumes `SubtitleTranslator` without opening an ASR adapter or model lease.
`translate_selected` composes the translation window, request disclosure, `TranslationRun`
attempt budget and `Retranslate` application for an explicit `TranslationSelection` (cue IDs,
target, glossary, force). The CLI's whole-track translation delegates to it. Callers requiring
atomic multi-window application stage a snapshot Session and apply its final command against the
live Session's captured job target; a failed or cancelled staging run is never applied.

## Translation selection and application

`translation::window` selects cue IDs from one caption item, includes two source context cues on
each side, and skips target texts with `edited=true` unless force is explicit. Unknown or
duplicate IDs, no eligible source cues, missing source text and mixed source languages are typed
refusals. Selection and context are bound in ai's canonical `TranslationWindow`; source timing is
untouched.

`TranslationRun::new` takes a `TranslationTarget` (Session and caption item), validates that
every selected ID and text still matches its source, and captures that revision before
retaining the selected provider, complete window, `Scope` and `Consent`. It preflights the
per-attempt estimate against an explicit aggregate `Money` budget for 1..=10 attempts.
`submit(runner, session)` refuses a changed Session or revision before submission, including
edits during consent preparation. Each invocation atomically consumes one operation-wide
attempt, including schema and transport retries; resubmitting the same run never resets that
allowance, and every attempt revalidates consent. The runner alone owns backoff and retry
admission; `Schema` is retryable, while refusal, auth/quota, payload/consent/cost and incomplete
responses stop. Schema repair retries the complete window; no partial output is applied or
silently switched to another provider. Four MiB of work plus one MiB of result is reserved from
the aggregate budget before execution. The content key includes the whole window, source and
target scripts and exact provider/model/settings identity.

Deserialize completed output bytes as `ai::Translation` and apply through
`JobOutput::apply(session, Command::Retranslate { item, target_language, texts,
translation_ref, force })`. The registered JSON id is `retranslate`; its required fields are
those typed fields, with force explicitly boolean. A complete edit admits up to 10,000 returned texts independently of
the provider window bound, so a multi-window result applies as one undo step. Only returned and selected IDs participate;
unknown IDs or blank text refuse the whole command. Edited targets are preserved unless
force=true, including through JSON. Source-language text cannot be replaced by a translation. A
successful command is one undo step with translation provenance; locked items and stale Session
outputs are refused.

Both translation compositions generate a 76-byte opaque `translation:` reference from
the BLAKE3 of the canonical JSON tuple `("caption-translation-v1", ProviderConfig)`.
This binds complete provider/model/settings provenance within the existing 256-byte
application field, including maximum-length model IDs; no model or reference is truncated.

During retained trim gestures, groups, batches and merged edits, selected translations update
the authoritative cue track before remapping the visible range. Hidden unselected content,
source timings and all other language metadata survive inward and outward reversal; protected
no-ops preserve retention as well. Cancel or refusal rolls these changes back with the
candidate. Digit warnings travel in the AI result for presentation; no provider edits Project.
