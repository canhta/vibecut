# Transcript

Interface: `transcribe`, `translate_selected`, `translate_track`, `build_session`, `wait_for_job`, `Cancellation`, `Progress`, `translation`.

Acquires source-timed cues from the user's chosen providers, shared by subtitles and
the [shorts crate](../../../shorts/README.md). The public `transcript` interface shares source-timed acquisition, cue-session assembly,
translation and job waiting with downstream workflows. `Cancellation::checkpoint`
refuses the next stage; `media_flag` shares cancellation with media decoding.
Provider credentials expose only the selected OpenAI or Anthropic key lookup, returning
the existing typed missing-credential refusal before adapter construction.

The desktop also consumes `SubtitleTranslator` (owned by `transcript/`) without opening an ASR
adapter or model lease. `translate_selected` composes the same translation window,
request disclosure, `TranslationRun` attempt budget and `Retranslate` application
for an explicit `TranslationSelection` (cue IDs, target, glossary, force). The
CLI's whole-track translation delegates to it. Callers requiring atomic multi-window
application stage a snapshot Session and apply its final command against the live
Session's captured job target; a failed/cancelled staging run is never applied.

## Translation selection and application

`translation::window` selects cue IDs from one caption item, includes two source
context cues on each side, and skips target texts with edited=true unless force
is explicit. Unknown/duplicate IDs, no eligible source cues, missing source text
and mixed source languages are typed refusals. Selection and context are bound
in ai's canonical TranslationWindow; source timing remains untouched.

`TranslationRun::new` takes a TranslationTarget (Session and caption item), validates
that every selected ID/text still matches its source, and captures that revision
before retaining the selected provider, complete window, Scope and Consent. It preflights the per-attempt estimate against an explicit aggregate Money
budget for 1..=10 attempts. `submit(runner, session)` uses the existing shared job
runner and refuses a changed Session/revision before submission, including edits
during consent preparation. Each invocation atomically consumes
one operation-wide attempt, including schema and transport retries; retrying or
resubmitting the same run never resets that allowance. Every attempt revalidates
consent. The runner alone owns backoff and retry admission; Schema is retryable,
while refusal, auth/quota, payload/consent/cost and incomplete responses stop.
Schema repair retries the complete window; no partial output is applied or silently
switched to another provider. Four MiB work plus one MiB result is reserved from
the aggregate budget before execution. The content key includes the whole window,
source/target scripts and exact provider/model/settings identity.

Deserialize completed output bytes as ai::Translation and apply through
`JobOutput::apply(session, Command::Retranslate { item, target_language, texts,
translation_ref, force })`. The command's registered JSON ID is `retranslate`;
its required fields are those same typed fields, with force explicitly boolean.
Only returned/selected IDs participate, and unknown IDs or blank text refuse the
whole command. Existing edited targets are preserved unless force=true, including
when called directly through JSON. Source-language text cannot be replaced by a
translation. A successful command is one undo step with translation provenance;
locked items and stale Session outputs are refused through existing owners.
During retained trim gestures/groups/batches/merged edits, selected translations
update the authoritative cue track before remapping the visible range. Hidden
unselected content, source timings and all other language metadata survive inward
and outward reversal; protected no-ops preserve retention as well. Cancel/refusal
rolls these changes back with the candidate. Digit warnings travel in the AI result
for presentation; no provider edits Project.

Offline integration tests exercise actual OpenAI/fake-transport job submission,
schema repair on the selected provider, shared attempt limits, exact selection and
context, protected and force-retranslated targets, unselected cue preservation,
atomic hostile-ID refusal, undo and stale-result rejection. These establish the
execution contract, not human translation quality or actual cloud billing.
