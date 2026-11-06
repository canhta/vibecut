# Transcript clip suggestions

`clips::ClipInput` accepts stable transcript IDs, NFC text and nonoverlapping,
ordered source ranges (`ClipSegment::from(&captions::Segment)`), plus count,
inclusive min/max total duration in ticks, required positive `min_segment_duration`,
`max_segments` (JSON default four, 1..=32), maximum allowed summed source IoU, language and
optional keywords. Count is 1..=32; input is at most 10,000 segments / 4 MiB of
ID/text data. Empty/duplicate IDs, blank text, negative or overlapping ranges,
nonfinite overlap values and invalid bounds refuse before execution. Languages
are vi/en/zh/zh-Hans/zh-Hant for all three current adapters.

`ClipInput::windows()` plans overlapping windows capped at 128 segments and 24 KiB
of canonical JSON, at most 1,024 windows. Overlap covers the configured maximum
clip duration for contiguous spans. Assembly is local to each bounded window;
pieces separated by more than a window cannot be proposed together. This does not
guarantee exhaustive noncontiguous combinations across a long transcript. A transcript too dense to preserve all admissible boundary-spanning
ranges in those limits returns Payload; increase segmentation granularity or
reduce the requested duration explicitly. Plan first, show each window's exact
`consent_text`, provider policy link and estimate, and reserve an aggregate budget
in engine. Pass one `Request` per window in the same order to `suggest_clips`.
Every window's provider/language/settings/payload consent and per-invocation cost
is checked before any IO. Requests remain independent invocations, never implicit
retries or job-level budget approvals. Wire JSON is also capped at 64 KiB and by
the approved byte ceiling; shared transport caps responses and supplies timeouts.
At most 4,096 distinct ordered lists of endpoint IDs and roles are retained per
execution, counting both valid and invalid candidates. Repeated observations of a list are
reconciled before this bound, retaining the highest validated score (first on a tie).
A valid observation supersedes an out-of-window/invalid observation of the same
list; invalid metadata can never replace an accepted short. Each
valid list retains at most one representative duplicate diagnostic; repeated
invalid observations share one typed drop diagnostic. Thus retained state
holds at most 8,192 proposals including representative duplicates, regardless of
window overlap. The final global ranking and overlap selection use all distinct
valid candidates; no window is trimmed to the requested output count.

`OpenAiClipSuggester` uses the existing shared HTTPS transport, Responses endpoint,
strict structured output parser, user-selected `gpt-6.1-sol` or `gpt-6-luna`, and
`OPENAI_SETTINGS` (version two for the segment-list contract);
`AnthropicClipSuggester` reuses `anthropic::MessagesClient` and `message_text`,
`claude-sonnet-5-5`, and `ANTHROPIC_SETTINGS`. Both require the official provider
endpoint and exact versioned settings identity through the existing ProviderConfig.
Their operation-specific disclosure returns the provider name, policy link,
endpoint/model, exact payload and conservative USD invocation upper bound (input
wire byte ceiling plus framing allowance, 8,192 output tokens). Luna uses the
same conservative price ceiling as Sol; the disclosure and consent retain the
exact selected model. No SDK, runtime,
second provider configuration, generic retry or automatic fallback is introduced.

All adapters run through `ClipSuggester` and the same result validation. A
`ClipProposal` is `title`, `hook_text`, `reason`, ranking `score` and `segments` in
play order. Each `ClipPiece` contains exact `start_segment_id`, `end_segment_id`
and a closed `role` (`hook`, `context`, `payoff`). Reordering pieces is allowed;
each piece's endpoints must remain in source order. Only transcript IDs resolve
ranges; timestamp fields, unknown roles, old single-range fields, oversized IDs,
blank/oversized metadata and nonfinite/out-of-range scores refuse as Schema.
Semantic violations drop the individual proposal with a typed reason:
`UnknownId`, `ReversedRange`, `SegmentCount`, `HookOrder`, `SegmentOverlap`,
`SegmentDuration` or `Duration`. A hook is optional; when present exactly one must
be first. Source overlap within a short is refused even when pieces play apart.
Raw piece count must fit `max_segments`. Consecutive play pieces whose source
boundaries touch exactly merge, retaining the first role and the combined endpoint
IDs; the resolved piece carries typed `repair: AdjacentMerged`. The per-piece minimum applies after this canonical repair. Total duration is
the checked sum of canonical piece durations, excluding source gaps, and must fit
the inclusive min/max bounds.

`ClipSuggestion` carries the canonical proposal, `segments` resolved to integer
source `range`, `role` and zero-based `source_order` rank (still in play order),
`duration` (summed ticks), and `source: llm` or `source: heuristic`. There is no
single enclosing range. Downstream review/export must iterate this segment list;
source-order rank lets them show source and play order independently. Inner cuts
are the gaps between pieces; this contract contains no word-level cut list.
The serialized engine provider result wraps these suggestions and drops in its
existing provider-provenance envelope. CLI provider execution/rendering belongs
to its separate feature ticket; the CLI currently exposes the job stage only.

Scores sort descending with deterministic playback-coordinate ties. Summed
pairwise source intersections divided by summed source union define IoU; gaps
never contribute. Equal source coverage, or IoU >= 0.5 with normalized source-text
character-bigram Jaccard >= 0.8, deduplicates retaining the higher score regardless
of play order. Other candidates exceeding overlap tolerance drop with `Overlap`;
candidates beyond count drop with `CountLimit`. Fewer than count, including zero,
is valid. Scores are rankings, never probabilities; suggestions await user review.

`HeuristicClipSuggester::new()` is an explicitly selected, free local provider.
It ranks complete segment-edge spans using sentence-ending punctuation, pauses
of at least 0.5 seconds and user keyword density. It never splits timed segments
or fabricates word boundaries. It emits one `payoff` piece per suggestion, respecting both total and piece
minimums. It returns at most one best end per start before
shared validation/merging. On a remote failure the original typed error propagates;
`clip_recovery_choice(&error)` supplies a `RerunLocally` offer for presentation.
Only an affirmative user selection starts a fresh local execution. Cancellation
returns Cancelled and discards partial output, including cancellation after IO.

`cargo test -p ai --test it clips::` covers both wire formats, hostile/schema/ID/duration
inputs, exact consent and wire budgets, provider failures, cancellation, duplicate
and overlap selection, long-window coverage and multilingual heuristic execution.
The fixed synthetic EN/VI/ZH ranking fixture compares keyword ranking to a
sentence/pause-only baseline; the broader labelled pipeline harness is separate.
The ignored `live_clip_providers` test requires `VIBECUT_LIVE_AI=1` and
`OPENAI_API_KEY` and sends one bounded `gpt-6-luna` request. It uses synthetic
input by default; `VIBECUT_CLIP_SMOKE_INPUT` accepts a local `ClipInput` JSON file
from a consented real transcript, limited to one window. Optional
`VIBECUT_CLIP_SMOKE_OUTPUT_DIR` writes the validated result locally. Offline fixtures and
live integration prove contracts, not model accuracy or editorial quality.
