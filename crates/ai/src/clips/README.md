# Transcript clip suggestions

`clips::ClipInput` takes stable transcript IDs, NFC text and ordered, nonoverlapping source ranges (`ClipSegment::from(&captions::Segment)`), plus count (1..=32), inclusive min/max total duration in ticks, positive `min_segment_duration`, `max_segments` (default four, 1..=32), maximum summed source IoU, language and optional keywords. Input is at most 10,000 segments / 4 MiB. Invalid input refuses before execution. Languages: vi/en/zh/zh-Hans/zh-Hant.

`ClipInput::windows()` plans overlapping windows of at most 128 segments and 24 KiB of canonical JSON, at most 1,024 windows. Overlap covers the maximum clip duration. Assembly is local to each window, so pieces separated by more than a window are never combined. A transcript too dense to keep all admissible boundary-spanning ranges within these limits returns Payload.

Plan first, show each window's exact `consent_text`, policy link and estimate, and reserve an aggregate budget in engine. Pass one `Request` per window, in order, to `suggest_clips`. Each window's consent and cost are checked before any IO. Requests are independent invocations, never implicit retries. Wire JSON is capped at 64 KiB and by the approved byte ceiling.

Per execution, at most 4,096 distinct ordered lists of endpoint IDs and roles are kept (valid and invalid), and at most 8,192 proposals including representative duplicates. Repeated observations of a list keep the highest validated score (first on a tie). A valid observation supersedes an invalid one of the same list. Each valid list keeps one duplicate diagnostic; invalid observations share one drop diagnostic. Final ranking uses all distinct valid candidates; windows are not trimmed to the requested count.

`OpenAiClipSuggester` uses the Responses endpoint, the strict structured parser, user-selected `gpt-6.1-sol` or `gpt-6-luna`, and `OPENAI_SETTINGS` (version two). `AnthropicClipSuggester` uses `anthropic::MessagesClient`, `claude-sonnet-5-5` and `ANTHROPIC_SETTINGS`. Both require the official endpoint and exact settings identity, built by `provider_config(model)`. Disclosure returns the shared `ai::UploadDisclosure` with provider, endpoint and policy links, model, payload bytes and a USD upper bound (input byte ceiling plus framing allowance, 8,192 output tokens). Luna uses Sol's price ceiling. No retry or fallback.

All adapters share one result validation. `ClipProposal` is `title`, `hook_text`, `reason`, ranking `score` and `segments` in play order. Each `ClipPiece` has `start_segment_id`, `end_segment_id` and a `role` (`hook`, `context`, `payoff`). Pieces may be reordered; each piece's endpoints stay in source order. Timestamp fields, unknown roles, oversized IDs, blank or oversized metadata and nonfinite or out-of-range scores refuse as Schema.

A violating proposal is dropped with a typed reason: `UnknownId`, `ReversedRange`, `SegmentCount`, `HookOrder`, `SegmentOverlap`, `SegmentDuration` or `Duration`.

- A hook is optional; if present, exactly one, first.
- Source overlap within a short is refused.
- Raw piece count fits `max_segments`.
- Consecutive play pieces whose source boundaries touch merge, keeping the first role; the piece carries `repair: AdjacentMerged`. The per-piece minimum applies after merging.
- Total duration is the checked sum of piece durations, excluding source gaps, within the min/max bounds.

`ClipSuggestion` carries the proposal with `segments` resolved to integer source `range`, `role` and zero-based `source_order`, summed `duration` in ticks, and `source: llm` or `heuristic`. There is no single enclosing range and no word-level cut list; inner cuts are the gaps between pieces. Downstream review/export iterates the segment list.

Scores sort descending with deterministic playback-coordinate ties. Equal source coverage, or IoU >= 0.5 with normalized source-text character-bigram Jaccard >= 0.8, deduplicates keeping the higher score. Other candidates over the overlap tolerance drop with `Overlap`; candidates beyond count drop with `CountLimit`. Fewer than count, including zero, is valid. Scores are rankings, not probabilities.

`HeuristicClipSuggester::new()` is a free local provider, selected explicitly. It ranks complete segment-edge spans by sentence-ending punctuation, pauses of at least 0.5 s and keyword density, never splits timed segments, and emits one `payoff` piece per suggestion. On a remote failure the typed error propagates and `clip_recovery_choice(&error)` offers `RerunLocally`; only an affirmative user choice starts a local run. Cancellation returns Cancelled and discards partial output.

Tests: `cargo test -p ai --test it clips::`. The ignored `live_clip_providers` test needs `VIBECUT_LIVE_AI=1` and `OPENAI_API_KEY` and sends one bounded `gpt-6-luna` request. `VIBECUT_CLIP_SMOKE_INPUT` takes a local single-window `ClipInput` JSON file; `VIBECUT_CLIP_SMOKE_OUTPUT_DIR` writes the result.
