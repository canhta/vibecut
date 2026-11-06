# Clip suggestion workflow

`clip_suggest` is the composition used by the CLI and desktop: the shared
`transcript::transcribe` ASR-to-cue stage, an explicit clip provider, ordered-segment
ranking and boundary snapping. The caller owns the runner, cache, credentials,
consent prompt, progress and cancellation token; the module writes no file during
suggestion.

`choose_suggestion` accepts only the offline `suggestion_options` catalog rows:
local `heuristic`, OpenAI `gpt-6-luna`/`gpt-6.1-sol`, or Anthropic
`claude-sonnet-5-5`. There is no fallback list. Suggestion runs as a bounded `SuggestClips` job;
transient remote failures use the runner’s existing attempt/backoff policy, while
local and permanent failures are returned. Retries retain the exact approved
scopes and their per-invocation cost ceiling; reported cost includes all attempts. Remote adapters receive only transcript
segments (IDs, text, ranges and settings), never audio. Before the first upload,
`UploadRequest` names the provider, endpoint, model, exact payload bytes, cost
ceiling and the provider's data-retention/training policy link; refusal stops
before any adapter receives data. `DenyUploads` is the test default.

`ClipRequest` resolves the source language (or asks local whisper), runs ASR,
builds the AI `ClipInput` from the resulting cues, and executes
`ai::clips::suggest_clips` on the shared runner. Candidates then pass through
`snap_clip_boundaries` with the source frame rate, caller tolerance/RMS and
duration/segment limits. A snap refusal drops that candidate with a typed
`RejectedClip` reason instead of publishing an unsnapped range; AI drops keep
their `ClipDropReason`. Candidate segments keep play order, roles, source-order
ranks and signed edge deltas.

`prepare_export` validates 1..=1000 distinct candidate indices, translates the
source cue track for each requested non-source language through the subtitle
translation path, and calls `ClipBatch::prepare`. `export` keeps up to the
runner's export concurrency in flight (capped by terminal-result retention),
refilling slots as shorts complete and pumping the runner when its queue is full.
A failed admission or job cancels and drains admitted siblings before returning
its typed error; already published
complete files remain. The CLI only selects candidates and prints paths.

`cargo test -p shorts --test it shorts::clip_suggest::` covers the ASR-to-snapped workflow,
source and translated caption preparation, portrait geometry, hostile/empty/
duplicate selections and the explicit provider catalog/credential refusal. Provider
attempt/consent tests remain in `../../../engine/tests/it/providers/selection.rs`; media encoding remains in
`tests/it/shorts/clip_export.rs`.

The clip suggestion outcome also retains its source project, source cues,
transcript, measured speech energy and resolved boundary settings for desktop
review. Consumers reuse these inputs for safe trimming without re-running ASR or
energy decoding; the suggestion and snapping algorithms are unchanged.
