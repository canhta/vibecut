# Segmentation

Generated cues keep measured source word ranges in `Cue::word_ranges` when every contributing unit has native word or span timing. Estimated or proportional units leave the list empty, so callers can tell measured timing from a layout estimate.

`segment/units` derives indivisible speech units from words, spans or plain text. `segment/layout` fits them into the configured lines. `segment/assembly` turns them into positioned cues. `segment/validate` rejects transcripts outside the contract.

`segment_transcript` takes a transcript, one caller-resolved `LanguageProfile` and `SegmentRules`. It returns `Segmentation` with fresh `SegmentedCue` values and positioned warnings, and never mutates the input. `SegmentRules::from_profile` uses the profile's line length and CPS, two lines, 5/6..7 seconds, a two-frame gap at the supplied rational rate and a one-second pause threshold. `LineLimit` is the grapheme line width. There is no font dependency.

## Input contract

Segment IDs are unique and nonempty. Source segments are nonnegative, ordered and disjoint. Aggregate segment IDs and segment and word text cap at 16 MiB, and generated cue payloads plus accounted cue and warning overhead cap at 16 MiB. Word text must match the segment text apart from whitespace; missing or inconsistent word text falls back to the authoritative segment text. Invalid, out-of-order or out-of-segment word times are errors. Empty or whitespace segments yield no cues. Formatting whitespace collapses at line boundaries while visible text, punctuation and grapheme clusters are retained.

## Units and breaks

Complete timed words are indivisible and keep their source ranges. Adjacent timed words separated by a forbidden UAX #14 boundary form one unit from the first word start to the last word end, including silence between them, with timing `SourceWords`.

Untimed text uses UAX #14 opportunities with extended-grapheme safety. Chinese gets no inserted spaces and Latin runs stay intact. The same boundary policy applies before choosing timed-word or segment timing, including invalid spans.

Capacity and duration boundaries prefer sentence end, then clause punctuation, then pause, then a legal word boundary. Speaker changes and pauses also force a split at legal text boundaries, but never divide an indivisible unit, which keeps its first speaker. Indivisible words wider or longer than the limits stay intact and warn. Layout is greedy within each cue, without syntactic parsing.

## Timing kinds

Source word results use `SourceWords`. Complete source-span results use `SourceSpans` with the chosen layer's exact source endpoints. Missing timing or visible coverage in the selected span layer makes the result `Estimated` and leaves persisted source ranges absent. Both source kinds may borrow following silence under the rule below. These marks are generation metadata on `SegmentedCue`, not part of persisted `Cue` or SRT/VTT.

`segment_translation` takes one source cue or sentence group and translated `CueText`, and never accepts source word timestamps. Split times use cumulative grapheme weights (including input spaces) and integer floor offsets inside the input range, without cumulative rounding drift. Fragments are `TimingKind::Estimated`; an unsplit result uses `SourceSegment`. Split segment-only source text is also estimated, and mismatched or partially timed word lists always warn as estimated. Target fragments keep the edit flag and translation reference (default source ID) and contain target text only. IDs deterministically encode source identity, language and fragment ordinal.

## Conflicts

CPS, duration, gap and layout conflicts warn instead of truncating text or moving speech. A short timed-word cue may extend only into following silence before the next cue minus the requested gap. The last cue and segment or estimated timings never extend. Gaps that require cutting speech warn. Timing too short for a nonempty fragment returns a typed error. Consumers form a `CueTrack` from the generated cues and keep timing metadata separately.

## Source cut units

`source_speech_units(transcript, resolved_profile)` reuses subtitle validation, NFC correspondence, word and span grouping and profile punctuation to return indivisible `SourceSpeechUnit`s and sentence-end marks for engine boundary snapping. Fully measured, text-matching native groups keep their source ranges. Missing, incomplete or mismatched timing protects the entire source segment, and no proportional estimate is admitted as a cut boundary. Limits are 10,000 segments, 500,000 words or output units and the 16 MiB segmentation byte cap. The caller resolves the `LanguageProfile` once.
