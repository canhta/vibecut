# Clip suggestion workflow

`clip_suggest` is the composition used by the CLI and desktop: the shared `transcript::transcribe` ASR-to-cue stage, an explicit clip provider, ordered-segment ranking and boundary snapping. The caller owns the runner, cache, credentials, consent prompt, progress and cancellation token. Suggestion writes no file.

`choose_suggestion` accepts only the offline `suggestion_options` catalog rows: local `heuristic`, OpenAI `gpt-6-luna` or `gpt-6.1-sol`, or Anthropic `claude-sonnet-5-5`. There is no fallback list. Suggestion runs as a bounded `SuggestClips` job. Transient remote failures use the runner's attempt and backoff policy, while local and permanent failures are returned. Retries keep the approved scopes and per-invocation cost ceiling, and reported cost includes all attempts. Remote adapters receive only transcript segments (IDs, text, ranges and settings), never audio. Before the first upload, `UploadRequest` names the provider, endpoint, model, exact payload bytes, cost ceiling and the provider's data-retention and training policy link. Refusal stops before any adapter receives data. `DenyUploads` is the test default.

`ClipRequest` resolves the source language (or asks local whisper), runs ASR, builds the AI `ClipInput` from the cues and runs `ai::clips::suggest_clips` on the shared runner. Candidates then pass through `snap_clip_boundaries` with the source frame rate, caller tolerance and RMS, and duration and segment limits. A snap refusal drops that candidate with a typed `RejectedClip` reason instead of publishing an unsnapped range. AI drops keep their `ClipDropReason`. Candidate segments keep play order, roles, source-order ranks and signed edge deltas.

The outcome also keeps the source project, source cues, transcript, measured speech energy and resolved boundary settings, so desktop review can trim without re-running ASR or energy decoding.

Before language detection or ASR, the blocking composition reserves the
[transcript work/result ceilings](../../../engine/src/transcript/README.md) from the runner's
same aggregate budget. Work releases after transcription, while the result lease covers the
remaining composition; pressure returns a typed refusal before invoking the provider.

## Registry jobs

`ClipJobs` implements engine's `ClipWorkflow` for `clips.suggest` and `clips.export`; it holds the credentials a command may not carry and uses the lent access's providers, consent and cache. `clips.suggest` checks the provider choice and its credential at submission, so a refusal costs no ASR or upload, then runs language detection, ASR, windowing, per-window upload approval, ranking and snapping inside one `SuggestClips` job. A retry reuses the first attempt's approvals and scopes and the reported cost counts every attempt. The output is the engine `ClipsResult` (candidates with `pieces`, `segments`, `adjustments`, rejected and dropped reasons, and the source cues); the blocking `suggest` above stays for the desktop until it moves to the commands.

`clips.export` validates 1..=1000 distinct candidate indices, builds the shorts from the retained `ClipsResult` and the caption item's cues (translation is the caption commands' job, not this crate's) and calls `ClipBatch::prepare`, then submits one job per short with `submit_one`. `ClipBatch` is the one export path: it owns naming, caption mapping, portrait crops, publication and per-short cancellation (`job_cancel` per short); the former blocking `export`/`prepare_export` loop is removed. A failed admission cancels the shorts already admitted and returns its typed error; the caller sizes the runner queue for the whole selection. Published files are complete and existing files are preserved unless `overwrite`.
