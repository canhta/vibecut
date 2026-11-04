# captions

Headless source-timed transcript and multilingual cue storage, BCP 47 language
profiles, readable cue segmentation, pure source-to-output edit mapping and
SRT/WebVTT interchange. Excludes
translation providers and subtitle rendering.

## Entry points and persistence

`Transcript` owns stable `Segment` IDs, optional source `Word` ranges, speaker and
validated confidence, and provider/model/settings `Fingerprint`. `Cue` owns a
stable ID, validated `SourceRange`, source language, and language-keyed `CueText`
with independent edit flags and translation references. `set_text` protects edits
unless explicitly overridden. `CueTrack` groups cues; all models support serde.
`NfcText` normalizes every stored text on construction/deserialization; its Debug
is redacted. BCP 47 tags validate syntax and standardize casing without a fixed
language allowlist. No project store/schema is introduced here.

`LanguageRegistry::standard` provides editable profiles for vi, en, zh-Hans and
zh-Hant. Lookup tries progressively shorter tag prefixes then the generic profile.
Limits are grapheme counts/rates, not byte counts; policies and font preferences
are initial defaults, not measured reading-speed or font-coverage guarantees.

## Edit mapping

`EditMapping::new` validates an ordered list of `(SourceRange, output_start)`
pairs; each pair retains source duration at unit speed. Empty mappings, gaps,
repeated source spans, signed ticks and overlapping output spans are supported.
Output endpoint overflow returns a typed time error. Segment order identifies
occurrences and need not match temporal order; the mapping has no persisted schema.

`map_tick` returns every retained occurrence as a `MappedTick` (segment index and
output tick). `map_range` returns independent `MappedRange` intersections with both
source and output coordinates. All bounds are half-open. `inverse_tick` uses the
segment identity to recover the source exactly, even for overlapping output spans;
unknown occurrences and output ticks outside the selected segment have no inverse.

`map_cues` rebuilds a `CueTrack` from an original source track, drops fully cut
cues and clips partial intersections without merging gaps. It stable-sorts output
starts and preserves text, language, edit flags and translation references; it
never shifts overlaps or calls subtitle repair. Source IDs must be unique. Each
output cue receives a deterministic fragment ID encoding the mapping occurrence
and complete original ID. `CueFragmentId::from_id` recovers both; `to_id` encodes
an occurrence index followed by `/` and the unchanged source ID. This remains
unambiguous when the original ID contains slashes or looks like another fragment.

After changing cuts, build a new mapping and map the original source track again;
never map previously clipped output. Normalize stream timestamps to source-asset
time before mapping, applying the audio start offset once at that boundary. This
module neither knows nor reapplies stream offsets. Frame-snapped cuts use the
existing time crate conversions; retiming, media IO and project commands belong
to their respective consumers. Overlapping/negative output cues remain exact here;
interchange export separately applies its documented repair policy below.

## Segmentation

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
Source word results use `SourceWords`. These marks are explicit generation
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

`NormalizedText` owns reusable raw-to-NFC `TextSpan` correspondence at whole
extended-grapheme granularity. Spans are byte ranges, including many-to-one
combining sequences; `raw_range` expands partial normalized graphemes. Offsets
are not source/target word alignments. Script-aware timing grouping belongs to
its extension ticket.

