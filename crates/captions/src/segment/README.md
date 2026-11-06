# Segmentation

`segment/units` derives indivisible speech units from words, spans or plain text. `segment/layout` fits speech units into the configured lines. `segment/assembly` turns speech units into positioned cues. `segment/validate` rejects transcripts outside the segmentation contract.

`segment_transcript` accepts a transcript, one caller-resolved `LanguageProfile`,
`SegmentRules` and `LayoutHooks`. It returns `Segmentation` containing fresh
`SegmentedCue` values and positioned warnings; it never mutates the input.
`SegmentRules::from_profile` uses profile line length/CPS, two lines, 5/6..7 seconds,
a two-frame gap at the supplied rational rate and a one-second pause threshold.
`LineLimit` selects grapheme or caller-measured pixel width; callbacks return typed
errors and measurements must be finite/nonnegative. There is no font dependency.

Complete timed words are indivisible and retain source ranges. Adjacent timed
words separated by a forbidden UAX #14 or dictionary boundary form one
indivisible unit with the first word start and last word end, including silence
between them; timing remains `SourceWords`. They must match
the segment text apart from whitespace. Missing/inconsistent word text falls back
to the authoritative segment text; invalid/out-of-order/out-of-segment word times
are errors. Segment IDs must be unique and nonempty; source segments must be
nonnegative, ordered and disjoint. Aggregate segment IDs and segment/word text are capped at 16 MiB.
Generated cue payloads and accounted cue/warning overhead also have a 16 MiB
budget, including repeated IDs/references. Empty/whitespace segments yield no cues. Formatting whitespace is collapsed at
line boundaries; visible text, punctuation and grapheme clusters are retained.

Untimed text uses Unicode UAX #14 opportunities, with extended-grapheme safety.
Chinese has no inserted spaces; Latin runs stay intact. An optional dictionary
callback returns sorted nonoverlapping NFC byte ranges, protecting word interiors
from line/cue breaks. Callers resolve a profile's optional segmenter name to this
callback. The same validated boundary policy applies before choosing timed-word
or segment timing, including callback errors and invalid spans. No dictionary is bundled. Capacity/duration boundaries prefer sentence
end, then clause punctuation, then pause, then a legal word boundary; speaker
changes and pauses also force a split at legal text boundaries. Text protection
takes precedence when a speaker change or pause falls inside an indivisible unit;
the unit retains its first speaker for grouping with neighbors. Indivisible words wider/longer than limits
remain intact and warn. Layout is greedy within each cue, without syntactic parsing.

`segment_translation` takes one source cue/sentence group and translated `CueText`;
it never accepts source word timestamps. Split times use cumulative grapheme
weights (including input spaces) and integer floor offsets in the input range,
without cumulative rounding drift. Fragments are marked `TimingKind::Estimated`;
an unsplit result uses `SourceSegment`. Split segment-only source text is also
estimated, and mismatched/partially timed word lists always warn as estimated.
Source word results use `SourceWords`; complete source-span results use
`SourceSpans`, retaining the chosen layer's exact source endpoints. Missing timing
or visible coverage in the selected span layer makes segmentation explicitly
estimated; the persisted source ranges stay absent. Both source timing kinds may
borrow following silence under the same rule. These marks are explicit generation
metadata on `SegmentedCue`, not additions to persisted `Cue` or SRT/VTT formats.
Target fragments retain the edit flag and translation reference (defaulting to
source ID) and contain target text only; source text remains on the input cue.
IDs deterministically encode source identity, language and fragment ordinal.

CPS, duration, gap and layout conflicts warn instead of truncating text or moving
speech. A short timed-word cue may extend only into following silence before the
next cue minus the requested gap. The last cue and segment/estimated timings are
never extended. Gaps that require cutting speech warn; they are not silently
forced. Timing too short to represent a nonempty fragment returns a typed error.
Consumers can form a `CueTrack` from the generated cues for interchange while
retaining the timing metadata separately for review.
## Segmentation performance

Observed on Apple M1 Pro, 16 GiB RAM, macOS 27.2, release profile: the deterministic
63,000-byte Vietnamese NFD corpus (41,000 normalized graphemes over 600 source
seconds) generated 500 cues with exact visible-text preservation and zero end-tick
drift in both segment-only and fully timed-word modes. Each mode ran 20 same-process
calls. Segment-only: first 9.992 ms, 19 warm calls 9.368–13.728 ms (median 9.534 ms).
Timed words: first 9.852 ms, 19 warm calls 9.674–11.484 ms (median 9.873 ms).
The combined test process peaked at 11,075,584 bytes RSS. Timed fixtures use
whitespace-delimited words with uniform source ranges; dictionary callbacks are
not benchmarked.
Run `cargo test -p captions --release --test it segmentation:: --no-run`, then invoke
the emitted executable with `--exact segmentation::segmentation_benchmark --ignored --nocapture`
under `/usr/bin/time -l` to measure without including compilation. No pixel metric
applies to this pure text/timing operation; other hardware and font callbacks are
unverified.
## Source cut units

`source_speech_units(transcript, resolved_profile, hooks)` reuses subtitle
validation, NFC correspondence, word/span grouping and profile punctuation to
return indivisible SourceSpeechUnits and sentence-end marks for engine boundary
snapping. Fully measured, text-matching native groups retain their source ranges;
missing/incomplete/mismatched timing protects the entire source segment. No
proportional estimated text timing is admitted as a cut boundary. Limits are
10,000 segments, 500,000 words/output units and the existing 16 MiB segmentation byte cap.
The caller resolves LanguageProfile once and can inject dictionary word hooks.