Defaults use adult interlingual guidelines: [Vietnamese](https://partnerhelp.netflixstudios.com/hc/en-us/articles/220447048-Vietnamese-Timed-Text-Style-Guide)
42 graphemes/line and 17 CPS; [English](https://partnerhelp.netflixstudios.com/hc/en-us/articles/217350977-English-USA-Timed-Text-Style-Guide)
42/20; [Simplified Chinese](https://partnerhelp.netflixstudios.com/hc/en-us/articles/215986007-Chinese-Simplified-Timed-Text-Style-Guide)
and [Traditional Chinese](https://partnerhelp.netflixstudios.com/hc/en-us/articles/215994807-Chinese-Traditional-Timed-Text-Style-Guide)
16/9. [General requirements](https://partnerhelp.netflixstudios.com/hc/en-us/articles/215758617-Timed-Text-Style-Guide-General-Requirements)
supply two lines and 5/6..7 seconds; [timing guidelines](https://partnerhelp.netflixstudios.com/hc/en-us/articles/360051554394-Timed-Text-Style-Guide-Subtitle-Timing-Guidelines)
supply the two-frame gap. These are editable defaults, not a certification of
Netflix delivery compliance: our counts use graphemes, and no shot analysis,
linguistic reduction or children's/SDH preset is implied.


## Interchange and deterministic repairs

`parse(bytes, format, source_lang)` accepts up to 16 MiB: UTF-8, UTF-8 BOM,
UTF-16LE/BE with BOM, or Windows-1252 fallback. BOM, encoding fallback, malformed
encoding replacements and CRLF/CR conversion warn. UTF-16 without BOM is not
detected. Missing SRT counters, comma/dot fractions (1–9 digits), omitted hours,
and missing blank separators are accepted. Bad timings/stray lines warn and skip;
empty text remains in the model. VTT header absence warns. VTT identifiers are
replaced with deterministic ordinal IDs; settings, header metadata and
NOTE/STYLE/REGION blocks are discarded (with warnings for metadata). Inline VTT
markup remains literal model text; this is text interchange, not a style renderer.

Zero/backward durations become 1 ms at the same start with a warning. The track
stable-sorts by start (ties retain input order), warns on nonmonotonic input,
clamps negative starts to zero, and shifts overlaps after the previous cue while
preserving duration. `SourceRange` rejects zero/backward persisted ranges; callers
cannot bypass validation through serde. Arithmetic overflow returns a typed error.
Repair positions refer to sorted cue ordinals; import syntax positions are lines;
position zero means a document-wide warning. IDs and language edit flags survive
track repair. Repair can return an error after modifying earlier cues.

`export(track, format, languages)` selects one tag or two distinct tags in display
order; missing/empty language text warns and is omitted. Output uses UTF-8/LF;
blank payload lines are removed with a warning to prevent block splitting. SRT
renumbers cues; VTT escapes &, < and > and decodes those entities on import.
SRT rejects payload lines resembling timestamps to prevent cue injection.
Ticks round to nearest millisecond, ties away from zero (integer arithmetic).
Collapsed durations expand to 1 ms and push following starts as needed; overflow
errors. After canonical export, SRT import/export is byte-stable. Sub-millisecond
source precision and file formatting/metadata are not preserved.

## Verification

`cargo test -p captions` exercises model persistence/NFC/edit protection, language
fallback, bilingual ordering, decoding, timing repair and malformed/overflow
inputs. Mapping tests exercise cut boundaries, repeated occurrences, disjoint fragments,
rebuild from source, 29.97 frame cuts, one-time audio offset normalization, extreme
ticks, source identity/edit preservation and SRT accuracy within 1 ms. Property
tests verify exact inverses for each retained occurrence and no inverse for gaps.
Segmentation tables cover diacritics, mixed Han/Latin/emoji, long words,
long sentences, caller measurements/dictionary spans, warning conflicts and target
timing provenance. Property tests cover canonical interchange stability, hostile
bytes and segmentation text/timing preservation. The ignored
`segmentation_benchmark` test runs a deterministic Vietnamese corpus in release
mode and checks exact visible-text preservation and zero end-tick drift.

No media, network provider or renderer oracle is needed for this pure crate.

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
Run `cargo test -p captions --release --test segmentation --no-run`, then invoke
the emitted executable with `--exact segmentation_benchmark --ignored --nocapture`
under `/usr/bin/time -l` to measure without including compilation. No pixel metric
applies to this pure text/timing operation; other hardware and font callbacks are
unverified.
